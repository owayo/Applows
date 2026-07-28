//! sh バックエンド。Core IR を macOS `/bin/sh` (bash) + zsh 向けコードへ変換する。
//!
//! 方針:
//! - 副作用のある値 (run / ユーザ関数 / http_download) は一時変数 `__ap_tN` に正規化してから使う。
//!   これにより入れ子・条件内でもコマンドが 1 回だけ実行され、両バックエンドで挙動が揃う。
//! - 文字列補間は single quote リテラルと `"$var"` の連結で組み立てる。
//! - `arg()` / `args()` / `argc()` はトップレベル (= スクリプト引数) 前提 (sema が関数内使用を禁止)。

use crate::ast::CmpOp;
use crate::builtins::Builtin;
use crate::emit::common::{arith_op, literal_name, num_op};
use crate::emit::escape::sh_lit;
use crate::ir::{Cond, IrFunc, IrProgram, IrStmt, List, StrPart, Value};
use std::collections::HashSet;

pub fn emit_sh(program: &IrProgram) -> String {
    let mut e = Sh {
        out: String::new(),
        indent: 0,
        temp: 0,
    };
    for func in &program.funcs {
        e.emit_func(func);
    }
    e.emit_stmts(&program.body);
    e.out
}

struct Sh {
    out: String,
    indent: usize,
    temp: usize,
}

impl Sh {
    fn line(&mut self, s: &str) {
        for _ in 0..self.indent {
            self.out.push_str("  ");
        }
        self.out.push_str(s);
        self.out.push('\n');
    }

    fn fresh_temp(&mut self) -> String {
        let n = self.temp;
        self.temp += 1;
        format!("__ap_t{n}")
    }

    fn emit_func(&mut self, func: &IrFunc) {
        self.line(&format!("{}() {{", func.name));
        self.indent += 1;
        // パラメータを局所変数へ束縛
        for (i, p) in func.params.iter().enumerate() {
            self.line(&format!("local {}=\"${}\"", p, i + 1));
        }
        // 本体で代入される変数をすべて local 宣言 (外側との分離)
        let mut assigned = Vec::new();
        let mut assigned_seen = HashSet::new();
        collect_assigned(&func.body, &mut assigned, &mut assigned_seen);
        let params: HashSet<&str> = func.params.iter().map(String::as_str).collect();
        for v in assigned {
            if !params.contains(v) {
                self.line(&format!("local {v}"));
            }
        }
        self.emit_stmts(&func.body);
        if !matches!(func.body.last(), Some(IrStmt::Return { .. })) {
            self.line("return 0");
        }
        self.indent -= 1;
        self.line("}");
    }

    fn emit_stmts(&mut self, stmts: &[IrStmt]) {
        for s in stmts {
            self.emit_stmt(s);
        }
    }

    fn emit_stmt(&mut self, stmt: &IrStmt) {
        match stmt {
            IrStmt::Let { var, value } => {
                let mut pre = Vec::new();
                let word = self.materialize(value, &mut pre);
                self.emit_pre(&pre);
                self.line(&format!("{var}={word}"));
            }
            IrStmt::Print { value } => {
                let mut pre = Vec::new();
                let word = self.materialize(value, &mut pre);
                self.emit_pre(&pre);
                self.line(&format!("printf '%s\\n' {word}"));
            }
            IrStmt::Discard { call } => self.emit_discard(call),
            IrStmt::If {
                branches,
                otherwise,
            } => self.emit_if(branches, otherwise),
            IrStmt::While { cond, body } => {
                self.line("while :; do");
                self.indent += 1;
                let mut pre = Vec::new();
                let test = self.render_cond(cond, &mut pre);
                self.emit_pre(&pre);
                self.line(&format!("{test} || break"));
                self.emit_stmts(body);
                self.indent -= 1;
                self.line("done");
            }
            IrStmt::ForRange {
                var,
                start,
                end,
                body,
            } => {
                let mut pre = Vec::new();
                let s = self.materialize(start, &mut pre);
                let en = self.materialize(end, &mut pre);
                self.emit_pre(&pre);
                // 反復は隠しカウンタで制御し、ループ変数は毎周それから代入する。
                // これにより本体がループ変数を書き換えても反復は壊れない (Python/Rust と同じ意味論)。
                let counter = self.fresh_temp();
                let end_tmp = self.fresh_temp();
                self.line(&format!("{counter}={s}"));
                self.line(&format!("{end_tmp}={en}"));
                self.line(&format!("while [ \"${counter}\" -le \"${end_tmp}\" ]; do"));
                self.indent += 1;
                self.line(&format!("{var}=\"${counter}\""));
                self.emit_stmts(body);
                self.line(&format!("{counter}=$(({counter} + 1))"));
                self.indent -= 1;
                self.line("done");
            }
            IrStmt::ForEach { var, list, body } => {
                let items = self.render_list(list);
                self.line(&format!("for {var} in {items}; do"));
                self.indent += 1;
                self.emit_stmts(body);
                self.indent -= 1;
                self.line("done");
            }
            IrStmt::Return { status } => {
                let mut pre = Vec::new();
                let word = self.materialize(status, &mut pre);
                self.emit_pre(&pre);
                self.line(&format!("return {word}"));
            }
            IrStmt::Exit { code } => {
                let mut pre = Vec::new();
                let word = self.materialize(code, &mut pre);
                self.emit_pre(&pre);
                self.line(&format!("exit {word}"));
            }
        }
    }

    fn emit_pre(&mut self, pre: &[String]) {
        for line in pre {
            self.line(line);
        }
    }

    fn emit_if(&mut self, branches: &[(Cond, Vec<IrStmt>)], otherwise: &Option<Vec<IrStmt>>) {
        // if / else if / else を入れ子の if/else へ展開 (各条件の prelude を正しい位置に置くため)
        self.emit_if_chain(branches, otherwise, 0);
    }

    fn emit_if_chain(
        &mut self,
        branches: &[(Cond, Vec<IrStmt>)],
        otherwise: &Option<Vec<IrStmt>>,
        i: usize,
    ) {
        let (cond, body) = &branches[i];
        let mut pre = Vec::new();
        let test = self.render_cond(cond, &mut pre);
        self.emit_pre(&pre);
        self.line(&format!("if {test}; then"));
        self.indent += 1;
        self.emit_stmts(body);
        self.indent -= 1;
        if i + 1 < branches.len() {
            self.line("else");
            self.indent += 1;
            self.emit_if_chain(branches, otherwise, i + 1);
            self.indent -= 1;
            self.line("fi");
        } else if let Some(else_body) = otherwise {
            self.line("else");
            self.indent += 1;
            self.emit_stmts(else_body);
            self.indent -= 1;
            self.line("fi");
        } else {
            self.line("fi");
        }
    }

    fn emit_discard(&mut self, call: &Value) {
        let mut pre = Vec::new();
        match call {
            Value::Run { argv } => {
                let cmd = self.render_argv(argv, &mut pre);
                self.emit_pre(&pre);
                self.line(&cmd);
            }
            Value::Call { name, args } => {
                let words: Vec<String> =
                    args.iter().map(|a| self.materialize(a, &mut pre)).collect();
                self.emit_pre(&pre);
                self.line(format!("{} {}", name, words.join(" ")).trim_end());
            }
            Value::Builtin { builtin, args } => {
                self.emit_side_effect(*builtin, args, &mut pre);
            }
            other => {
                // 純粋値を捨てる文は sema が禁止済み。防御的に評価だけする。
                let mut pre2 = Vec::new();
                let word = self.materialize(other, &mut pre2);
                self.emit_pre(&pre2);
                self.line(&format!(": {word}"));
            }
        }
    }

    /// 副作用のある組み込みを文として発行する。
    fn emit_side_effect(&mut self, builtin: Builtin, args: &[Value], pre: &mut Vec<String>) {
        match builtin {
            Builtin::WriteText => {
                let path = self.materialize(&args[0], pre);
                let content = self.materialize(&args[1], pre);
                self.emit_pre(pre);
                pre.clear();
                let d = self.fresh_temp();
                self.line(&format!("{d}={path}"));
                self.line(&format!(
                    "printf '%s' {content} > \"${d}.tmp.$$\" && mv -f \"${d}.tmp.$$\" \"${d}\""
                ));
            }
            Builtin::AppendText => {
                let path = self.materialize(&args[0], pre);
                let content = self.materialize(&args[1], pre);
                self.emit_pre(pre);
                pre.clear();
                self.line(&format!("printf '%s' {content} >> {path}"));
            }
            Builtin::Copy => {
                let from = self.materialize(&args[0], pre);
                let to = self.materialize(&args[1], pre);
                self.emit_pre(pre);
                pre.clear();
                self.line(&format!("cp -f -- {from} {to}"));
            }
            Builtin::Remove => {
                let path = self.materialize(&args[0], pre);
                self.emit_pre(pre);
                pre.clear();
                self.line(&format!("rm -f -- {path}"));
            }
            Builtin::HttpDownload => {
                let mut p = Vec::new();
                let word = self.render_http_download(args, &mut p);
                self.emit_pre(&p);
                self.line(&format!(": {word}"));
            }
            _ => {}
        }
    }

    // ---- 値の具現化 (word 化。副作用値は一時変数へ) ----

    fn materialize(&mut self, value: &Value, pre: &mut Vec<String>) -> String {
        match value {
            Value::Int(n) => n.to_string(),
            Value::Str(parts) => render_str(parts),
            Value::Var(v) => format!("\"${{{v}}}\""),
            Value::Arith { op, left, right } => {
                let l = self.arith_operand(left, pre);
                let r = self.arith_operand(right, pre);
                format!("\"$(({l} {} {r}))\"", arith_op(*op))
            }
            Value::Run { argv } => {
                let cmd = self.render_argv(argv, pre);
                let t = self.fresh_temp();
                pre.push(cmd);
                pre.push(format!("{t}=$?"));
                format!("\"${t}\"")
            }
            Value::RunCapture { argv, default } => {
                let d = self.materialize(default, pre);
                let cmd = self.render_argv(argv, pre);
                let raw = self.fresh_temp();
                let t = self.fresh_temp();
                // `VAR="$(cmd)"` の終了ステータスはコマンド置換のものになるので、
                // そのまま if の条件に使える (非 0 終了・command not found は else へ)。
                // args() が空のときはコマンド未指定なので実行せず default にする
                // (PowerShell 側の `$__ap_args.Count -gt 0` ガードと挙動を揃える)。
                let guard = match argv {
                    List::Args => "[ \"$#\" -gt 0 ] && ".to_string(),
                    List::Literal(_) => String::new(),
                };
                pre.push(format!("if {guard}{raw}=\"$({cmd} 2>/dev/null)\"; then"));
                // 行区切りを LF に正規化する (PowerShell はネイティブ出力を行配列で受け取り
                // LF で結合するため、CR を残すと OS 差になる)。
                pre.push(format!(
                    "  {t}=\"$(printf '%s' \"${{{raw}}}\" | tr -d '\\r')\""
                ));
                pre.push("else".to_string());
                pre.push(format!("  {t}={d}"));
                pre.push("fi".to_string());
                format!("\"${t}\"")
            }
            Value::HttpPost { url, headers, body } => {
                self.render_http_post(url, headers, body, pre)
            }
            Value::Call { name, args } => {
                let words: Vec<String> = args.iter().map(|a| self.materialize(a, pre)).collect();
                let t = self.fresh_temp();
                pre.push(
                    format!("{} {}", name, words.join(" "))
                        .trim_end()
                        .to_string(),
                );
                pre.push(format!("{t}=$?"));
                format!("\"${t}\"")
            }
            Value::Builtin { builtin, args } => self.render_value_builtin(*builtin, args, pre),
        }
    }

    /// 算術文脈での被演算子 (bare な項)。複雑な項は一時変数へ落とす。
    fn arith_operand(&mut self, value: &Value, pre: &mut Vec<String>) -> String {
        match value {
            Value::Int(n) => n.to_string(),
            Value::Var(v) => v.clone(),
            Value::Arith { op, left, right } => {
                let l = self.arith_operand(left, pre);
                let r = self.arith_operand(right, pre);
                format!("({l} {} {r})", arith_op(*op))
            }
            other => {
                let word = self.materialize(other, pre);
                let t = self.fresh_temp();
                pre.push(format!("{t}={word}"));
                t
            }
        }
    }

    fn render_value_builtin(
        &mut self,
        builtin: Builtin,
        args: &[Value],
        pre: &mut Vec<String>,
    ) -> String {
        match builtin {
            Builtin::Env => {
                let name = literal_name(&args[0]);
                let default = self.materialize(&args[1], pre);
                let t = self.fresh_temp();
                pre.push(format!(
                    "if [ -n \"${{{name}+x}}\" ]; then {t}=\"${{{name}}}\"; else {t}={default}; fi"
                ));
                format!("\"${t}\"")
            }
            Builtin::Arg => {
                if let Value::Int(i) = &args[0] {
                    if *i >= 10 {
                        format!("\"${{{i}}}\"")
                    } else {
                        format!("\"${i}\"")
                    }
                } else {
                    "\"\"".to_string()
                }
            }
            Builtin::Argc => "\"$#\"".to_string(),
            Builtin::ReadText => {
                let path = self.materialize(&args[0], pre);
                format!("\"$(cat -- {path})\"")
            }
            Builtin::Upper => {
                let s = self.materialize(&args[0], pre);
                format!("\"$(printf '%s' {s} | tr '[:lower:]' '[:upper:]')\"")
            }
            Builtin::Lower => {
                let s = self.materialize(&args[0], pre);
                format!("\"$(printf '%s' {s} | tr '[:upper:]' '[:lower:]')\"")
            }
            Builtin::Trim => {
                let s = self.materialize(&args[0], pre);
                format!(
                    "\"$(printf '%s' {s} | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')\""
                )
            }
            Builtin::JsonEscape => {
                let s = self.materialize(&args[0], pre);
                json_escape_expr(&s)
            }
            Builtin::JsonAdd => {
                let json = self.materialize(&args[0], pre);
                let key = self.materialize(&args[1], pre);
                let value = self.materialize(&args[2], pre);
                let ek = self.fresh_temp();
                let ev = self.fresh_temp();
                let base = self.fresh_temp();
                let cut = self.fresh_temp();
                let inner = self.fresh_temp();
                let out = self.fresh_temp();

                pre.push(format!("{ek}={}", json_escape_expr(&key)));
                pre.push(format!("{ev}={}", json_escape_expr(&value)));
                // JSON 本体は解析しない。制御文字の除去と末尾空白の削除だけして、
                // 閉じ波括弧の直前へフィールドを差し込む (中身は一切変形しない)。
                pre.push(format!(
                    r#"{base}="$(printf '%s' {json} | tr -d '\000-\037' | sed -e 's/[[:space:]]*$//')""#
                ));
                pre.push(format!(r#"{cut}="${{{base}%\}}}}""#));
                pre.push(format!(r#"if [ "${{{cut}}}" = "${{{base}}}" ]; then"#));
                // 末尾が } でない = top-level オブジェクトではないので手を付けない
                pre.push(format!(r#"  {out}="${{{base}}}""#));
                pre.push("else".to_string());
                pre.push(format!(
                    r#"  {inner}="$(printf '%s' "${{{cut}}}" | sed -e 's/[[:space:]]*$//')""#
                ));
                pre.push(format!(r#"  if [ "${{{inner}}}" = '{{' ]; then"#));
                pre.push(format!(r#"    {out}="{{\"${{{ek}}}\":\"${{{ev}}}\"}}""#));
                pre.push("  else".to_string());
                pre.push(format!(
                    r#"    {out}="${{{inner}}},\"${{{ek}}}\":\"${{{ev}}}\"}}""#
                ));
                pre.push("  fi".to_string());
                pre.push("fi".to_string());
                format!("\"${{{out}}}\"")
            }
            Builtin::HttpDownload => self.render_http_download(args, pre),
            Builtin::ReadStdin => {
                let t = self.fresh_temp();
                // tty から起動された場合に入力待ちで固まらないよう、リダイレクトされている
                // ときだけ読む (PowerShell 側の [Console]::IsInputRedirected と同じ意味)。
                pre.push(format!("if [ -t 0 ]; then {t}=''; else {t}=\"$(cat)\"; fi"));
                format!("\"${t}\"")
            }
            Builtin::Hostname => {
                let t = self.fresh_temp();
                pre.push(format!("{t}=\"$(uname -n 2>/dev/null)\""));
                format!("\"${t}\"")
            }
            Builtin::ScriptPath => "\"$0\"".to_string(),
            Builtin::ScriptDir => {
                "\"$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd)\"".to_string()
            }
            Builtin::Cwd => "\"$PWD\"".to_string(),
            _ => "\"\"".to_string(),
        }
    }

    fn render_http_download(&mut self, args: &[Value], pre: &mut Vec<String>) -> String {
        let url = self.materialize(&args[0], pre);
        let dest = self.materialize(&args[1], pre);
        let d = self.fresh_temp();
        let t = self.fresh_temp();
        pre.push(format!("{d}={dest}"));
        pre.push(format!(
            "if curl -fsSL {url} -o \"${d}.part.$$\"; then mv -f \"${d}.part.$$\" \"${d}\"; {t}=0; else rm -f \"${d}.part.$$\"; {t}=1; fi"
        ));
        format!("\"${t}\"")
    }

    /// HTTP POST。body とヘッダは 0600 の一時ファイル経由で curl へ渡す。
    ///
    /// - 秘密情報 (Authorization 等) をコマンドライン引数に載せない (`ps` から読めてしまうため)
    ///   ので、ヘッダは `--header @file` で渡す。
    /// - CR/LF を含むヘッダはヘッダインジェクションになるため送信せず `2` を返す。
    /// - `--location` は付けない (リダイレクト先へ Authorization を漏らさない)。
    fn render_http_post(
        &mut self,
        url: &Value,
        headers: &List,
        body: &Value,
        pre: &mut Vec<String>,
    ) -> String {
        let url = self.materialize(url, pre);
        let body = self.materialize(body, pre);
        let t = self.fresh_temp();
        let b = self.fresh_temp();
        let h = self.fresh_temp();
        let ok = self.fresh_temp();
        let hv = self.fresh_temp();

        // 空のリテラルリストは `for x in ; do` になり構文エラーなので、ループごと省く。
        let header_words = match headers {
            List::Literal(items) if items.is_empty() => None,
            other => Some(self.render_list(other)),
        };

        pre.push(format!("{t}=2"));
        pre.push(format!(
            "{b}=\"$(mktemp \"${{TMPDIR:-/tmp}}/applows.XXXXXX\" 2>/dev/null)\" || {b}=''"
        ));
        pre.push(format!("if [ -n \"${b}\" ]; then"));
        pre.push(format!("  {h}=\"${b}.h\""));
        pre.push(format!("  : > \"${h}\""));
        pre.push(format!("  chmod 600 \"${b}\" \"${h}\" 2>/dev/null || :"));
        pre.push(format!("  printf '%s' {body} > \"${b}\""));
        pre.push(format!("  {ok}=1"));
        if let Some(words) = header_words {
            pre.push(format!("  for {hv} in {words}; do"));
            pre.push(format!(
                "    if [ \"$(printf '%s' \"${hv}\" | tr -d '\\r\\n')\" != \"${hv}\" ]; then {ok}=0; else printf '%s\\n' \"${hv}\" >> \"${h}\"; fi"
            ));
            pre.push("  done".to_string());
        }
        pre.push(format!("  if [ \"${ok}\" = 1 ]; then"));
        pre.push(format!(
            "    if curl --fail --silent --show-error --max-time 30 --request POST --header @\"${h}\" --data-binary @\"${b}\" --output /dev/null --url {url}; then {t}=0; else {t}=1; fi"
        ));
        pre.push("  fi".to_string());
        pre.push(format!("  rm -f \"${b}\" \"${h}\""));
        pre.push("fi".to_string());
        format!("\"${t}\"")
    }

    /// argv (List) をコマンド文字列へ。
    fn render_argv(&mut self, list: &List, pre: &mut Vec<String>) -> String {
        match list {
            List::Literal(items) => {
                let words: Vec<String> = items.iter().map(|v| self.materialize(v, pre)).collect();
                words.join(" ")
            }
            List::Args => "\"$@\"".to_string(),
        }
    }

    /// for-each の反復子。
    fn render_list(&mut self, list: &List) -> String {
        match list {
            List::Literal(items) => {
                let mut pre = Vec::new();
                let words: Vec<String> = items
                    .iter()
                    .map(|v| self.materialize(v, &mut pre))
                    .collect();
                // for-each のリテラルは副作用を含まない前提 (sema が Text/Int に限定)
                words.join(" ")
            }
            List::Args => "\"$@\"".to_string(),
        }
    }

    // ---- 条件 ----

    fn render_cond(&mut self, cond: &Cond, pre: &mut Vec<String>) -> String {
        match cond {
            Cond::Cmp {
                op,
                numeric,
                left,
                right,
            } => {
                let l = self.materialize(left, pre);
                let r = self.materialize(right, pre);
                if *numeric {
                    format!("[ {l} {} {r} ]", num_op(*op))
                } else {
                    let o = if matches!(op, CmpOp::Eq) { "=" } else { "!=" };
                    format!("[ {l} {o} {r} ]")
                }
            }
            Cond::And(a, b) => {
                let a = self.render_cond(a, pre);
                let b = self.render_cond(b, pre);
                format!("{{ {a} && {b}; }}")
            }
            Cond::Or(a, b) => {
                let a = self.render_cond(a, pre);
                let b = self.render_cond(b, pre);
                format!("{{ {a} || {b}; }}")
            }
            Cond::Not(a) => {
                let a = self.render_cond(a, pre);
                format!("! {a}")
            }
            Cond::Test { builtin, args } => {
                let flag = match builtin {
                    Builtin::Exists => "-e",
                    Builtin::IsFile => "-f",
                    Builtin::IsDir => "-d",
                    _ => "-e",
                };
                let path = self.materialize(&args[0], pre);
                format!("[ {flag} {path} ]")
            }
        }
    }
}

/// 文字列補間を「single quote リテラル + "$var"」の連結へ。
fn render_str(parts: &[StrPart]) -> String {
    if parts.is_empty() {
        return "''".to_string();
    }
    let mut out = String::new();
    for part in parts {
        match part {
            StrPart::Lit(s) => out.push_str(&sh_lit(s)),
            StrPart::Var(v) => out.push_str(&format!("\"${{{v}}}\"")),
        }
    }
    out
}

/// 文列で代入されるすべての変数スロットを収集する (関数内 local 宣言用)。
fn collect_assigned<'a>(stmts: &'a [IrStmt], out: &mut Vec<&'a str>, seen: &mut HashSet<&'a str>) {
    for s in stmts {
        match s {
            IrStmt::Let { var, .. } => push_unique(out, seen, var),
            IrStmt::ForRange { var, body, .. } | IrStmt::ForEach { var, body, .. } => {
                push_unique(out, seen, var);
                collect_assigned(body, out, seen);
            }
            IrStmt::If {
                branches,
                otherwise,
            } => {
                for (_, body) in branches {
                    collect_assigned(body, out, seen);
                }
                if let Some(b) = otherwise {
                    collect_assigned(b, out, seen);
                }
            }
            IrStmt::While { body, .. } => collect_assigned(body, out, seen),
            _ => {}
        }
    }
}

fn push_unique<'a>(out: &mut Vec<&'a str>, seen: &mut HashSet<&'a str>, v: &'a str) {
    if seen.insert(v) {
        out.push(v);
    }
}

/// JSON 文字列リテラルの中身として安全な形へ変換する式を返す。
/// 制御文字 (U+0000〜U+001F) を除去し、`\` と `"` をエスケープする。
/// PowerShell 側の実装と結果を一致させること。
fn json_escape_expr(src: &str) -> String {
    format!(r#""$(printf '%s' {src} | tr -d '\000-\037' | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g')""#)
}

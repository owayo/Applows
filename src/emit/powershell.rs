//! PowerShell バックエンド。Core IR を Windows PowerShell 5.1 互換コードへ変換する。
//!
//! sh バックエンドと同じ正規化方針 (副作用値を一時変数へ) を採り、両者の挙動を揃える。
//!
//! PowerShell 5.1 固有の対策 (Codex レビュー):
//! - 出力は `[Console]::Out.WriteLine` + `OutputEncoding=UTF8(BOM無)` で UTF-8 に固定。
//! - ファイル I/O は `[System.IO.File]` を UTF-8(BOM 無) で使い、`Out-File` の UTF-16 化を避ける。
//! - コマンド/関数の出力ストリームで戻り値を汚さないため、run は `$LASTEXITCODE`、
//!   ユーザ関数は `$global:__ap_ret` 経由でステータスを受け渡す (bare 実行で stdio を継承)。
//! - 自己パスは `$env:APPLOWS_SELF` (Batch が束縛) を使い `$PSScriptRoot` に依存しない。

use crate::ast::{ArithOp, CmpOp};
use crate::builtins::Builtin;
use crate::emit::escape::ps_lit;
use crate::ir::{Cond, IrFunc, IrProgram, IrStmt, List, StrPart, Value};

pub fn emit_powershell(program: &IrProgram) -> String {
    let mut e = Ps {
        out: String::new(),
        indent: 0,
        temp: 0,
    };
    // 実行時セットアップ
    e.line("$__ap_args = $args");
    e.line("$global:__ap_ret = 0");
    e.line(
        "try { [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false } catch {}",
    );
    e.line("$ErrorActionPreference = 'Stop'");
    // 関数定義
    for func in &program.funcs {
        e.emit_func(func);
    }
    // main 本体 (try/catch で未捕捉例外を status 1 に)
    e.line("try {");
    e.indent += 1;
    e.emit_stmts(&program.body);
    e.line("exit 0");
    e.indent -= 1;
    e.line("} catch {");
    e.indent += 1;
    e.line("[Console]::Error.WriteLine($_)");
    e.line("exit 1");
    e.indent -= 1;
    e.line("}");
    e.out
}

struct Ps {
    out: String,
    indent: usize,
    temp: usize,
}

impl Ps {
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
        format!("$__ap_t{n}")
    }

    fn emit_func(&mut self, func: &IrFunc) {
        self.line(&format!("function {} {{", func.name));
        self.indent += 1;
        if !func.params.is_empty() {
            let params: Vec<String> = func.params.iter().map(|p| format!("${p}")).collect();
            self.line(&format!("param({})", params.join(", ")));
        }
        self.emit_stmts(&func.body);
        if !matches!(func.body.last(), Some(IrStmt::Return { .. })) {
            self.line("$global:__ap_ret = 0");
            self.line("return");
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
                let expr = self.materialize(value, &mut pre);
                self.emit_pre(&pre);
                self.line(&format!("${var} = {expr}"));
            }
            IrStmt::Print { value } => {
                let mut pre = Vec::new();
                let expr = self.materialize(value, &mut pre);
                self.emit_pre(&pre);
                self.line(&format!("[Console]::Out.WriteLine({expr})"));
            }
            IrStmt::Discard { call } => self.emit_discard(call),
            IrStmt::If {
                branches,
                otherwise,
            } => self.emit_if_chain(branches, otherwise, 0),
            IrStmt::While { cond, body } => {
                self.line("while ($true) {");
                self.indent += 1;
                let mut pre = Vec::new();
                let test = self.render_cond(cond, &mut pre);
                self.emit_pre(&pre);
                self.line(&format!("if (-not ({test})) {{ break }}"));
                self.emit_stmts(body);
                self.indent -= 1;
                self.line("}");
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
                let end_tmp = self.fresh_temp();
                self.line(&format!("${var} = {s}"));
                self.line(&format!("{end_tmp} = {en}"));
                self.line(&format!("while (${var} -le {end_tmp}) {{"));
                self.indent += 1;
                self.emit_stmts(body);
                self.line(&format!("${var} = ${var} + 1"));
                self.indent -= 1;
                self.line("}");
            }
            IrStmt::ForEach { var, list, body } => {
                let items = self.render_list(list);
                self.line(&format!("foreach (${var} in {items}) {{"));
                self.indent += 1;
                self.emit_stmts(body);
                self.indent -= 1;
                self.line("}");
            }
            IrStmt::Return { status } => {
                let mut pre = Vec::new();
                let expr = self.materialize(status, &mut pre);
                self.emit_pre(&pre);
                self.line(&format!("$global:__ap_ret = {expr}"));
                self.line("return");
            }
            IrStmt::Exit { code } => {
                let mut pre = Vec::new();
                let expr = self.materialize(code, &mut pre);
                self.emit_pre(&pre);
                self.line(&format!("exit [int]({expr})"));
            }
        }
    }

    fn emit_pre(&mut self, pre: &[String]) {
        for line in pre {
            self.line(line);
        }
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
        self.line(&format!("if ({test}) {{"));
        self.indent += 1;
        self.emit_stmts(body);
        self.indent -= 1;
        if i + 1 < branches.len() {
            self.line("} else {");
            self.indent += 1;
            self.emit_if_chain(branches, otherwise, i + 1);
            self.indent -= 1;
            self.line("}");
        } else if let Some(else_body) = otherwise {
            self.line("} else {");
            self.indent += 1;
            self.emit_stmts(else_body);
            self.indent -= 1;
            self.line("}");
        } else {
            self.line("}");
        }
    }

    fn emit_discard(&mut self, call: &Value) {
        let mut pre = Vec::new();
        match call {
            Value::Run { argv } => {
                let cmd = self.render_argv(argv, &mut pre);
                self.emit_pre(&pre);
                // 起動失敗 (command not found 等) で全体終了しないよう局所 catch で握りつぶす
                // (sh の 127 継続に相当。戻り値は捨てる文なので値は保持しない)。
                self.line(&format!("try {{ {cmd} }} catch {{ }}"));
            }
            Value::Call { name, args } => {
                let words: Vec<String> =
                    args.iter().map(|a| self.materialize(a, &mut pre)).collect();
                self.emit_pre(&pre);
                self.line(format!("{} {}", name, words.join(" ")).trim_end());
            }
            Value::Builtin { builtin, args } => self.emit_side_effect(*builtin, args),
            other => {
                let word = self.materialize(other, &mut pre);
                self.emit_pre(&pre);
                self.line(&format!("$null = {word}"));
            }
        }
    }

    fn emit_side_effect(&mut self, builtin: Builtin, args: &[Value]) {
        let mut pre = Vec::new();
        match builtin {
            Builtin::WriteText => {
                let path = self.materialize(&args[0], &mut pre);
                let content = self.materialize(&args[1], &mut pre);
                self.emit_pre(&pre);
                let d = self.fresh_temp();
                self.line(&format!("{d} = {path}"));
                self.line(&format!(
                    "[System.IO.File]::WriteAllText(\"$({d}).tmp\", {content}, (New-Object System.Text.UTF8Encoding $false))"
                ));
                self.line(&format!(
                    "Move-Item -Force -LiteralPath \"$({d}).tmp\" -Destination {d}"
                ));
            }
            Builtin::AppendText => {
                let path = self.materialize(&args[0], &mut pre);
                let content = self.materialize(&args[1], &mut pre);
                self.emit_pre(&pre);
                self.line(&format!(
                    "[System.IO.File]::AppendAllText({path}, {content}, (New-Object System.Text.UTF8Encoding $false))"
                ));
            }
            Builtin::Copy => {
                let from = self.materialize(&args[0], &mut pre);
                let to = self.materialize(&args[1], &mut pre);
                self.emit_pre(&pre);
                self.line(&format!(
                    "Copy-Item -Force -LiteralPath {from} -Destination {to}"
                ));
            }
            Builtin::Remove => {
                let path = self.materialize(&args[0], &mut pre);
                self.emit_pre(&pre);
                // sh の `rm -f` に合わせ、存在しないファイルでもエラーにしない
                self.line(&format!(
                    "Remove-Item -Force -ErrorAction SilentlyContinue -LiteralPath {path}"
                ));
            }
            Builtin::HttpDownload => {
                let word = self.render_http_download(args, &mut pre);
                self.emit_pre(&pre);
                self.line(&format!("$null = {word}"));
            }
            _ => {}
        }
    }

    // ---- 値の具現化 ----

    fn materialize(&mut self, value: &Value, pre: &mut Vec<String>) -> String {
        match value {
            Value::Int(n) => n.to_string(),
            Value::Str(parts) => render_str(parts),
            Value::Var(v) => format!("${v}"),
            Value::Arith { op, left, right } => {
                let l = self.arith_operand(left, pre);
                let r = self.arith_operand(right, pre);
                ps_arith(*op, &l, &r)
            }
            Value::Run { argv } => {
                let cmd = self.render_argv(argv, pre);
                let t = self.fresh_temp();
                // コマンドが見つからない等の起動失敗は sh では終了コード 127 で継続する。
                // PS は $ErrorActionPreference='Stop' で例外→全体終了になるため局所 catch で 127 に揃える。
                pre.push(format!(
                    "try {{ {cmd}; {t} = $LASTEXITCODE }} catch {{ {t} = 127 }}"
                ));
                t
            }
            Value::Call { name, args } => {
                let words: Vec<String> = args.iter().map(|a| self.materialize(a, pre)).collect();
                let t = self.fresh_temp();
                pre.push(
                    format!("{} {}", name, words.join(" "))
                        .trim_end()
                        .to_string(),
                );
                pre.push(format!("{t} = $global:__ap_ret"));
                t
            }
            Value::Builtin { builtin, args } => self.render_value_builtin(*builtin, args, pre),
        }
    }

    fn arith_operand(&mut self, value: &Value, pre: &mut Vec<String>) -> String {
        match value {
            Value::Int(n) => n.to_string(),
            Value::Var(v) => format!("[long]${v}"),
            Value::Arith { op, left, right } => {
                let l = self.arith_operand(left, pre);
                let r = self.arith_operand(right, pre);
                ps_arith(*op, &l, &r)
            }
            other => {
                let word = self.materialize(other, pre);
                let t = self.fresh_temp();
                pre.push(format!("{t} = {word}"));
                format!("[long]{t}")
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
                format!("$(if ($null -ne $env:{name}) {{ $env:{name} }} else {{ {default} }})")
            }
            Builtin::Arg => {
                if let Value::Int(i) = &args[0] {
                    format!("[string]($__ap_args[{}])", i - 1)
                } else {
                    "''".to_string()
                }
            }
            Builtin::Argc => "$__ap_args.Count".to_string(),
            Builtin::ReadText => {
                let path = self.materialize(&args[0], pre);
                format!(
                    "[System.IO.File]::ReadAllText({path}, (New-Object System.Text.UTF8Encoding $false))"
                )
            }
            Builtin::Upper => {
                let s = self.materialize(&args[0], pre);
                format!("([string]{s}).ToUpper()")
            }
            Builtin::Lower => {
                let s = self.materialize(&args[0], pre);
                format!("([string]{s}).ToLower()")
            }
            Builtin::Trim => {
                let s = self.materialize(&args[0], pre);
                format!("([string]{s}).Trim()")
            }
            Builtin::HttpDownload => self.render_http_download(args, pre),
            Builtin::ScriptPath => "$env:APPLOWS_SELF".to_string(),
            Builtin::ScriptDir => {
                "[System.IO.Path]::GetDirectoryName($env:APPLOWS_SELF)".to_string()
            }
            Builtin::Cwd => "(Get-Location).Path".to_string(),
            _ => "''".to_string(),
        }
    }

    fn render_http_download(&mut self, args: &[Value], pre: &mut Vec<String>) -> String {
        let url = self.materialize(&args[0], pre);
        let dest = self.materialize(&args[1], pre);
        let d = self.fresh_temp();
        let t = self.fresh_temp();
        pre.push(format!("{d} = {dest}"));
        pre.push(format!(
            "try {{ Invoke-WebRequest -UseBasicParsing -Uri {url} -OutFile \"$({d}).part\"; Move-Item -Force -LiteralPath \"$({d}).part\" -Destination {d}; {t} = 0 }} catch {{ Remove-Item -Force -ErrorAction SilentlyContinue -LiteralPath \"$({d}).part\"; {t} = 1 }}"
        ));
        t
    }

    fn render_argv(&mut self, list: &List, pre: &mut Vec<String>) -> String {
        match list {
            List::Literal(items) => {
                let words: Vec<String> = items.iter().map(|v| self.materialize(v, pre)).collect();
                format!("& {}", words.join(" "))
            }
            // 引数 0 個のとき $__ap_args[0] が $null で `& $null` 例外になるのを防ぐ
            // (sh の "$@" が空で無害なのと挙動を揃える)
            List::Args => "if ($__ap_args.Count -gt 0) { & $__ap_args[0] @($__ap_args | Select-Object -Skip 1) }".to_string(),
        }
    }

    fn render_list(&mut self, list: &List) -> String {
        match list {
            List::Literal(items) => {
                let mut pre = Vec::new();
                let words: Vec<String> = items
                    .iter()
                    .map(|v| self.materialize(v, &mut pre))
                    .collect();
                format!("@({})", words.join(", "))
            }
            List::Args => "$__ap_args".to_string(),
        }
    }

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
                    format!("([long]{l} {} [long]{r})", num_op(*op))
                } else {
                    let o = if matches!(op, CmpOp::Eq) {
                        "-ceq"
                    } else {
                        "-cne"
                    };
                    format!("([string]{l} {o} [string]{r})")
                }
            }
            Cond::And(a, b) => {
                let a = self.render_cond(a, pre);
                let b = self.render_cond(b, pre);
                format!("({a} -and {b})")
            }
            Cond::Or(a, b) => {
                let a = self.render_cond(a, pre);
                let b = self.render_cond(b, pre);
                format!("({a} -or {b})")
            }
            Cond::Not(a) => {
                let a = self.render_cond(a, pre);
                format!("(-not ({a}))")
            }
            Cond::Test { builtin, args } => {
                let path = self.materialize(&args[0], pre);
                match builtin {
                    Builtin::Exists => format!("(Test-Path -LiteralPath {path})"),
                    Builtin::IsFile => format!("(Test-Path -LiteralPath {path} -PathType Leaf)"),
                    Builtin::IsDir => {
                        format!("(Test-Path -LiteralPath {path} -PathType Container)")
                    }
                    _ => format!("(Test-Path -LiteralPath {path})"),
                }
            }
        }
    }
}

/// 文字列補間を「single quote リテラル + [string]$var」の連結へ。
fn render_str(parts: &[StrPart]) -> String {
    if parts.is_empty() {
        return "''".to_string();
    }
    let terms: Vec<String> = parts
        .iter()
        .map(|p| match p {
            StrPart::Lit(s) => ps_lit(s),
            StrPart::Var(v) => format!("[string]${v}"),
        })
        .collect();
    // 単一かつ内部に連結 (` + `) を含まない項はそのまま返す。それ以外は必ず括弧で
    // 包む (argv `& cmd (...)` やメソッド引数で `+` が別トークンに割れるのを防ぐ)。
    if terms.len() == 1 && !terms[0].contains(" + ") {
        return terms.into_iter().next().unwrap();
    }
    format!("({})", terms.join(" + "))
}

fn literal_name(value: &Value) -> String {
    if let Value::Str(parts) = value
        && let [StrPart::Lit(s)] = parts.as_slice()
    {
        return s.clone();
    }
    "APPLOWS_UNKNOWN".to_string()
}

/// 算術式を PowerShell へ。除算だけは特別扱い: PowerShell の `/` は浮動小数除算のため、
/// sh の整数除算 (0 方向への切り捨て) に合わせて `[long][math]::Truncate(...)` を使う。
fn ps_arith(op: ArithOp, l: &str, r: &str) -> String {
    match op {
        ArithOp::Div => format!("[long][math]::Truncate([double]{l} / [double]{r})"),
        _ => format!("({l} {} {r})", arith_op(op)),
    }
}

fn arith_op(op: ArithOp) -> &'static str {
    match op {
        ArithOp::Add => "+",
        ArithOp::Sub => "-",
        ArithOp::Mul => "*",
        ArithOp::Div => "/",
        ArithOp::Mod => "%",
    }
}

fn num_op(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Eq => "-eq",
        CmpOp::Ne => "-ne",
        CmpOp::Lt => "-lt",
        CmpOp::Le => "-le",
        CmpOp::Gt => "-gt",
        CmpOp::Ge => "-ge",
    }
}

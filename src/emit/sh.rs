//! sh バックエンド。Core IR を macOS `/bin/sh` (bash) + zsh 向けコードへ変換する。
//!
//! 方針:
//! - 副作用のある値 (run / ユーザ関数 / http_download) は一時変数 `__ap_tN` に正規化してから使う。
//!   これにより入れ子・条件内でもコマンドが 1 回だけ実行され、両バックエンドで挙動が揃う。
//! - 文字列補間は single quote リテラルと `"$var"` の連結で組み立てる。
//! - `arg()` / `args()` / `argc()` はトップレベル (= スクリプト引数) 前提 (sema が関数内使用を禁止)。

use crate::ast::{ArithOp, CmpOp};
use crate::builtins::Builtin;
use crate::emit::escape::sh_squote;
use crate::ir::{Cond, IrFunc, IrProgram, IrStmt, List, StrPart, Value};

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
        collect_assigned(&func.body, &mut assigned);
        for v in assigned {
            if !func.params.contains(&v) {
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
                self.line(&format!("{var}={s}"));
                let end_tmp = self.fresh_temp();
                self.line(&format!("{end_tmp}={en}"));
                self.line(&format!("while [ \"${var}\" -le \"${end_tmp}\" ]; do"));
                self.indent += 1;
                self.emit_stmts(body);
                self.line(&format!("{var}=$(({var} + 1))"));
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
                self.emit_pre(&pre.clone());
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
                self.emit_pre(&pre.clone());
                pre.clear();
                self.line(&format!("printf '%s' {content} >> {path}"));
            }
            Builtin::Copy => {
                let from = self.materialize(&args[0], pre);
                let to = self.materialize(&args[1], pre);
                self.emit_pre(&pre.clone());
                pre.clear();
                self.line(&format!("cp -f -- {from} {to}"));
            }
            Builtin::Remove => {
                let path = self.materialize(&args[0], pre);
                self.emit_pre(&pre.clone());
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
            Builtin::HttpDownload => self.render_http_download(args, pre),
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
            StrPart::Lit(s) => out.push_str(&sh_squote(s)),
            StrPart::Var(v) => out.push_str(&format!("\"${{{v}}}\"")),
        }
    }
    out
}

/// env の第 1 引数 (リテラル文字列) から環境変数名を取り出す。
fn literal_name(value: &Value) -> String {
    if let Value::Str(parts) = value
        && let [StrPart::Lit(s)] = parts.as_slice()
    {
        return s.clone();
    }
    "APPLOWS_UNKNOWN".to_string()
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

/// 文列で代入されるすべての変数スロットを収集する (関数内 local 宣言用)。
fn collect_assigned(stmts: &[IrStmt], out: &mut Vec<String>) {
    for s in stmts {
        match s {
            IrStmt::Let { var, .. } => push_unique(out, var),
            IrStmt::ForRange { var, body, .. } | IrStmt::ForEach { var, body, .. } => {
                push_unique(out, var);
                collect_assigned(body, out);
            }
            IrStmt::If {
                branches,
                otherwise,
            } => {
                for (_, body) in branches {
                    collect_assigned(body, out);
                }
                if let Some(b) = otherwise {
                    collect_assigned(b, out);
                }
            }
            IrStmt::While { body, .. } => collect_assigned(body, out),
            _ => {}
        }
    }
}

fn push_unique(out: &mut Vec<String>, v: &str) {
    if !out.iter().any(|x| x == v) {
        out.push(v.to_string());
    }
}

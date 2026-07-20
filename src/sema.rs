//! 意味解析 (名前解決 + 型検査) と Core IR への lowering を 1 パスで行う。
//!
//! ここで担保する規則:
//! - 変数/関数は生成識別子 (`__ap_vN` / `__ap_fN`) に解決し、シェルの特殊変数と衝突させない。
//! - 暗黙の型変換・truthiness を認めない。条件には Bool、値には Text/Int のみ。
//! - `List` は変数へ束縛・補間できず、`run` の argv と `for` の反復にのみ現れる。
//! - 関数は値渡し・トップレベル定義のみ・**自身より後に定義された関数を呼べない**
//!   (直接/相互/前方参照の再帰をすべて禁止)。関数内から外側 (グローバル) 変数は参照不可。

use crate::ast::*;
use crate::builtins::{Builtin, Type};
use crate::diagnostic::Diagnostic;
use crate::ir::{self, Cond, IrFunc, IrProgram, IrStmt, List, Value};
use std::collections::HashMap;

pub fn compile_to_ir(program: &Program) -> Result<IrProgram, Vec<Diagnostic>> {
    let mut lw = Lowerer::new();
    lw.collect_funcs(&program.stmts);

    // 関数本体を定義順に lowering (再帰規則のため index を保持)
    let mut funcs = Vec::new();
    let mut fn_index = 0usize;
    for stmt in &program.stmts {
        if let Stmt::Func {
            name, params, body, ..
        } = stmt
        {
            if let Some(f) = lw.lower_func(fn_index, name, params, body) {
                funcs.push(f);
            }
            fn_index += 1;
        }
    }

    // トップレベル本体 (関数定義以外)
    let mut global = Scope::default();
    let body = lw.lower_block(&program.stmts, &mut global, None, true);

    if lw.diags.is_empty() {
        Ok(IrProgram { funcs, body })
    } else {
        Err(lw.diags)
    }
}

/// 変数の解決情報。
#[derive(Clone)]
struct VarInfo {
    slot: String,
    ty: Type,
}

/// スコープ (グローバル or 関数単位)。ブロックは新スコープを作らない (シェル準拠)。
#[derive(Default)]
struct Scope {
    vars: HashMap<String, VarInfo>,
}

/// 関数シグネチャ。
struct FuncSig {
    slot: String,
    arity: usize,
    index: usize,
}

struct Lowerer {
    funcs: HashMap<String, FuncSig>,
    var_counter: usize,
    diags: Vec<Diagnostic>,
}

impl Lowerer {
    fn new() -> Self {
        Self {
            funcs: HashMap::new(),
            var_counter: 0,
            diags: Vec::new(),
        }
    }

    fn fresh_var(&mut self) -> String {
        let n = self.var_counter;
        self.var_counter += 1;
        format!("__ap_v{n}")
    }

    fn err(&mut self, d: Diagnostic) {
        self.diags.push(d);
    }

    /// トップレベルの関数定義を収集し、生成名・arity・index を登録する。
    fn collect_funcs(&mut self, stmts: &[Stmt]) {
        let mut index = 0usize;
        for stmt in stmts {
            if let Stmt::Func {
                name, params, span, ..
            } = stmt
            {
                if Builtin::from_name(name).is_some() {
                    self.err(Diagnostic::error(
                        format!("組み込み関数 `{name}` と同名の関数は定義できません"),
                        *span,
                    ));
                }
                if self.funcs.contains_key(name) {
                    self.err(Diagnostic::error(
                        format!("関数 `{name}` が二重定義されています"),
                        *span,
                    ));
                }
                let slot = format!("__ap_f{index}");
                self.funcs.insert(
                    name.clone(),
                    FuncSig {
                        slot,
                        arity: params.len(),
                        index,
                    },
                );
                index += 1;
            }
        }
    }

    fn lower_func(
        &mut self,
        index: usize,
        name: &str,
        params: &[String],
        body: &[Stmt],
    ) -> Option<IrFunc> {
        let slot = self.funcs.get(name)?.slot.clone();
        let mut scope = Scope::default();
        let mut slot_params = Vec::new();
        for p in params {
            let g = self.fresh_var();
            // パラメータは Text 型として扱う (MVP)
            scope.vars.insert(
                p.clone(),
                VarInfo {
                    slot: g.clone(),
                    ty: Type::Text,
                },
            );
            slot_params.push(g);
        }
        let ir_body = self.lower_block(body, &mut scope, Some(index), false);
        Some(IrFunc {
            name: slot,
            params: slot_params,
            body: ir_body,
        })
    }

    /// 文列を lowering。`fn_index` が Some ならその関数内、None ならトップレベル。
    /// `top_level` が true のときのみ関数定義を許す (定義自体は別途処理済みなのでスキップ)。
    fn lower_block(
        &mut self,
        stmts: &[Stmt],
        scope: &mut Scope,
        fn_index: Option<usize>,
        top_level: bool,
    ) -> Vec<IrStmt> {
        let mut out = Vec::new();
        for stmt in stmts {
            match self.lower_stmt(stmt, scope, fn_index, top_level) {
                Ok(Some(s)) => out.push(s),
                Ok(None) => {}
                Err(d) => self.err(d),
            }
        }
        out
    }

    fn lower_stmt(
        &mut self,
        stmt: &Stmt,
        scope: &mut Scope,
        fn_index: Option<usize>,
        top_level: bool,
    ) -> Result<Option<IrStmt>, Diagnostic> {
        match stmt {
            Stmt::Func { span, .. } => {
                if !top_level {
                    return Err(Diagnostic::error(
                        "関数はトップレベルでのみ定義できます",
                        *span,
                    ));
                }
                Ok(None) // 定義は collect_funcs / lower_func で処理済み
            }
            Stmt::Let { name, value, span } => {
                let (val, ty) = self.lower_value(value, scope, fn_index)?;
                if !matches!(ty, Type::Text | Type::Int) {
                    return Err(Diagnostic::error(
                        format!("`{}` 型の値は変数へ代入できません", ty.describe()),
                        *span,
                    )
                    .with_note("代入できるのは Text / Int のみ"));
                }
                let slot = match scope.vars.get(name) {
                    Some(info) => info.slot.clone(),
                    None => self.fresh_var(),
                };
                scope.vars.insert(
                    name.clone(),
                    VarInfo {
                        slot: slot.clone(),
                        ty,
                    },
                );
                Ok(Some(IrStmt::Let {
                    var: slot,
                    value: val,
                }))
            }
            Stmt::Print { value, span } => {
                let (val, ty) = self.lower_value(value, scope, fn_index)?;
                if !matches!(ty, Type::Text | Type::Int) {
                    return Err(Diagnostic::error(
                        format!("`{}` 型は print できません", ty.describe()),
                        *span,
                    ));
                }
                Ok(Some(IrStmt::Print { value: val }))
            }
            Stmt::ExprStmt { expr, span } => {
                let Expr::Call { name, args, .. } = expr else {
                    return Err(Diagnostic::error(
                        "式文は呼び出しである必要があります",
                        *span,
                    ));
                };
                self.lower_call_stmt(name, args, expr.span(), scope, fn_index)
                    .map(Some)
            }
            Stmt::If {
                branches,
                otherwise,
                ..
            } => {
                let mut ir_branches = Vec::new();
                for b in branches {
                    let cond = self.lower_cond(&b.cond, scope, fn_index)?;
                    let body = self.lower_block(&b.body, scope, fn_index, false);
                    ir_branches.push((cond, body));
                }
                let otherwise = otherwise
                    .as_ref()
                    .map(|b| self.lower_block(b, scope, fn_index, false));
                Ok(Some(IrStmt::If {
                    branches: ir_branches,
                    otherwise,
                }))
            }
            Stmt::While { cond, body, .. } => {
                let cond = self.lower_cond(cond, scope, fn_index)?;
                let body = self.lower_block(body, scope, fn_index, false);
                Ok(Some(IrStmt::While { cond, body }))
            }
            Stmt::For {
                var, iter, body, ..
            } => {
                let ir = match iter {
                    ForIter::Range { start, end } => {
                        let (s, st) = self.lower_value(start, scope, fn_index)?;
                        self.expect(st, Type::Int, start.span())?;
                        let (e, et) = self.lower_value(end, scope, fn_index)?;
                        self.expect(et, Type::Int, end.span())?;
                        let slot = self.declare_loop_var(var, Type::Int, scope);
                        let body = self.lower_block(body, scope, fn_index, false);
                        IrStmt::ForRange {
                            var: slot,
                            start: s,
                            end: e,
                            body,
                        }
                    }
                    ForIter::Each(list_expr) => {
                        let list = self.lower_list(list_expr, scope, fn_index)?;
                        let slot = self.declare_loop_var(var, Type::Text, scope);
                        let body = self.lower_block(body, scope, fn_index, false);
                        IrStmt::ForEach {
                            var: slot,
                            list,
                            body,
                        }
                    }
                };
                Ok(Some(ir))
            }
            Stmt::Return { value, span } => {
                if fn_index.is_none() {
                    return Err(Diagnostic::error("`return` は関数内でのみ使えます", *span));
                }
                let status = match value {
                    Some(e) => {
                        let (v, t) = self.lower_value(e, scope, fn_index)?;
                        self.expect(t, Type::Int, e.span())?;
                        v
                    }
                    None => Value::Int(0),
                };
                Ok(Some(IrStmt::Return { status }))
            }
            Stmt::Exit { code, .. } => {
                let (v, t) = self.lower_value(code, scope, fn_index)?;
                self.expect(t, Type::Int, code.span())?;
                Ok(Some(IrStmt::Exit { code: v }))
            }
        }
    }

    fn declare_loop_var(&mut self, name: &str, ty: Type, scope: &mut Scope) -> String {
        let slot = match scope.vars.get(name) {
            Some(info) => info.slot.clone(),
            None => self.fresh_var(),
        };
        scope.vars.insert(
            name.to_string(),
            VarInfo {
                slot: slot.clone(),
                ty,
            },
        );
        slot
    }

    /// 式文としての呼び出し (戻り値を捨てる)。副作用のある呼び出しのみ許可。
    fn lower_call_stmt(
        &mut self,
        name: &str,
        args: &[Expr],
        span: Span,
        scope: &mut Scope,
        fn_index: Option<usize>,
    ) -> Result<IrStmt, Diagnostic> {
        if let Some(builtin) = Builtin::from_name(name) {
            if !builtin.is_side_effecting() {
                return Err(Diagnostic::error(
                    format!("`{name}` の戻り値が使われていません",),
                    span,
                )
                .with_note("値を返す組み込みは `let x = ...` で受ける"));
            }
            let call = self.lower_builtin_call(builtin, args, span, scope, fn_index)?;
            Ok(IrStmt::Discard { call })
        } else if self.funcs.contains_key(name) {
            let call = self.lower_user_call(name, args, span, scope, fn_index)?;
            Ok(IrStmt::Discard { call })
        } else {
            Err(Diagnostic::error(format!("未定義の関数 `{name}`"), span))
        }
    }

    // ---- 値 (スカラ: Text / Int) ----

    fn lower_value(
        &mut self,
        expr: &Expr,
        scope: &mut Scope,
        fn_index: Option<usize>,
    ) -> Result<(Value, Type), Diagnostic> {
        match expr {
            Expr::Int { value, .. } => Ok((Value::Int(*value), Type::Int)),
            Expr::Str { parts, span } => {
                let mut out = Vec::new();
                for part in parts {
                    match part {
                        StrPart::Lit(s) => out.push(ir::StrPart::Lit(s.clone())),
                        StrPart::Var(name) => {
                            let info = self.lookup_var(name, scope, *span)?;
                            if !matches!(info.ty, Type::Text | Type::Int) {
                                return Err(Diagnostic::error(
                                    format!(
                                        "`{}` 型の変数 `{name}` は補間できません",
                                        info.ty.describe()
                                    ),
                                    *span,
                                ));
                            }
                            out.push(ir::StrPart::Var(info.slot));
                        }
                    }
                }
                Ok((Value::Str(out), Type::Text))
            }
            Expr::Var { name, span } => {
                let info = self.lookup_var(name, scope, *span)?;
                match info.ty {
                    Type::Text | Type::Int => Ok((Value::Var(info.slot), info.ty)),
                    other => Err(Diagnostic::error(
                        format!("`{}` 型の `{name}` は値として使えません", other.describe()),
                        *span,
                    )),
                }
            }
            Expr::Neg { expr: inner, span } => {
                if let Expr::Int { value, .. } = inner.as_ref() {
                    return Ok((Value::Int(-value), Type::Int));
                }
                let (v, t) = self.lower_value(inner, scope, fn_index)?;
                self.expect(t, Type::Int, *span)?;
                Ok((
                    Value::Arith {
                        op: ArithOp::Sub,
                        left: Box::new(Value::Int(0)),
                        right: Box::new(v),
                    },
                    Type::Int,
                ))
            }
            Expr::Arith {
                op,
                left,
                right,
                span,
            } => {
                let (l, lt) = self.lower_value(left, scope, fn_index)?;
                self.expect(lt, Type::Int, left.span())?;
                let (r, rt) = self.lower_value(right, scope, fn_index)?;
                self.expect(rt, Type::Int, right.span())?;
                let _ = span;
                Ok((
                    Value::Arith {
                        op: *op,
                        left: Box::new(l),
                        right: Box::new(r),
                    },
                    Type::Int,
                ))
            }
            Expr::Cmp { span, .. } | Expr::Logic { span, .. } | Expr::Not { span, .. } => {
                Err(Diagnostic::error("真偽値は値として使えません", *span)
                    .with_note("比較・論理式は if / while の条件でのみ使える"))
            }
            Expr::List { span, .. } => Err(Diagnostic::error("リストは値として使えません", *span)
                .with_note("リストは run([...]) の引数か for の反復にのみ使える")),
            Expr::Call { name, args, span } => {
                self.lower_value_call(name, args, *span, scope, fn_index)
            }
        }
    }

    fn lower_value_call(
        &mut self,
        name: &str,
        args: &[Expr],
        span: Span,
        scope: &mut Scope,
        fn_index: Option<usize>,
    ) -> Result<(Value, Type), Diagnostic> {
        if let Some(builtin) = Builtin::from_name(name) {
            match builtin.ret() {
                Type::Bool => Err(Diagnostic::error(
                    format!("`{name}` は Bool を返すため値にできません"),
                    span,
                )
                .with_note("Bool は if / while の条件でのみ使える")),
                Type::Unit => Err(Diagnostic::error(
                    format!("`{name}` は値を返しません"),
                    span,
                )),
                Type::List => Err(Diagnostic::error(
                    format!("`{name}` はリストを返すため値にできません"),
                    span,
                )),
                ret => {
                    let call = self.lower_builtin_call(builtin, args, span, scope, fn_index)?;
                    Ok((call, ret))
                }
            }
        } else if self.funcs.contains_key(name) {
            let call = self.lower_user_call(name, args, span, scope, fn_index)?;
            Ok((call, Type::Int))
        } else {
            Err(Diagnostic::error(format!("未定義の関数 `{name}`"), span))
        }
    }

    /// 組み込み呼び出しを Value へ (run は Value::Run、その他は Value::Builtin)。
    fn lower_builtin_call(
        &mut self,
        builtin: Builtin,
        args: &[Expr],
        span: Span,
        scope: &mut Scope,
        fn_index: Option<usize>,
    ) -> Result<Value, Diagnostic> {
        // arg()/argc() はスクリプト引数を指す。関数内では位置引数と紛れるためトップレベル限定。
        if matches!(builtin, Builtin::Arg | Builtin::Argc) && fn_index.is_some() {
            return Err(Diagnostic::error(
                format!(
                    "`{}` はトップレベルでのみ使えます (関数内では使えません)",
                    builtin.name()
                ),
                span,
            )
            .with_note("スクリプト引数はトップレベルで取得し、関数へは通常の引数として渡す"));
        }

        // run(list) は特別扱い
        if builtin == Builtin::Run {
            if args.len() != 1 {
                return Err(self.arity_err(builtin, args.len(), span));
            }
            let argv = self.lower_list(&args[0], scope, fn_index)?;
            if let List::Literal(items) = &argv
                && items.is_empty()
            {
                return Err(
                    Diagnostic::error("`run([])` の argv は空にできません", span)
                        .with_note("先頭要素は実行するコマンド名にする: run([\"cmd\", ...])"),
                );
            }
            return Ok(Value::Run { argv });
        }

        let params = builtin.params();
        if args.len() != params.len() {
            return Err(self.arity_err(builtin, args.len(), span));
        }

        // リテラル制約
        if builtin.requires_literal_first_arg() && !is_string_literal(&args[0]) {
            return Err(Diagnostic::error(
                format!(
                    "`{}` の第1引数は文字列リテラルである必要があります",
                    builtin.name()
                ),
                args[0].span(),
            ));
        }
        if builtin.requires_literal_int_arg() && !matches!(args[0], Expr::Int { .. }) {
            return Err(Diagnostic::error(
                format!(
                    "`{}` の引数は整数リテラルである必要があります",
                    builtin.name()
                ),
                args.first().map(|a| a.span()).unwrap_or(span),
            ));
        }

        let mut ir_args = Vec::new();
        for (arg, want) in args.iter().zip(params.iter()) {
            let (v, got) = self.lower_value(arg, scope, fn_index)?;
            self.expect(got, *want, arg.span())?;
            ir_args.push(v);
        }
        Ok(Value::Builtin {
            builtin,
            args: ir_args,
        })
    }

    fn lower_user_call(
        &mut self,
        name: &str,
        args: &[Expr],
        span: Span,
        scope: &mut Scope,
        fn_index: Option<usize>,
    ) -> Result<Value, Diagnostic> {
        let (slot, arity, callee_index) = {
            let sig = self.funcs.get(name).expect("caller checked existence");
            (sig.slot.clone(), sig.arity, sig.index)
        };
        if let Some(caller_index) = fn_index
            && callee_index >= caller_index
        {
            return Err(Diagnostic::error(
                format!("関数 `{name}` を呼べません (再帰・前方参照は禁止)"),
                span,
            )
            .with_note("関数は自身より前に定義された関数のみ呼べる"));
        }
        if args.len() != arity {
            return Err(Diagnostic::error(
                format!(
                    "関数 `{name}` は引数 {arity} 個ですが {} 個渡されました",
                    args.len()
                ),
                span,
            ));
        }
        let mut ir_args = Vec::new();
        for arg in args {
            let (v, t) = self.lower_value(arg, scope, fn_index)?;
            if !matches!(t, Type::Text | Type::Int) {
                return Err(Diagnostic::error(
                    "関数の引数は Text / Int のみです",
                    arg.span(),
                ));
            }
            ir_args.push(v);
        }
        Ok(Value::Call {
            name: slot,
            args: ir_args,
        })
    }

    // ---- 条件 (Bool) ----

    fn lower_cond(
        &mut self,
        expr: &Expr,
        scope: &mut Scope,
        fn_index: Option<usize>,
    ) -> Result<Cond, Diagnostic> {
        match expr {
            Expr::Cmp {
                op,
                left,
                right,
                span,
                ..
            } => {
                let (l, lt) = self.lower_value(left, scope, fn_index)?;
                let (r, rt) = self.lower_value(right, scope, fn_index)?;
                if lt != rt {
                    return Err(Diagnostic::error(
                        format!(
                            "比較の両辺の型が一致しません ({} と {})",
                            lt.describe(),
                            rt.describe()
                        ),
                        *span,
                    ));
                }
                let numeric = match lt {
                    Type::Int => true,
                    Type::Text => {
                        if !matches!(op, CmpOp::Eq | CmpOp::Ne) {
                            return Err(Diagnostic::error(
                                "文字列の大小比較はできません (== != のみ)",
                                *span,
                            ));
                        }
                        false
                    }
                    other => {
                        return Err(Diagnostic::error(
                            format!("`{}` 型は比較できません", other.describe()),
                            *span,
                        ));
                    }
                };
                Ok(Cond::Cmp {
                    op: *op,
                    numeric,
                    left: l,
                    right: r,
                })
            }
            Expr::Logic {
                op, left, right, ..
            } => {
                let l = self.lower_cond(left, scope, fn_index)?;
                let r = self.lower_cond(right, scope, fn_index)?;
                Ok(match op {
                    LogicOp::And => Cond::And(Box::new(l), Box::new(r)),
                    LogicOp::Or => Cond::Or(Box::new(l), Box::new(r)),
                })
            }
            Expr::Not { expr: inner, .. } => Ok(Cond::Not(Box::new(
                self.lower_cond(inner, scope, fn_index)?,
            ))),
            Expr::Call { name, args, span } => {
                let Some(builtin) = Builtin::from_name(name) else {
                    return Err(Diagnostic::error(
                        format!(
                            "条件に使えるのは比較・論理式・真偽値組み込みだけです (`{name}` は不可)"
                        ),
                        *span,
                    ));
                };
                if builtin.ret() != Type::Bool {
                    return Err(Diagnostic::error(
                        format!("`{name}` は Bool を返しません"),
                        *span,
                    )
                    .with_note("条件には exists / is_file / is_dir などの Bool 組み込みか比較を使う"));
                }
                let params = builtin.params();
                if args.len() != params.len() {
                    return Err(self.arity_err(builtin, args.len(), *span));
                }
                let mut ir_args = Vec::new();
                for (arg, want) in args.iter().zip(params.iter()) {
                    let (v, got) = self.lower_value(arg, scope, fn_index)?;
                    self.expect(got, *want, arg.span())?;
                    ir_args.push(v);
                }
                Ok(Cond::Test {
                    builtin,
                    args: ir_args,
                })
            }
            other => Err(Diagnostic::error("条件には真偽値が必要です", other.span())
                .with_note("truthiness は無い。例: `if run([...]) == 0` や `if exists(p)`")),
        }
    }

    // ---- リスト ----

    fn lower_list(
        &mut self,
        expr: &Expr,
        scope: &mut Scope,
        fn_index: Option<usize>,
    ) -> Result<List, Diagnostic> {
        match expr {
            Expr::List { items, .. } => {
                let mut out = Vec::new();
                for item in items {
                    let (v, t) = self.lower_value(item, scope, fn_index)?;
                    if !matches!(t, Type::Text | Type::Int) {
                        return Err(Diagnostic::error(
                            "リストの要素は Text / Int のみです",
                            item.span(),
                        ));
                    }
                    out.push(v);
                }
                Ok(List::Literal(out))
            }
            Expr::Call { name, args, span } if name == "args" => {
                if !args.is_empty() {
                    return Err(Diagnostic::error("`args()` は引数を取りません", *span));
                }
                if fn_index.is_some() {
                    return Err(Diagnostic::error(
                        "`args()` はトップレベルでのみ使えます (関数内では使えません)",
                        *span,
                    )
                    .with_note(
                        "スクリプト引数はトップレベルで取得し、関数へは通常の引数として渡す",
                    ));
                }
                Ok(List::Args)
            }
            other => Err(Diagnostic::error(
                "リストが必要です (リテラル `[...]` または `args()`)",
                other.span(),
            )),
        }
    }

    // ---- ヘルパ ----

    fn lookup_var(&mut self, name: &str, scope: &Scope, span: Span) -> Result<VarInfo, Diagnostic> {
        scope
            .vars
            .get(name)
            .cloned()
            .ok_or_else(|| Diagnostic::error(format!("未定義の変数 `{name}`"), span))
    }

    fn expect(&self, got: Type, want: Type, span: Span) -> Result<(), Diagnostic> {
        if got == want {
            Ok(())
        } else {
            Err(Diagnostic::error(
                format!(
                    "型が一致しません: {} を期待しましたが {} でした",
                    want.describe(),
                    got.describe()
                ),
                span,
            ))
        }
    }

    fn arity_err(&self, builtin: Builtin, got: usize, span: Span) -> Diagnostic {
        Diagnostic::error(
            format!(
                "`{}` は引数 {} 個ですが {} 個渡されました",
                builtin.name(),
                builtin.params().len(),
                got
            ),
            span,
        )
    }
}

/// 補間なしの単一リテラル文字列か。
fn is_string_literal(expr: &Expr) -> bool {
    matches!(expr, Expr::Str { parts, .. } if parts.iter().all(|p| matches!(p, StrPart::Lit(_))))
}

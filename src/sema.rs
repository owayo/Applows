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

/// スコープ (グローバル or 関数単位)。ブロックは新スコープを作らないが (シェル準拠)、
/// if/else の分岐だけはクローンして分離し、合流時に型を照合する。
#[derive(Default, Clone)]
struct Scope {
    vars: HashMap<String, VarInfo>,
    /// if の分岐で型/定義が食い違い、以降で安全に使えない変数名。
    diverged: std::collections::HashSet<String>,
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
                // パラメータ名の重複を検出 (片方が名前でアクセス不能になる typo を防ぐ)
                let mut seen = std::collections::HashSet::new();
                for p in params {
                    if !seen.insert(p.clone()) {
                        self.err(Diagnostic::error(
                            format!("関数 `{name}` のパラメータ名 `{p}` が重複しています"),
                            *span,
                        ));
                    }
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
                // 無条件の再代入は型を確定させるので diverged 状態を解除する
                scope.diverged.remove(name);
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
                // 各分岐を親スコープのクローン上で処理し、合流時に型を照合する。
                // (分岐間で型が食い違う変数は「diverged」とし、以降の使用をエラーにする)
                let before = scope.clone();
                let mut ir_branches = Vec::new();
                let mut path_scopes: Vec<Scope> = Vec::new();
                for b in branches {
                    // 条件は「どの分岐本体も未実行」の状態 (= before) で評価される
                    let cond = self.lower_cond(&b.cond, scope, fn_index, false)?;
                    let mut branch_scope = before.clone();
                    let body = self.lower_block(&b.body, &mut branch_scope, fn_index, false);
                    path_scopes.push(branch_scope);
                    ir_branches.push((cond, body));
                }
                let otherwise = match otherwise {
                    Some(b) => {
                        let mut else_scope = before.clone();
                        let body = self.lower_block(b, &mut else_scope, fn_index, false);
                        path_scopes.push(else_scope);
                        Some(body)
                    }
                    None => {
                        // else が無い = 「どの分岐にも入らない」パスがあり、変数は before のまま
                        path_scopes.push(before.clone());
                        None
                    }
                };
                *scope = merge_branch_scopes(&before, &path_scopes);
                Ok(Some(IrStmt::If {
                    branches: ir_branches,
                    otherwise,
                }))
            }
            Stmt::While { cond, body, .. } => {
                let cond = self.lower_cond(cond, scope, fn_index, false)?;
                // ループ本体はクローン上で処理し、代入された変数は while 後 diverged 扱いにする
                // (0 回実行の可能性があるため、本体で作られた型を後段で当てにできない)
                let before = scope.clone();
                let mut body_scope = before.clone();
                let body = self.lower_block(body, &mut body_scope, fn_index, false);
                *scope = merge_branch_scopes(&before, &[body_scope, before.clone()]);
                Ok(Some(IrStmt::While { cond, body }))
            }
            Stmt::For {
                var, iter, body, ..
            } => {
                // ループは 0 回実行され得る (空リスト / start>end) ため、本体はクローン上で
                // 処理し、本体で作られた型を while と同様に合流時 diverged 扱いにする。
                let before = scope.clone();
                let (ir, body_scope) = match iter {
                    ForIter::Range { start, end } => {
                        let (s, st) = self.lower_value(start, scope, fn_index)?;
                        self.expect(st, Type::Int, start.span())?;
                        let (e, et) = self.lower_value(end, scope, fn_index)?;
                        self.expect(et, Type::Int, end.span())?;
                        let mut body_scope = before.clone();
                        let slot = self.declare_loop_var(var, Type::Int, &mut body_scope);
                        let body = self.lower_block(body, &mut body_scope, fn_index, false);
                        (
                            IrStmt::ForRange {
                                var: slot,
                                start: s,
                                end: e,
                                body,
                            },
                            body_scope,
                        )
                    }
                    ForIter::Each(list_expr) => {
                        let list = self.lower_list(list_expr, scope, fn_index)?;
                        let mut body_scope = before.clone();
                        let slot = self.declare_loop_var(var, Type::Text, &mut body_scope);
                        let body = self.lower_block(body, &mut body_scope, fn_index, false);
                        (
                            IrStmt::ForEach {
                                var: slot,
                                list,
                                body,
                            },
                            body_scope,
                        )
                    }
                };
                *scope = merge_branch_scopes(&before, &[body_scope, before.clone()]);
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
        // env の変数名は識別子文字に限定する (生成コードの構文位置へ素通しさせない = コード注入防止)。
        if builtin == Builtin::Env {
            let name = string_literal_text(&args[0]).unwrap_or_default();
            if !is_valid_env_name(&name) {
                return Err(Diagnostic::error(
                    format!("環境変数名 `{name}` が不正です"),
                    args[0].span(),
                )
                .with_note("英字/アンダースコア始まりの英数字・アンダースコアのみ使用可 (例: PATH, HOME, MY_VAR)"));
            }
        }
        if builtin.requires_literal_int_arg() {
            match &args[0] {
                Expr::Int { value, .. } if *value >= 1 => {}
                Expr::Int { .. } => {
                    return Err(Diagnostic::error(
                        format!(
                            "`{}` のインデックスは 1 以上である必要があります",
                            builtin.name()
                        ),
                        args[0].span(),
                    )
                    .with_note("引数は 1 始まり (arg(1) が最初の引数)"));
                }
                _ => {
                    return Err(Diagnostic::error(
                        format!(
                            "`{}` の引数は整数リテラルである必要があります",
                            builtin.name()
                        ),
                        args.first().map(|a| a.span()).unwrap_or(span),
                    ));
                }
            }
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
        compound: bool,
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
                // and/or/not の内側では副作用のある呼び出しを禁止する。
                // 現状の生成コードは条件を短絡評価しないため、複合条件に run 等を書くと
                // 判定前に必ず実行されてしまう。let で受けてから比較させる。
                if compound && (value_has_side_effect(&l) || value_has_side_effect(&r)) {
                    return Err(Diagnostic::error(
                        "and/or/not の内側に副作用のある呼び出し (run / http_download / 関数呼び出し) は書けません",
                        *span,
                    )
                    .with_note("`let c = run([...])` で受けてから `c == 0` を条件に使う (条件は短絡評価されないため)"));
                }
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
                let l = self.lower_cond(left, scope, fn_index, true)?;
                let r = self.lower_cond(right, scope, fn_index, true)?;
                Ok(match op {
                    LogicOp::And => Cond::And(Box::new(l), Box::new(r)),
                    LogicOp::Or => Cond::Or(Box::new(l), Box::new(r)),
                })
            }
            Expr::Not { expr: inner, .. } => Ok(Cond::Not(Box::new(
                self.lower_cond(inner, scope, fn_index, true)?,
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
        if scope.diverged.contains(name) {
            return Err(Diagnostic::error(
                format!("変数 `{name}` は if/while の分岐で型が定まらないため使えません"),
                span,
            )
            .with_note(format!(
                "分岐の外で `let {name} = ...` と再代入して型を確定させてから使う"
            )));
        }
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

/// 補間なしリテラル文字列の中身を連結して取り出す (補間を含むなら None)。
fn string_literal_text(expr: &Expr) -> Option<String> {
    if let Expr::Str { parts, .. } = expr {
        let mut out = String::new();
        for p in parts {
            match p {
                StrPart::Lit(s) => out.push_str(s),
                StrPart::Var(_) => return None,
            }
        }
        Some(out)
    } else {
        None
    }
}

/// 値が副作用を持つか (run / ユーザ関数 / http_download 等)。複合条件での禁止判定に使う。
fn value_has_side_effect(v: &Value) -> bool {
    match v {
        Value::Run { .. } | Value::Call { .. } => true,
        Value::Builtin { builtin, args } => {
            builtin.is_side_effecting() || args.iter().any(value_has_side_effect)
        }
        Value::Arith { left, right, .. } => {
            value_has_side_effect(left) || value_has_side_effect(right)
        }
        Value::Int(_) | Value::Str(_) | Value::Var(_) => false,
    }
}

/// if/while の各パスのスコープを合流させる。
/// - `before` に既にある変数: 全パスで同じ型なら残す (slot は before のまま = 実行時も同一変数)。
///   型が食い違えば diverged にして以降の使用を禁止。
/// - 分岐内で新規導入された変数: パスごとに slot が異なり合流後に一意に定まらないため diverged。
fn merge_branch_scopes(before: &Scope, paths: &[Scope]) -> Scope {
    let mut merged = Scope {
        vars: std::collections::HashMap::new(),
        diverged: before.diverged.clone(),
    };
    for p in paths {
        merged.diverged.extend(p.diverged.iter().cloned());
    }
    // before にある変数の型整合性を確認
    for (name, info) in &before.vars {
        let consistent = paths
            .iter()
            .all(|p| p.vars.get(name).map(|v| v.ty) == Some(info.ty));
        if consistent && !merged.diverged.contains(name) {
            merged.vars.insert(
                name.clone(),
                VarInfo {
                    slot: info.slot.clone(),
                    ty: info.ty,
                },
            );
        } else {
            merged.diverged.insert(name.clone());
        }
    }
    // 分岐内で新規に導入された変数は合流後に使えない (slot がパスごとに異なる)
    for p in paths {
        for name in p.vars.keys() {
            if !before.vars.contains_key(name) {
                merged.diverged.insert(name.clone());
            }
        }
    }
    merged
}

/// シェル/PowerShell の環境変数名として妥当か (`^[A-Za-z_][A-Za-z0-9_]*$`)。
fn is_valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

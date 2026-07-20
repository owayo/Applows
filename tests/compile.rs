//! コンパイル成功/失敗と生成コードの不変条件を検査する統合テスト。

use applows::compile;

fn ok(src: &str) -> applows::CompileResult {
    match compile(src) {
        Ok(r) => r,
        Err(diags) => panic!(
            "コンパイル成功を期待:\n{}",
            diags
                .iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        ),
    }
}

/// コンパイル失敗を期待し、いずれかの診断が `needle` を含むことを確認する。
fn err_contains(src: &str, needle: &str) {
    match compile(src) {
        Ok(_) => panic!("コンパイル失敗を期待したが成功した: {src:?}"),
        Err(diags) => {
            let joined = diags
                .iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                joined.contains(needle),
                "診断に `{needle}` が含まれない。実際:\n{joined}"
            );
        }
    }
}

// ---- 成功ケース: 生成物の構造 ----

#[test]
fn hello_world_structure() {
    let r = ok("print \"hello\"\nexit 0\n");
    // ポリグロットの骨格
    assert!(r.output.starts_with("#!/bin/sh\n"));
    assert!(r.output.contains("function REM() { return; }"));
    assert!(r.output.contains("REM @'"));
    assert!(r.output.contains("\nAPPLOWS_BATCH\n"));
    // sh / ps ペイロード
    assert!(r.sh_payload.contains("printf '%s\\n' 'hello'"));
    assert!(r.ps_payload.contains("[Console]::Out.WriteLine('hello')"));
}

#[test]
fn output_invariants() {
    let r = ok("print \"x\"\nlet a = 1\nfor i in 1 to 3 { print \"{i}\" }\n");
    // CR / BOM / NUL を含まない
    assert!(!r.output.contains('\r'), "CR を含んではならない");
    assert!(!r.output.as_bytes().starts_with(&[0xEF, 0xBB, 0xBF]));
    assert!(!r.output.contains('\0'));
    // '@ 終端が列 0 にちょうど 1 つ
    let end = r.output.split('\n').filter(|l| l.starts_with("'@")).count();
    assert_eq!(end, 1, "'@ 終端は 1 つであるべき");
}

#[test]
fn identifiers_are_generated() {
    // ユーザ変数名 PATH がシェル特殊変数を汚さないこと (生成識別子に写像)
    let r = ok("let PATH = \"x\"\nprint \"{PATH}\"\n");
    assert!(r.sh_payload.contains("__ap_v0"), "生成識別子を使うべき");
    assert!(
        !r.sh_payload.contains("PATH="),
        "ユーザ名 PATH を直接代入してはならない"
    );
}

// ---- quoting ----

#[test]
fn sh_single_quote_escaping() {
    let r = ok("print \"it's a test\"\n");
    // sh: ' は '\'' で分割
    assert!(
        r.sh_payload.contains("'it'\\''s a test'"),
        "sh payload:\n{}",
        r.sh_payload
    );
}

#[test]
fn ps_single_quote_escaping() {
    let r = ok("print \"it's a test\"\n");
    // PS: ' は '' で二重化
    assert!(
        r.ps_payload.contains("'it''s a test'"),
        "ps payload:\n{}",
        r.ps_payload
    );
}

#[test]
fn special_chars_are_literal() {
    // Codex 指定の危険文字群。single quote に包まれ生コードにならないこと。
    let src = "print \"a$b `c` %d! ^e& |f< >g\"\n";
    let r = ok(src);
    assert!(r.sh_payload.contains("'a$b `c` %d! ^e& |f< >g'"));
    assert!(r.ps_payload.contains("'a$b `c` %d! ^e& |f< >g'"));
}

// ---- 失敗ケース (型/名前/スコープ) ----

#[test]
fn undefined_variable() {
    err_contains("print \"{missing}\"\n", "未定義の変数");
}

#[test]
fn undefined_function() {
    err_contains("nope(1)\n", "未定義の関数");
}

#[test]
fn type_mismatch_comparison() {
    err_contains(
        "let a = 1\nif a == \"x\" { print \"?\" }\n",
        "型が一致しません",
    );
}

#[test]
fn string_ordering_rejected() {
    err_contains("if \"a\" < \"b\" { print \"?\" }\n", "大小比較");
}

#[test]
fn bool_as_value_rejected() {
    err_contains("let x = exists(\"/tmp\")\n", "Bool");
}

#[test]
fn list_as_value_rejected() {
    err_contains("let x = args()\n", "リスト");
}

#[test]
fn condition_must_be_bool() {
    err_contains("if 3 { print \"?\" }\n", "真偽値");
}

#[test]
fn direct_recursion_rejected() {
    err_contains("fn f() {\n  return f()\n}\n", "再帰");
}

#[test]
fn forward_reference_rejected() {
    err_contains(
        "fn a() {\n  return b()\n}\nfn b() {\n  return 0\n}\n",
        "再帰",
    );
}

#[test]
fn function_arity_mismatch() {
    err_contains("fn f(x) {\n  return 0\n}\nf(1, 2)\n", "引数");
}

#[test]
fn env_name_must_be_literal() {
    err_contains("let p = \"PATH\"\nlet v = env(p, \"\")\n", "リテラル");
}

#[test]
fn return_outside_function() {
    err_contains("return 0\n", "関数内");
}

#[test]
fn func_inside_block_rejected() {
    err_contains("if 1 == 1 {\n  fn g() { return 0 }\n}\n", "トップレベル");
}

#[test]
fn comparison_chaining_rejected() {
    err_contains("let a = 1\nif a < 2 < 3 { print \"?\" }\n", "連鎖");
}

#[test]
fn discard_pure_value_rejected() {
    // 戻り値を使わない純粋組み込みは文にできない
    err_contains("upper(\"x\")\n", "戻り値");
}

// ---- スコープ: 関数内から外側変数は見えない ----

#[test]
fn function_cannot_see_globals() {
    err_contains(
        "let g = \"x\"\nfn f() {\n  print \"{g}\"\n  return 0\n}\n",
        "未定義の変数",
    );
}

#[test]
fn empty_run_argv_rejected() {
    err_contains("let c = run([])\n", "空にできません");
}

// ---- コードレビューで検出したバグの回帰テスト ----

#[test]
fn env_name_injection_rejected() {
    // env の変数名に注入的な文字列を渡すとコンパイルエラー
    err_contains(
        "let x = env(\"FOO:-$(touch /tmp/pwned)\", \"d\")\n",
        "環境変数名",
    );
    err_contains("let x = env(\"a b\", \"d\")\n", "環境変数名");
    // 正常な名前は通る
    ok("let x = env(\"PATH\", \"\")\n");
    ok("let x = env(\"MY_VAR\", \"default\")\n");
}

#[test]
fn if_else_type_divergence_rejected() {
    // 分岐で型が食い違う変数を if の後で使うとエラー
    err_contains(
        "if 1 == 1 {\n  let x = \"s\"\n} else {\n  let x = 2\n}\nlet y = x + 1\n",
        "型が定まらない",
    );
}

#[test]
fn consistent_branch_types_ok() {
    // 全分岐で同じ型なら if の後で使える
    ok(
        "let x = 0\nif 1 == 1 {\n  let x = 1\n} else {\n  let x = 2\n}\nlet y = x + 1\nprint \"{y}\"\n",
    );
}

#[test]
fn new_var_defined_in_all_branches_ok() {
    // 仕様の基本パターン: 両分岐で新規変数を同じ型で定義 → if の後で使える
    ok("if 1 == 1 {\n  let state = \"a\"\n} else {\n  let state = \"b\"\n}\nprint \"{state}\"\n");
}

#[test]
fn new_var_defined_in_only_one_branch_diverges() {
    // else 無しで片方の分岐でしか定義しない変数は後段で使えない
    err_contains(
        "if 1 == 1 {\n  let only = \"x\"\n}\nprint \"{only}\"\n",
        "定まらない",
    );
}

#[test]
fn for_each_side_effect_element_rejected() {
    err_contains(
        "for x in [run([\"true\"])] {\n  print \"{x}\"\n}\n",
        "副作用",
    );
    ok("for x in [\"a\", \"b\"] {\n  print \"{x}\"\n}\n");
}

#[test]
fn loop_var_reassign_keeps_slot() {
    // #1 回帰: ループ変数を分岐で発散させ再代入しても slot が分離しない
    let r = compile(
        "for i in 1 to 2 {\n  if 1 == 1 {\n    let i = \"x\"\n  }\n  let i = 0\n}\nprint \"ok\"\n",
    )
    .expect("compiles");
    assert!(r.sh_payload.contains("__ap_v0=0"), "sh:\n{}", r.sh_payload);
    assert!(
        !r.sh_payload.contains("__ap_v1=0"),
        "別 slot に分離してはならない:\n{}",
        r.sh_payload
    );
}

#[test]
fn side_effect_in_compound_condition_rejected() {
    // and/or/not の内側に副作用のある呼び出しは書けない
    err_contains(
        "if 1 == 2 and run([\"true\"]) == 0 {\n  print \"x\"\n}\n",
        "副作用",
    );
    // 単独の比較なら run を条件に書ける
    ok("if run([\"true\"]) == 0 {\n  print \"ok\"\n}\n");
    // 純粋な複合条件 (比較 and exists) は通る
    ok("let n = 1\nif n > 0 and exists(\"/tmp\") {\n  print \"ok\"\n}\n");
}

#[test]
fn arg_index_must_be_positive() {
    err_contains("let a = arg(0)\n", "1 以上");
    ok("let a = arg(1)\n");
}

#[test]
fn duplicate_param_names_rejected() {
    err_contains("fn f(a, a) {\n  return 0\n}\nf(\"x\", \"y\")\n", "重複");
}

#[test]
fn loop_variable_not_usable_after_loop() {
    // 0 回実行され得るループの変数は後段で使えない
    err_contains(
        "for i in 1 to 3 {\n  print \"{i}\"\n}\nprint \"{i}\"\n",
        "型が定まらない",
    );
}

#[test]
fn while_body_cannot_retype_condition_var() {
    // 条件は毎周評価されるため、本体で条件変数の型を変えると破綻する → 禁止
    err_contains(
        "let n = 3\nwhile n > 0 {\n  let n = \"x\"\n}\n",
        "型を変えています",
    );
    ok("let n = 3\nwhile n > 0 {\n  let n = n - 1\n}\n");
}

#[test]
fn for_range_body_cannot_retype_loop_var() {
    err_contains("for i in 1 to 3 {\n  let i = \"x\"\n}\n", "ループ変数");
    ok("for i in 1 to 3 {\n  let i = i + 10\n}\n");
}

#[test]
fn args_only_at_top_level() {
    err_contains(
        "fn f() {\n  let x = arg(1)\n  return 0\n}\nf()\n",
        "トップレベルでのみ",
    );
    err_contains(
        "fn f() {\n  for a in args() {\n    print \"{a}\"\n  }\n  return 0\n}\nf()\n",
        "トップレベルでのみ",
    );
}

//! 字句・構文・エスケープ・コード生成の細部を検査する。

use applows::compile;

fn ok(src: &str) -> applows::CompileResult {
    compile(src).unwrap_or_else(|d| {
        panic!(
            "compile失敗:\n{}",
            d.iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        )
    })
}

#[test]
fn string_escapes() {
    // \n \t \\ \" \{ \} が正しくリテラル化される
    let r = ok("print \"a\\tb\\nc \\\\ \\\" \\{x\\}\"\n");
    // sh single quote 内に実タブ・実改行・バックスラッシュ・引用符・波括弧が入る
    assert!(
        r.sh_payload.contains("a\tb\nc \\ \" {x}"),
        "sh payload:\n{}",
        r.sh_payload
    );
}

#[test]
fn comments_ignored() {
    let r = ok("# leading comment\nprint \"x\"  # trailing comment\n# trailing full line\n");
    assert!(r.sh_payload.contains("printf '%s\\n' 'x'"));
    // コメント文字列がコードに漏れない
    assert!(!r.sh_payload.contains("comment"));
}

#[test]
fn multiline_list_in_run() {
    // 角括弧内では改行が無視され複数行リストが書ける
    let src =
        "let c = run([\n  \"echo\",\n  \"hello\",\n  \"world\",\n])\nif c == 0 { print \"ok\" }\n";
    let r = ok(src);
    assert!(
        r.sh_payload.contains("'echo' 'hello' 'world'"),
        "sh:\n{}",
        r.sh_payload
    );
}

#[test]
fn arithmetic_precedence() {
    // 2 + 3 * 4 = 14 (乗算が先)
    let r = ok("let x = 2 + 3 * 4\nprint \"{x}\"\n");
    // sh 算術式に (3 * 4) のネストが現れる
    assert!(
        r.sh_payload.contains("2 + (3 * 4)"),
        "sh:\n{}",
        r.sh_payload
    );
}

#[test]
fn paren_overrides_precedence() {
    let r = ok("let x = (2 + 3) * 4\nprint \"{x}\"\n");
    assert!(
        r.sh_payload.contains("(2 + 3) * 4"),
        "sh:\n{}",
        r.sh_payload
    );
}

#[test]
fn http_download_codegen() {
    let src = "let c = http_download(\"https://example.com/f\", \"out.bin\")\nif c == 0 { print \"ok\" }\n";
    let r = ok(src);
    // sh は curl、原子的置換 (part -> mv)
    assert!(r.sh_payload.contains("curl -fsSL"), "sh:\n{}", r.sh_payload);
    assert!(r.sh_payload.contains("mv -f"));
    // PowerShell は Invoke-WebRequest
    assert!(
        r.ps_payload.contains("Invoke-WebRequest"),
        "ps:\n{}",
        r.ps_payload
    );
    assert!(r.ps_payload.contains("Move-Item"));
}

#[test]
fn write_text_is_atomic() {
    let r = ok("write_text(\"cfg.txt\", \"data\")\n");
    // sh: 一時ファイルへ書いてから mv
    assert!(
        r.sh_payload.contains(".tmp.$$") && r.sh_payload.contains("mv -f"),
        "sh:\n{}",
        r.sh_payload
    );
    // PowerShell: UTF-8 BOM 無しで書いてから Move-Item
    assert!(
        r.ps_payload.contains("UTF8Encoding") && r.ps_payload.contains("Move-Item"),
        "ps:\n{}",
        r.ps_payload
    );
}

#[test]
fn empty_string_and_interpolation_only() {
    let r = ok("let e = \"\"\nlet name = \"x\"\nprint \"{name}\"\n");
    // 空文字列は '' に
    assert!(r.sh_payload.contains("__ap_v0=''"), "sh:\n{}", r.sh_payload);
}

#[test]
fn negative_numbers() {
    let r = ok("let x = -5\nlet y = 0 - 3\nprint \"{x}{y}\"\n");
    assert!(r.sh_payload.contains("__ap_v0=-5"));
}

#[test]
fn nested_functions_call_order() {
    // 後に定義した関数から前の関数は呼べる
    let src =
        "fn a() {\n  print \"a\"\n  return 0\n}\nfn b() {\n  let r = a()\n  return r\n}\nb()\n";
    let r = ok(src);
    assert!(r.sh_payload.contains("__ap_f0() {"));
    assert!(r.sh_payload.contains("__ap_f1() {"));
}

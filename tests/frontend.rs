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
fn powershell_integer_division() {
    // PS の / は浮動小数除算なので、整数除算に揃える ([long][math]::Truncate)
    let r = ok("let x = 7 / 2\nprint \"{x}\"\n");
    assert!(
        r.ps_payload.contains("[long][math]::Truncate"),
        "ps:\n{}",
        r.ps_payload
    );
    // sh は $(( )) の整数除算
    assert!(r.sh_payload.contains("7 / 2"), "sh:\n{}", r.sh_payload);
}

#[test]
fn powershell_run_has_launch_guard() {
    // 存在しないコマンドで全体終了しないよう、PS の run は try/catch で 127 に揃える
    let r = ok("let c = run([\"some-cmd\"])\nif c == 0 { print \"ok\" }\n");
    assert!(
        r.ps_payload.contains("catch { $__ap_t0 = 127 }"),
        "ps:\n{}",
        r.ps_payload
    );
}

#[test]
fn newline_in_string_powershell_safe() {
    // 改行を含む文字列: PS の single-quoted は複数行不可なので [char]10 へ退避する
    let r = ok("print \"line1\\nline2\"\n");
    // sh は single quote に生 LF を含められる
    assert!(
        r.sh_payload.contains("'line1\nline2'"),
        "sh:\n{}",
        r.sh_payload
    );
    // PS 側の文字列に生 LF が single quote 内で現れてはならない
    assert!(r.ps_payload.contains("[char]10"), "ps:\n{}", r.ps_payload);
    for line in r.ps_payload.lines() {
        // 各行内で開いた single quote がその行で閉じている (複数行 single-quoted が無い)
        let single_quotes = line.matches('\'').count();
        assert!(
            single_quotes.is_multiple_of(2),
            "PS 行内で single quote が閉じていない: {line}"
        );
    }
}

#[test]
fn cr_in_string_no_raw_cr_in_output() {
    // CR は verify の CR 禁止に触れないよう退避され、コンパイルが通る
    let r = ok("print \"a\\rb\"\n");
    assert!(!r.output.contains('\r'), "出力に生 CR があってはならない");
    assert!(
        r.sh_payload.contains("printf '\\r'"),
        "sh:\n{}",
        r.sh_payload
    );
    assert!(r.ps_payload.contains("[char]13"), "ps:\n{}", r.ps_payload);
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

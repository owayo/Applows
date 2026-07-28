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
fn ps_string_comparison_is_case_sensitive() {
    // PS の -eq / -ne は大文字小文字を無視するため、-ceq / -cne を使う
    let r = ok("if \"a\" == \"A\" {\n  print \"eq\"\n}\nif \"a\" != \"b\" {\n  print \"ne\"\n}\n");
    assert!(r.ps_payload.contains("-ceq"), "ps:\n{}", r.ps_payload);
    assert!(r.ps_payload.contains("-cne"), "ps:\n{}", r.ps_payload);
    assert!(
        !r.ps_payload.contains("-eq ") && !r.ps_payload.contains("-ne "),
        "case-insensitive 比較を使ってはならない:\n{}",
        r.ps_payload
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

#[test]
fn read_stdin_codegen() {
    let r = ok("let s = read_stdin()\nprint \"{s}\"\n");
    // sh: tty から起動されたときに入力待ちで固まらないよう [ -t 0 ] で分岐する
    assert!(r.sh_payload.contains("[ -t 0 ]"), "sh:\n{}", r.sh_payload);
    assert!(r.sh_payload.contains("$(cat)"), "sh:\n{}", r.sh_payload);
    // PowerShell: 既定の [Console]::In は OEM コードページになり得るので UTF-8 を明示する
    assert!(
        r.ps_payload.contains("[Console]::IsInputRedirected"),
        "ps:\n{}",
        r.ps_payload
    );
    assert!(
        r.ps_payload.contains("OpenStandardInput"),
        "ps:\n{}",
        r.ps_payload
    );
    assert!(
        r.ps_payload.contains("UTF8Encoding $false"),
        "ps:\n{}",
        r.ps_payload
    );
}

#[test]
fn hostname_codegen() {
    let r = ok("let h = hostname()\nprint \"{h}\"\n");
    assert!(r.sh_payload.contains("uname -n"), "sh:\n{}", r.sh_payload);
    assert!(
        r.ps_payload.contains("[System.Net.Dns]::GetHostName()"),
        "ps:\n{}",
        r.ps_payload
    );
}

#[test]
fn http_post_uses_curl_on_both_targets() {
    let src = "let rc = http_post(\"https://example.com/api\", [\"Content-Type: application/json\"], \"{}\")\nprint \"{rc}\"\n"
        .replace("{}", "\\{\\}");
    let r = ok(&src);
    assert!(
        r.sh_payload.contains("curl --fail"),
        "sh:\n{}",
        r.sh_payload
    );
    // PowerShell 5.1 の Invoke-WebRequest は body のエンコーディングと HTTP エラーの
    // 扱いが curl と違い、送信バイト列が sh 側と一致しないため curl.exe を使う。
    assert!(
        r.ps_payload.contains("curl.exe --fail"),
        "ps:\n{}",
        r.ps_payload
    );
    assert!(
        !r.ps_payload.contains("Invoke-WebRequest"),
        "http_post は Invoke-WebRequest を使わない:\n{}",
        r.ps_payload
    );
}

#[test]
fn http_post_never_puts_headers_on_the_command_line() {
    let src = "let t = env(\"TOKEN\", \"\")\nlet rc = http_post(\"https://example.com/api\", [\"Authorization: Bearer {t}\"], \"body\")\nprint \"{rc}\"\n";
    let r = ok(src);
    // 秘密情報が `ps` から読めてしまうため、ヘッダは必ずファイル経由 (--header @file)
    for payload in [&r.sh_payload, &r.ps_payload] {
        assert!(
            payload.contains("--header @") || payload.contains("--header \"@"),
            "ヘッダはファイル経由で渡すこと:\n{payload}"
        );
        assert!(
            !payload.contains("--header \"Authorization"),
            "ヘッダ値をコマンドラインに載せてはいけない:\n{payload}"
        );
    }
}

#[test]
fn http_post_does_not_follow_redirects() {
    let src = "let rc = http_post(\"https://example.com/api\", [], \"body\")\nprint \"{rc}\"\n";
    let r = ok(src);
    // リダイレクト先へ Authorization を漏らさないため --location は付けない
    for payload in [&r.sh_payload, &r.ps_payload] {
        assert!(
            !payload.contains("--location"),
            "リダイレクトは追わない:\n{payload}"
        );
    }
}

#[test]
fn http_post_with_empty_headers_emits_no_loop() {
    let src = "let rc = http_post(\"https://example.com/api\", [], \"body\")\nprint \"{rc}\"\n";
    let r = ok(src);
    // 空リテラルで `for x in ; do` を出すと sh 構文エラーになる
    assert!(!r.sh_payload.contains("in ; do"), "sh:\n{}", r.sh_payload);
}

#[test]
fn run_capture_discards_stderr_and_normalizes_on_both_targets() {
    let src = "let x = run_capture([\"git\", \"rev-parse\"], \"\")\nprint \"{x}\"\n";
    let r = ok(src);

    // sh: stdout だけ取り、CR を落とす。`VAR="$(...)"` の終了ステータスで採否を決める
    assert!(
        r.sh_payload.contains("2>/dev/null"),
        "sh:\n{}",
        r.sh_payload
    );
    assert!(
        r.sh_payload.contains("tr -d '\\r'"),
        "sh:\n{}",
        r.sh_payload
    );

    // PS: 起動失敗は catch、非 0 終了は $LASTEXITCODE で弾く。
    // 呼び出し先が $LASTEXITCODE を更新しないケースに備えて実行前に 0 へ戻す。
    assert!(r.ps_payload.contains("2>$null"), "ps:\n{}", r.ps_payload);
    assert!(
        r.ps_payload.contains("$global:LASTEXITCODE = 0"),
        "ps:\n{}",
        r.ps_payload
    );
    assert!(
        r.ps_payload.contains("if ($LASTEXITCODE -eq 0)"),
        "ps:\n{}",
        r.ps_payload
    );
    // 末尾 LF を落として sh の $() と揃える
    assert!(r.ps_payload.contains("TrimEnd"), "ps:\n{}", r.ps_payload);
}

#[test]
fn run_capture_with_args_guards_empty_argv() {
    let src = "let x = run_capture(args(), \"d\")\nprint \"{x}\"\n";
    let r = ok(src);
    // 引数 0 個でコマンド未指定にならないよう、両バックエンドでガードする
    assert!(
        r.sh_payload.contains("[ \"$#\" -gt 0 ]"),
        "sh:\n{}",
        r.sh_payload
    );
    assert!(
        r.ps_payload.contains("$__ap_args.Count -gt 0"),
        "ps:\n{}",
        r.ps_payload
    );
}

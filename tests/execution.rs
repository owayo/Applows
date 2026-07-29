//! 実行 E2E テスト (macOS 限定)。
//!
//! 生成した `.bat` を `/bin/sh` と `zsh` で実際に実行し、標準出力と終了コードを検証する。
//! Windows PowerShell 側は CI (windows-latest) で検証する。
//!
//! Applows がサポートする Unix ターゲットは **macOS の `/bin/sh` (bash) と zsh** であり、
//! Linux は対象外 (ポリグロットヘッダが `function` キーワードを使うため、Debian 系の
//! `/bin/sh` = dash では解釈できない。README の Limitations 参照)。
//! そのため実行テストは macOS でのみ動かす。CI は macos-latest でこのテストを実行する。

#![cfg(target_os = "macos")]

use applows::compile;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// src をコンパイルして一時 .bat に書き、パスを返す。
fn build_temp(src: &str) -> PathBuf {
    let result = compile(src).unwrap_or_else(|diags| {
        panic!(
            "compile failed:\n{}",
            diags
                .iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        )
    });
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    let mut path = std::env::temp_dir();
    path.push(format!("applows_test_{pid}_{n}.bat"));
    std::fs::write(&path, result.output.as_bytes()).unwrap();
    path
}

/// 指定シェルで実行し (stdout, exit_code) を返す。
fn run(shell: &str, script: &PathBuf, args: &[&str]) -> (String, i32) {
    run_with_env(shell, script, args, &[])
}

/// 環境変数を追加指定して実行し (stdout, exit_code) を返す。
fn run_with_env(
    shell: &str,
    script: &PathBuf,
    args: &[&str],
    envs: &[(&str, &str)],
) -> (String, i32) {
    let out = Command::new(shell)
        .arg(script)
        .args(args)
        .envs(envs.iter().copied())
        .output()
        .unwrap_or_else(|e| panic!("{shell} の起動に失敗: {e}"));
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    (stdout, out.status.code().unwrap_or(-1))
}

/// sh と zsh の双方で実行し、出力が一致することを確認して返す。
/// zsh が無い環境 (一部の Linux CI) では sh のみ実行する。
fn run_both(script: &PathBuf, args: &[&str]) -> (String, i32) {
    let (sh_out, sh_code) = run("/bin/sh", script, args);
    if std::path::Path::new("/bin/zsh").exists() {
        let (zsh_out, zsh_code) = run("/bin/zsh", script, args);
        assert_eq!(sh_out, zsh_out, "sh と zsh で stdout が異なる");
        assert_eq!(sh_code, zsh_code, "sh と zsh で終了コードが異なる");
    }
    (sh_out, sh_code)
}

#[test]
fn hello_and_exit_code() {
    let script = build_temp("print \"hello\"\nexit 0\n");
    let (out, code) = run_both(&script, &[]);
    assert_eq!(out, "hello\n");
    assert_eq!(code, 0);
}

#[test]
fn nonzero_exit() {
    let script = build_temp("print \"bye\"\nexit 42\n");
    let (out, code) = run_both(&script, &[]);
    assert_eq!(out, "bye\n");
    assert_eq!(code, 42);
}

#[test]
fn arithmetic_and_loops() {
    let src = "let s = 0\nfor i in 1 to 4 {\n  let s = s + i\n}\nprint \"sum={s}\"\n";
    let script = build_temp(src);
    let (out, code) = run_both(&script, &[]);
    assert_eq!(out, "sum=10\n");
    assert_eq!(code, 0);
}

#[test]
fn while_countdown() {
    let src = "let n = 3\nwhile n > 0 {\n  print \"{n}\"\n  let n = n - 1\n}\n";
    let script = build_temp(src);
    let (out, _) = run_both(&script, &[]);
    assert_eq!(out, "3\n2\n1\n");
}

#[test]
fn for_range_loop_var_reassign_is_finite() {
    // ループ変数を本体で書き換えても隠しカウンタで反復するため無限ループにならない
    let src = "for i in 1 to 3 {\n  print \"iter={i}\"\n  let i = 0\n}\nprint \"done\"\n";
    let script = build_temp(src);
    let (out, code) = run_both(&script, &[]);
    assert_eq!(out, "iter=1\niter=2\niter=3\ndone\n");
    assert_eq!(code, 0);
}

#[test]
fn args_and_utf8() {
    let src = "let n = argc()\nprint \"n={n}\"\nfor a in args() {\n  print \"a={a}\"\n}\n";
    let script = build_temp(src);
    let (out, code) = run_both(&script, &["alpha", "two words", "日本語"]);
    assert_eq!(out, "n=3\na=alpha\na=two words\na=日本語\n");
    assert_eq!(code, 0);
}

#[test]
fn run_with_empty_args_returns_command_not_found() {
    let src = "let code = run(args())\nprint \"code={code}\"\n";
    let script = build_temp(src);
    let (out, code) = run_both(&script, &[]);
    assert_eq!(out, "code=127\n");
    assert_eq!(code, 0);
}

#[test]
fn signed_integer_boundaries() {
    let src = "print 9223372036854775807\nprint -9223372036854775808\n";
    let script = build_temp(src);
    let (out, code) = run_both(&script, &[]);
    assert_eq!(out, "9223372036854775807\n-9223372036854775808\n");
    assert_eq!(code, 0);
}

#[test]
fn for_range_at_integer_max_terminates() {
    let src = concat!(
        "for i in 9223372036854775807 to 9223372036854775807 {\n",
        "  print \"i={i}\"\n",
        "}\n",
    );
    let script = build_temp(src);
    let (out, code) = run_both(&script, &[]);
    assert_eq!(out, "i=9223372036854775807\n");
    assert_eq!(code, 0);
}

#[test]
fn conditionals() {
    let src = "let x = 5\nif x > 10 {\n  print \"big\"\n} else if x > 3 {\n  print \"mid\"\n} else {\n  print \"small\"\n}\n";
    let script = build_temp(src);
    let (out, _) = run_both(&script, &[]);
    assert_eq!(out, "mid\n");
}

#[test]
fn functions_and_run() {
    let src = "fn shout(msg) {\n  print \"[{msg}]\"\n  return 0\n}\nshout(\"hi\")\nlet c = run([\"true\"])\nif c == 0 {\n  print \"ran\"\n}\n";
    let script = build_temp(src);
    let (out, code) = run_both(&script, &[]);
    assert_eq!(out, "[hi]\nran\n");
    assert_eq!(code, 0);
}

#[test]
fn special_characters_roundtrip() {
    // single quote / 記号 / 日本語 が壊れずに出力される
    let src = "print \"it's $HOME & 100% <ok> 日本語 🌏\"\n";
    let script = build_temp(src);
    let (out, _) = run_both(&script, &[]);
    assert_eq!(out, "it's $HOME & 100% <ok> 日本語 🌏\n");
}

#[test]
fn string_builtins_roundtrip() {
    let src = "let u = upper(\"MixedCase abc\")\nlet l = lower(\"MixedCase ABC\")\nlet t = trim(\"  padded  \")\nprint \"u={u}\"\nprint \"l={l}\"\nprint \"t=[{t}]\"\n";
    let script = build_temp(src);
    let (out, code) = run_both(&script, &[]);
    assert_eq!(out, "u=MIXEDCASE ABC\nl=mixedcase abc\nt=[padded]\n");
    assert_eq!(code, 0);
}

#[test]
fn env_reads_variable_and_falls_back() {
    let src = "let v = env(\"APPLOWS_TEST_VAR\", \"fallback\")\nprint \"v={v}\"\n";
    let script = build_temp(src);
    // 未設定ならデフォルト値
    let (out, _) = run("/bin/sh", &script, &[]);
    assert_eq!(out, "v=fallback\n");
    // 設定済みならその値
    let (out, _) = run_with_env(
        "/bin/sh",
        &script,
        &[],
        &[("APPLOWS_TEST_VAR", "set-value")],
    );
    assert_eq!(out, "v=set-value\n");
    // 空文字列に設定されている場合は空のまま採用する (sh の :- ではなく - 相当の意味論)
    let (out, _) = run_with_env("/bin/sh", &script, &[], &[("APPLOWS_TEST_VAR", "")]);
    assert_eq!(out, "v=\n");
}

#[test]
fn file_predicates_and_copy() {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    let base = std::env::temp_dir();
    let f = base.join(format!("applows_pred_{pid}_{n}.txt"));
    let g = base.join(format!("applows_pred_copy_{pid}_{n}.txt"));
    let (fp, gp) = (f.display().to_string(), g.display().to_string());
    let src = format!(
        "write_text(\"{fp}\", \"x\")\nif exists(\"{fp}\") {{\n  print \"exists-ok\"\n}}\nif is_file(\"{fp}\") {{\n  print \"isfile-ok\"\n}}\nif is_dir(\"{fp}\") {{\n  print \"isdir-wrong\"\n}} else {{\n  print \"isdir-not-file\"\n}}\ncopy(\"{fp}\", \"{gp}\")\nlet c = read_text(\"{gp}\")\nprint \"copied={{c}}\"\nremove(\"{fp}\")\nremove(\"{gp}\")\nif exists(\"{fp}\") {{\n  print \"remove-failed\"\n}} else {{\n  print \"removed\"\n}}\n"
    );
    let script = build_temp(&src);
    let (out, code) = run("/bin/sh", &script, &[]);
    assert_eq!(
        out,
        "exists-ok\nisfile-ok\nisdir-not-file\ncopied=x\nremoved\n"
    );
    assert_eq!(code, 0);
    assert!(!f.exists() && !g.exists());
}

#[test]
fn string_equality_is_case_sensitive() {
    // PowerShell の -eq は大文字小文字を無視するため -ceq へ揃えている。
    // sh 側の実行でも大文字小文字を区別することを固定する。
    let src = "if \"abc\" == \"ABC\" {\n  print \"insensitive\"\n} else {\n  print \"sensitive\"\n}\nif \"x\" != \"y\" {\n  print \"neq-ok\"\n}\n";
    let script = build_temp(src);
    let (out, _) = run_both(&script, &[]);
    assert_eq!(out, "sensitive\nneq-ok\n");
}

#[test]
fn logic_operators_and_else_if() {
    let src = "let s = \"b\"\nif s == \"a\" {\n  print \"is-a\"\n} else if s == \"b\" {\n  print \"is-b\"\n} else {\n  print \"other\"\n}\nif 1 == 1 and 2 == 2 {\n  print \"and-ok\"\n}\nif 1 == 2 or 2 == 2 {\n  print \"or-ok\"\n}\nif not 1 == 2 {\n  print \"not-ok\"\n}\n";
    let script = build_temp(src);
    let (out, code) = run_both(&script, &[]);
    assert_eq!(out, "is-b\nand-ok\nor-ok\nnot-ok\n");
    assert_eq!(code, 0);
}

#[test]
fn negative_division_truncates_toward_zero() {
    // sh の $(( )) と PowerShell の [math]::Truncate で 0 方向への切り捨てに揃えている。
    let src = "let neg = 0 - 7\nlet q = neg / 2\nlet m = neg % 2\nprint \"q={q} m={m}\"\n";
    let script = build_temp(src);
    let (out, _) = run_both(&script, &[]);
    assert_eq!(out, "q=-3 m=-1\n");
}

#[test]
fn file_io_roundtrip() {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    let mut f = std::env::temp_dir();
    f.push(format!("applows_io_{pid}_{n}.txt"));
    let fp = f.display().to_string();
    let src = format!(
        "write_text(\"{fp}\", \"hello\\n\")\nappend_text(\"{fp}\", \"world\\n\")\nlet c = read_text(\"{fp}\")\nprint \"{{c}}\"\nremove(\"{fp}\")\n"
    );
    let script = build_temp(&src);
    let (out, code) = run("/bin/sh", &script, &[]);
    // read_text は $(cat) の仕様上、末尾改行を除去する (既知の制限)。
    // よって "hello\nworld\n" を読むと "hello\nworld" になり、print が改行を 1 つ足す。
    assert_eq!(out, "hello\nworld\n");
    assert_eq!(code, 0);
    assert!(!f.exists(), "remove でファイルが消えるべき");
}

#[test]
fn run_capture_takes_stdout_and_falls_back() {
    // 成功 -> stdout / 起動失敗・非 0 終了 -> default / 成功して空出力 -> 空文字 (default ではない)
    let src = concat!(
        "let ok = run_capture([\"printf\", \"hello\"], \"(default)\")\n",
        "print \"ok=[{ok}]\"\n",
        "let missing = run_capture([\"applows-no-such-command\"], \"(missing)\")\n",
        "print \"missing=[{missing}]\"\n",
        "let failed = run_capture([\"sh\", \"-c\", \"echo out; exit 3\"], \"(failed)\")\n",
        "print \"failed=[{failed}]\"\n",
        "let empty = run_capture([\"sh\", \"-c\", \"exit 0\"], \"(unused)\")\n",
        "print \"empty=[{empty}]\"\n",
        "let noise = run_capture([\"sh\", \"-c\", \"echo warn >&2; echo real\"], \"\")\n",
        "print \"noise=[{noise}]\"\n",
    );
    let script = build_temp(src);
    let (out, code) = run_both(&script, &[]);
    assert_eq!(
        out,
        "ok=[hello]\nmissing=[(missing)]\nfailed=[(failed)]\nempty=[]\nnoise=[real]\n"
    );
    assert_eq!(code, 0);
}

#[test]
fn run_capture_normalizes_line_endings() {
    // 末尾 LF は何個あっても全部落とし、CRLF は LF に、行途中の空行は保持する。
    // PowerShell 側 (行配列を LF 結合 + TrimEnd) と同じ結果になることを固定する。
    let src = concat!(
        "let trail = run_capture([\"sh\", \"-c\", \"printf 'z\\n\\n\\n'\"], \"\")\n",
        "print \"trail=[{trail}]\"\n",
        "let inner = run_capture([\"sh\", \"-c\", \"printf 'p\\n\\nq\\n'\"], \"\")\n",
        "print \"inner=[{inner}]\"\n",
        "let crlf = run_capture([\"sh\", \"-c\", \"printf 'x\\r\\ny\\r\\n'\"], \"\")\n",
        "print \"crlf=[{crlf}]\"\n",
        "let bare = run_capture([\"sh\", \"-c\", \"printf 'no-final-newline'\"], \"\")\n",
        "print \"bare=[{bare}]\"\n",
    );
    let script = build_temp(src);
    let (out, code) = run_both(&script, &[]);
    assert_eq!(
        out,
        "trail=[z]\ninner=[p\n\nq]\ncrlf=[x\ny]\nbare=[no-final-newline]\n"
    );
    assert_eq!(code, 0);
}

#[test]
fn run_capture_removes_nul_on_both_shells() {
    // /bin/sh はコマンド置換で NUL を暗黙除去するが zsh は保持するため、
    // コンパイラが明示的に除去して結果を揃える。
    let src = concat!(
        "let value = run_capture([\"sh\", \"-c\", \"printf 'a\\\\000b'\"], \"\")\n",
        "print \"value=[{value}]\"\n",
    );
    let script = build_temp(src);
    let (out, code) = run_both(&script, &[]);
    assert_eq!(out, "value=[ab]\n");
    assert_eq!(code, 0);
}

#[test]
fn run_capture_with_args_falls_back_when_empty() {
    // args() が空ならコマンド未指定なので実行せず default
    // (PowerShell 側の `$__ap_args.Count -gt 0` ガードと揃える)
    let src = "let out = run_capture(args(), \"(no args)\")\nprint \"out=[{out}]\"\n";
    let script = build_temp(src);

    let (out, code) = run_both(&script, &["/bin/echo", "hi"]);
    assert_eq!(out, "out=[hi]\n");
    assert_eq!(code, 0);

    let (out, code) = run_both(&script, &[]);
    assert_eq!(out, "out=[(no args)]\n");
    assert_eq!(code, 0);
}

#[test]
fn json_escape_covers_quotes_backslash_and_utf8() {
    let src = r#"let a = json_escape("he said \"hi\"")
print "a=[{a}]"
let b = json_escape("C:\\Users\\test")
print "b=[{b}]"
let c = json_escape("日本語 と 空白")
print "c=[{c}]"
"#;
    let script = build_temp(src);
    let (out, code) = run_both(&script, &[]);
    assert_eq!(
        out,
        "a=[he said \\\"hi\\\"]\nb=[C:\\\\Users\\\\test]\nc=[日本語 と 空白]\n"
    );
    assert_eq!(code, 0);
}

#[test]
fn json_add_inserts_before_closing_brace() {
    // JSON は解析せず末尾へ差し込むだけなので、入れ子も配列もそのまま残る
    let src = r#"let body = "\{\"session_id\":\"abc\",\"nested\":\{\"a\":1,\"b\":[1,2]\}\}"
let one = json_add(body, "project", "/tmp/proj")
print "one=[{one}]"
let two = json_add(one, "git_remote_url", "https://example.com/r.git")
print "two=[{two}]"
"#;
    let script = build_temp(src);
    let (out, code) = run_both(&script, &[]);
    assert_eq!(
        out,
        concat!(
            "one=[{\"session_id\":\"abc\",\"nested\":{\"a\":1,\"b\":[1,2]},\"project\":\"/tmp/proj\"}]\n",
            "two=[{\"session_id\":\"abc\",\"nested\":{\"a\":1,\"b\":[1,2]},\"project\":\"/tmp/proj\",\"git_remote_url\":\"https://example.com/r.git\"}]\n"
        )
    );
    assert_eq!(code, 0);
}

#[test]
fn json_add_handles_empty_object_and_non_object() {
    let src = r#"let a = json_add("\{\}", "k", "v")
print "a=[{a}]"
let b = json_add("\{ \}", "k", "v")
print "b=[{b}]"
let c = json_add("[1,2]", "k", "v")
print "c=[{c}]"
let d = json_add("\{\"a\":1\}", "k", "va\"l\\ue")
print "d=[{d}]"
let e = json_add("not-an-object\}", "k", "v")
print "e=[{e}]"
let f = json_add("  \{ \}", "k", "v")
print "f=[{f}]"
"#;
    let script = build_temp(src);
    let (out, code) = run_both(&script, &[]);
    assert_eq!(
        out,
        concat!(
            "a=[{\"k\":\"v\"}]\n",
            "b=[{\"k\":\"v\"}]\n",
            // top-level オブジェクトでなければ手を付けない
            "c=[[1,2]]\n",
            "d=[{\"a\":1,\"k\":\"va\\\"l\\\\ue\"}]\n",
            "e=[not-an-object}]\n",
            "f=[  {\"k\":\"v\"}]\n"
        )
    );
    assert_eq!(code, 0);
}

#[test]
fn for_each_materializes_list_values_before_iteration() {
    let src = concat!(
        "for item in [json_add(\"\\{\\}\", \"k\", \"v\")] {\n",
        "  print \"item=[{item}]\"\n",
        "}\n",
    );
    let script = build_temp(src);
    let (out, code) = run_both(&script, &[]);
    assert_eq!(out, "item=[{\"k\":\"v\"}]\n");
    assert_eq!(code, 0);
}

#[test]
fn run_capture_handles_utf8_and_spaces() {
    let src = concat!(
        "let s = run_capture([\"printf\", \"%s\", \"日本語 と 空白\"], \"\")\n",
        "print \"s=[{s}]\"\n",
    );
    let script = build_temp(src);
    let (out, code) = run_both(&script, &[]);
    assert_eq!(out, "s=[日本語 と 空白]\n");
    assert_eq!(code, 0);
}

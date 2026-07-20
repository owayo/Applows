//! 実行 E2E テスト (Unix 限定)。
//!
//! 生成した `.bat` を macOS/Linux の `/bin/sh` と `zsh` で実際に実行し、
//! 標準出力と終了コードを検証する。Windows PowerShell 側は CI (windows-latest) で検証する。

#![cfg(unix)]

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
    let out = Command::new(shell)
        .arg(script)
        .args(args)
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

//! 生成物の構造検査。
//!
//! ポリグロットが 3 環境で壊れない不変条件を、出力後に機械検査する。
//! 1 つでも違反があればコンパイルを失敗させる (バックエンドのバグを早期に捕捉)。

use crate::bootstrap::{BATCH_DELIM, PS_HEREDOC_END};
use crate::diagnostic::Diagnostic;

pub fn verify(output: &str) -> Result<(), Vec<Diagnostic>> {
    let mut errors = Vec::new();

    // バイト単位の不変条件
    let bytes = output.as_bytes();
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        errors.push(Diagnostic::plain(
            "出力に UTF-8 BOM が含まれています (sh のシバンが壊れます)",
        ));
    }
    if bytes.contains(&0x00) {
        errors.push(Diagnostic::plain("出力に NUL バイトが含まれています"));
    }
    if bytes.contains(&0x0D) {
        errors.push(Diagnostic::plain(
            "出力に CR (0x0D) が含まれています。改行は LF のみである必要があります",
        ));
    }

    // 行単位の不変条件
    let lines: Vec<&str> = output.split('\n').collect();

    // PowerShell here-string 終端 '@ は列 0 にちょうど 1 つ
    let heredoc_end = lines
        .iter()
        .filter(|l| l.starts_with(PS_HEREDOC_END))
        .count();
    if heredoc_end != 1 {
        errors.push(Diagnostic::plain(format!(
            "PowerShell here-string 終端 `{PS_HEREDOC_END}` が列 0 に {heredoc_end} 個あります (ちょうど 1 個であるべき)"
        )));
    }

    // here-string 開始 @' も 1 つ (REM @' 行)
    let heredoc_start = lines.iter().filter(|l| l.trim_end() == "REM @'").count();
    if heredoc_start != 1 {
        errors.push(Diagnostic::plain(format!(
            "here-string 開始行 `REM @'` が {heredoc_start} 個あります (ちょうど 1 個であるべき)"
        )));
    }

    // Batch ヒアドキュメント区切り (列 0 の単独行) はちょうど 1 つ
    let delim_lines = lines.iter().filter(|l| **l == BATCH_DELIM).count();
    if delim_lines != 1 {
        errors.push(Diagnostic::plain(format!(
            "Batch 区切り `{BATCH_DELIM}` の単独行が {delim_lines} 個あります (ちょうど 1 個であるべき)"
        )));
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

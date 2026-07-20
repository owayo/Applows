//! コンパイル診断 (エラー) と、ソース位置付きの人間可読な描画。
//!
//! 外部クレートに依存せず、`file:line:col` とキャレット付きの抜粋を出力する。

use crate::ast::Span;
use std::fmt;

/// コンパイルエラー 1 件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// エラーの本文。
    pub message: String,
    /// ソース上の位置 (無い場合もある)。
    pub span: Option<Span>,
    /// 補足のヒント (任意)。
    pub note: Option<String>,
}

impl Diagnostic {
    /// 位置付きエラーを作る。
    pub fn error(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span: Some(span),
            note: None,
        }
    }

    /// 位置なしエラーを作る。
    pub fn plain(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            span: None,
            note: None,
        }
    }

    /// ヒントを付ける。
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    /// ソース全体とファイル名を与えて人間可読に描画する。
    pub fn render(&self, source: &str, filename: &str) -> String {
        let mut out = String::new();
        match self.span {
            Some(span) => {
                out.push_str(&format!(
                    "error: {}\n  --> {}:{}:{}\n",
                    self.message, filename, span.line, span.col
                ));
                if let Some(line_text) = source.lines().nth(span.line.saturating_sub(1)) {
                    let gutter = format!("{} | ", span.line);
                    out.push_str(&format!("{gutter}{line_text}\n"));
                    let pad = " ".repeat(gutter.len() + span.col.saturating_sub(1));
                    out.push_str(&format!("{pad}^\n"));
                }
            }
            None => {
                out.push_str(&format!("error: {}\n", self.message));
            }
        }
        if let Some(note) = &self.note {
            out.push_str(&format!("  note: {note}\n"));
        }
        out
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.span {
            Some(span) => write!(f, "{}:{}: {}", span.line, span.col, self.message),
            None => write!(f, "{}", self.message),
        }
    }
}

impl std::error::Error for Diagnostic {}

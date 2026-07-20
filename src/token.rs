//! トークン定義。

use crate::ast::{Span, StrPart};

/// 位置情報付きトークン。
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokKind,
    pub span: Span,
}

/// トークンの種類。
#[derive(Debug, Clone, PartialEq)]
pub enum TokKind {
    // リテラル
    Int(i64),
    /// 文字列リテラル (補間パーツに分解済み)。
    Str(Vec<StrPart>),
    Ident(String),

    // キーワード
    Let,
    Print,
    If,
    Else,
    While,
    For,
    In,
    To,
    Fn,
    Return,
    Exit,
    And,
    Or,
    Not,
    True,
    False,

    // 区切り
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,

    // 演算子
    Assign,
    EqEq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,

    // 構造
    Newline,
    Eof,
}

impl TokKind {
    /// 予約語文字列をキーワードトークンへ。該当しなければ `None`。
    pub fn keyword(word: &str) -> Option<TokKind> {
        Some(match word {
            "let" => TokKind::Let,
            "print" => TokKind::Print,
            "if" => TokKind::If,
            "else" => TokKind::Else,
            "while" => TokKind::While,
            "for" => TokKind::For,
            "in" => TokKind::In,
            "to" => TokKind::To,
            "fn" => TokKind::Fn,
            "return" => TokKind::Return,
            "exit" => TokKind::Exit,
            "and" => TokKind::And,
            "or" => TokKind::Or,
            "not" => TokKind::Not,
            "true" => TokKind::True,
            "false" => TokKind::False,
            _ => return None,
        })
    }

    /// 人間可読なトークン名 (エラー表示用)。
    pub fn describe(&self) -> String {
        match self {
            TokKind::Int(_) => "整数".into(),
            TokKind::Str(_) => "文字列".into(),
            TokKind::Ident(name) => format!("識別子 `{name}`"),
            TokKind::Newline => "改行".into(),
            TokKind::Eof => "入力の終端".into(),
            other => format!("`{}`", other.symbol()),
        }
    }

    fn symbol(&self) -> &'static str {
        match self {
            TokKind::Let => "let",
            TokKind::Print => "print",
            TokKind::If => "if",
            TokKind::Else => "else",
            TokKind::While => "while",
            TokKind::For => "for",
            TokKind::In => "in",
            TokKind::To => "to",
            TokKind::Fn => "fn",
            TokKind::Return => "return",
            TokKind::Exit => "exit",
            TokKind::And => "and",
            TokKind::Or => "or",
            TokKind::Not => "not",
            TokKind::True => "true",
            TokKind::False => "false",
            TokKind::LParen => "(",
            TokKind::RParen => ")",
            TokKind::LBrace => "{",
            TokKind::RBrace => "}",
            TokKind::LBracket => "[",
            TokKind::RBracket => "]",
            TokKind::Comma => ",",
            TokKind::Assign => "=",
            TokKind::EqEq => "==",
            TokKind::Ne => "!=",
            TokKind::Lt => "<",
            TokKind::Le => "<=",
            TokKind::Gt => ">",
            TokKind::Ge => ">=",
            TokKind::Plus => "+",
            TokKind::Minus => "-",
            TokKind::Star => "*",
            TokKind::Slash => "/",
            TokKind::Percent => "%",
            _ => "?",
        }
    }
}

//! 字句解析。ソース文字列をトークン列へ変換する。
//!
//! - 空白 (スペース/タブ) は無視、改行は `Newline` トークンにする。
//! - ただし丸括弧 `()` と角括弧 `[]` の内側では改行を抑制し、複数行の式・リストを許す。
//! - `#` から行末まではコメント。
//! - 文字列 `"..."` は補間 `{ident}` と基本エスケープを解釈し `StrPart` 列へ分解する。

use crate::ast::{Span, StrPart};
use crate::diagnostic::Diagnostic;
use crate::token::{TokKind, Token};

pub fn lex(source: &str) -> Result<Vec<Token>, Diagnostic> {
    Lexer::new(source).run()
}

struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
    /// `()` `[]` のネスト深さ。> 0 の間は改行を抑制する。
    bracket_depth: usize,
}

impl Lexer {
    fn new(source: &str) -> Self {
        Self {
            chars: source.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
            bracket_depth: 0,
        }
    }

    fn span(&self) -> Span {
        Span::new(self.line, self.col)
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek2(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied()?;
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn run(mut self) -> Result<Vec<Token>, Diagnostic> {
        let mut tokens = Vec::new();
        loop {
            // 空白・コメントを飛ばす
            while let Some(c) = self.peek() {
                match c {
                    ' ' | '\t' | '\r' => {
                        self.bump();
                    }
                    '#' => {
                        while let Some(c) = self.peek() {
                            if c == '\n' {
                                break;
                            }
                            self.bump();
                        }
                    }
                    _ => break,
                }
            }

            let start = self.span();
            let Some(c) = self.peek() else {
                tokens.push(Token {
                    kind: TokKind::Eof,
                    span: start,
                });
                break;
            };

            match c {
                '\n' => {
                    self.bump();
                    if self.bracket_depth == 0 {
                        // 連続改行は 1 つにまとめる (パーサ側の空行処理を簡単にする)
                        if !matches!(
                            tokens.last().map(|t| &t.kind),
                            Some(TokKind::Newline) | None
                        ) {
                            tokens.push(Token {
                                kind: TokKind::Newline,
                                span: start,
                            });
                        }
                    }
                }
                '"' => {
                    let tok = self.lex_string(start)?;
                    tokens.push(tok);
                }
                c if c.is_ascii_digit() => {
                    tokens.push(self.lex_number(start)?);
                }
                c if is_ident_start(c) => {
                    tokens.push(self.lex_ident(start));
                }
                _ => {
                    tokens.push(self.lex_symbol(start)?);
                }
            }
        }
        Ok(tokens)
    }

    fn lex_number(&mut self, start: Span) -> Result<Token, Diagnostic> {
        let mut text = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                text.push(c);
                self.bump();
            } else if c == '_' {
                self.bump(); // 桁区切りは無視
            } else {
                break;
            }
        }
        let value: i64 = text
            .parse()
            .map_err(|_| Diagnostic::error(format!("整数リテラルが大きすぎます: {text}"), start))?;
        Ok(Token {
            kind: TokKind::Int(value),
            span: start,
        })
    }

    fn lex_ident(&mut self, start: Span) -> Token {
        let mut text = String::new();
        while let Some(c) = self.peek() {
            if is_ident_continue(c) {
                text.push(c);
                self.bump();
            } else {
                break;
            }
        }
        let kind = TokKind::keyword(&text).unwrap_or(TokKind::Ident(text));
        Token { kind, span: start }
    }

    fn lex_string(&mut self, start: Span) -> Result<Token, Diagnostic> {
        self.bump(); // 開き "
        let mut parts: Vec<StrPart> = Vec::new();
        let mut lit = String::new();
        loop {
            let Some(c) = self.peek() else {
                return Err(Diagnostic::error("文字列が閉じられていません", start));
            };
            match c {
                '"' => {
                    self.bump();
                    break;
                }
                '\\' => {
                    self.bump();
                    let Some(esc) = self.bump() else {
                        return Err(Diagnostic::error(
                            "文字列末尾のエスケープが不完全です",
                            start,
                        ));
                    };
                    match esc {
                        'n' => lit.push('\n'),
                        't' => lit.push('\t'),
                        'r' => lit.push('\r'),
                        '\\' => lit.push('\\'),
                        '"' => lit.push('"'),
                        '{' => lit.push('{'),
                        '}' => lit.push('}'),
                        other => {
                            return Err(Diagnostic::error(
                                format!("未知のエスケープ `\\{other}`"),
                                start,
                            )
                            .with_note("使用可能: \\n \\t \\r \\\\ \\\" \\{ \\}"));
                        }
                    }
                }
                '{' => {
                    self.bump();
                    if !lit.is_empty() {
                        parts.push(StrPart::Lit(std::mem::take(&mut lit)));
                    }
                    let name = self.lex_interp_name(start)?;
                    parts.push(StrPart::Var(name));
                }
                _ => {
                    lit.push(c);
                    self.bump();
                }
            }
        }
        if !lit.is_empty() || parts.is_empty() {
            parts.push(StrPart::Lit(lit));
        }
        Ok(Token {
            kind: TokKind::Str(parts),
            span: start,
        })
    }

    /// 補間 `{ident}` の中身を読む (開き `{` は消費済み)。
    fn lex_interp_name(&mut self, start: Span) -> Result<String, Diagnostic> {
        let mut name = String::new();
        match self.peek() {
            Some(c) if is_ident_start(c) => {
                name.push(c);
                self.bump();
            }
            _ => {
                return Err(
                    Diagnostic::error("補間 `{...}` には変数名が必要です", start)
                        .with_note("例: \"hello {name}\" / リテラルの波括弧は \\{ \\}"),
                );
            }
        }
        while let Some(c) = self.peek() {
            if is_ident_continue(c) {
                name.push(c);
                self.bump();
            } else {
                break;
            }
        }
        match self.bump() {
            Some('}') => Ok(name),
            _ => Err(Diagnostic::error(
                "補間 `{...}` が `}` で閉じられていません",
                start,
            )),
        }
    }

    fn lex_symbol(&mut self, start: Span) -> Result<Token, Diagnostic> {
        let c = self.peek().unwrap();
        let two = self.peek2();
        let kind = match (c, two) {
            ('=', Some('=')) => {
                self.bump();
                self.bump();
                TokKind::EqEq
            }
            ('!', Some('=')) => {
                self.bump();
                self.bump();
                TokKind::Ne
            }
            ('<', Some('=')) => {
                self.bump();
                self.bump();
                TokKind::Le
            }
            ('>', Some('=')) => {
                self.bump();
                self.bump();
                TokKind::Ge
            }
            ('=', _) => {
                self.bump();
                TokKind::Assign
            }
            ('<', _) => {
                self.bump();
                TokKind::Lt
            }
            ('>', _) => {
                self.bump();
                TokKind::Gt
            }
            ('+', _) => {
                self.bump();
                TokKind::Plus
            }
            ('-', _) => {
                self.bump();
                TokKind::Minus
            }
            ('*', _) => {
                self.bump();
                TokKind::Star
            }
            ('/', _) => {
                self.bump();
                TokKind::Slash
            }
            ('%', _) => {
                self.bump();
                TokKind::Percent
            }
            (',', _) => {
                self.bump();
                TokKind::Comma
            }
            ('(', _) => {
                self.bump();
                self.bracket_depth += 1;
                TokKind::LParen
            }
            (')', _) => {
                self.bump();
                self.bracket_depth = self.bracket_depth.saturating_sub(1);
                TokKind::RParen
            }
            ('[', _) => {
                self.bump();
                self.bracket_depth += 1;
                TokKind::LBracket
            }
            (']', _) => {
                self.bump();
                self.bracket_depth = self.bracket_depth.saturating_sub(1);
                TokKind::RBracket
            }
            ('{', _) => {
                self.bump();
                TokKind::LBrace
            }
            ('}', _) => {
                self.bump();
                TokKind::RBrace
            }
            ('!', _) => {
                return Err(Diagnostic::error("`!` 単独は使えません", start)
                    .with_note("否定は `not`、不等号は `!=` を使う"));
            }
            (other, _) => {
                return Err(Diagnostic::error(
                    format!("予期しない文字 `{other}`"),
                    start,
                ));
            }
        };
        Ok(Token { kind, span: start })
    }
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_ascii_alphabetic()
}

fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_ascii_alphanumeric()
}

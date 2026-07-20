//! 構文解析。トークン列から Source AST を構築する再帰下降パーサ。
//!
//! 式は優先順位登り法 (Pratt 風) で解析する。優先順位 (低い順):
//! `or` < `and` < 比較 < `+ -` < `* /` `%` < 単項 `- not` < 一次式。

use crate::ast::*;
use crate::diagnostic::Diagnostic;
use crate::token::{TokKind, Token};

pub fn parse(tokens: Vec<Token>) -> Result<Program, Diagnostic> {
    Parser { tokens, pos: 0 }.parse_program()
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> &TokKind {
        &self.tokens[self.pos].kind
    }

    fn peek_span(&self) -> Span {
        self.tokens[self.pos].span
    }

    fn bump(&mut self) -> Token {
        let tok = self.tokens[self.pos].clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn eat(&mut self, kind: &TokKind) -> bool {
        if self.peek() == kind {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: &TokKind) -> Result<Token, Diagnostic> {
        if self.peek() == kind {
            Ok(self.bump())
        } else {
            Err(Diagnostic::error(
                format!(
                    "{} を期待しましたが {} が見つかりました",
                    kind.describe(),
                    self.peek().describe()
                ),
                self.peek_span(),
            ))
        }
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek(), TokKind::Newline) {
            self.bump();
        }
    }

    fn parse_program(&mut self) -> Result<Program, Diagnostic> {
        let mut stmts = Vec::new();
        self.skip_newlines();
        while !matches!(self.peek(), TokKind::Eof) {
            stmts.push(self.parse_stmt()?);
            self.skip_newlines();
        }
        Ok(Program { stmts })
    }

    fn parse_block(&mut self) -> Result<Block, Diagnostic> {
        self.expect(&TokKind::LBrace)?;
        let mut stmts = Vec::new();
        self.skip_newlines();
        while !matches!(self.peek(), TokKind::RBrace | TokKind::Eof) {
            stmts.push(self.parse_stmt()?);
            self.skip_newlines();
        }
        self.expect(&TokKind::RBrace)?;
        Ok(stmts)
    }

    fn parse_stmt(&mut self) -> Result<Stmt, Diagnostic> {
        let span = self.peek_span();
        match self.peek() {
            TokKind::Let => self.parse_let(span),
            TokKind::Print => {
                self.bump();
                let value = self.parse_expr()?;
                Ok(Stmt::Print { value, span })
            }
            TokKind::If => self.parse_if(span),
            TokKind::While => self.parse_while(span),
            TokKind::For => self.parse_for(span),
            TokKind::Fn => self.parse_fn(span),
            TokKind::Return => {
                self.bump();
                let value = if self.stmt_ends() {
                    None
                } else {
                    Some(self.parse_expr()?)
                };
                Ok(Stmt::Return { value, span })
            }
            TokKind::Exit => {
                self.bump();
                let code = if self.stmt_ends() {
                    Expr::Int { value: 0, span }
                } else {
                    self.parse_expr()?
                };
                Ok(Stmt::Exit { code, span })
            }
            _ => {
                // 式文 (コマンド/関数呼び出しのみ許可)
                let expr = self.parse_expr()?;
                match expr {
                    Expr::Call { .. } => Ok(Stmt::ExprStmt { expr, span }),
                    other => Err(Diagnostic::error(
                        "式文として使えるのは関数/コマンド呼び出しだけです",
                        other.span(),
                    )
                    .with_note(
                        "値を捨てる文は `run([...])` や `install(x)` のような呼び出しに限る",
                    )),
                }
            }
        }
    }

    /// 文の終端 (改行 / `}` / EOF) かどうか。
    fn stmt_ends(&self) -> bool {
        matches!(
            self.peek(),
            TokKind::Newline | TokKind::RBrace | TokKind::Eof
        )
    }

    fn parse_let(&mut self, span: Span) -> Result<Stmt, Diagnostic> {
        self.bump(); // let
        let name = self.expect_ident()?;
        self.expect(&TokKind::Assign)?;
        let value = self.parse_expr()?;
        Ok(Stmt::Let { name, value, span })
    }

    fn parse_if(&mut self, span: Span) -> Result<Stmt, Diagnostic> {
        self.bump(); // if
        let mut branches = Vec::new();
        let cond = self.parse_expr()?;
        let body = self.parse_block()?;
        branches.push(Branch { cond, body });

        let mut otherwise = None;
        while self.eat(&TokKind::Else) {
            if self.eat(&TokKind::If) {
                let cond = self.parse_expr()?;
                let body = self.parse_block()?;
                branches.push(Branch { cond, body });
            } else {
                otherwise = Some(self.parse_block()?);
                break;
            }
        }
        Ok(Stmt::If {
            branches,
            otherwise,
            span,
        })
    }

    fn parse_while(&mut self, span: Span) -> Result<Stmt, Diagnostic> {
        self.bump(); // while
        let cond = self.parse_expr()?;
        let body = self.parse_block()?;
        Ok(Stmt::While { cond, body, span })
    }

    fn parse_for(&mut self, span: Span) -> Result<Stmt, Diagnostic> {
        self.bump(); // for
        let var = self.expect_ident()?;
        self.expect(&TokKind::In)?;
        let first = self.parse_expr()?;
        let iter = if self.eat(&TokKind::To) {
            let end = self.parse_expr()?;
            ForIter::Range { start: first, end }
        } else {
            ForIter::Each(first)
        };
        let body = self.parse_block()?;
        Ok(Stmt::For {
            var,
            iter,
            body,
            span,
        })
    }

    fn parse_fn(&mut self, span: Span) -> Result<Stmt, Diagnostic> {
        self.bump(); // fn
        let name = self.expect_ident()?;
        self.expect(&TokKind::LParen)?;
        let mut params = Vec::new();
        while !matches!(self.peek(), TokKind::RParen) {
            params.push(self.expect_ident()?);
            if !self.eat(&TokKind::Comma) {
                break;
            }
        }
        self.expect(&TokKind::RParen)?;
        let body = self.parse_block()?;
        Ok(Stmt::Func {
            name,
            params,
            body,
            span,
        })
    }

    fn expect_ident(&mut self) -> Result<String, Diagnostic> {
        match self.peek().clone() {
            TokKind::Ident(name) => {
                self.bump();
                Ok(name)
            }
            other => Err(Diagnostic::error(
                format!(
                    "識別子を期待しましたが {} が見つかりました",
                    other.describe()
                ),
                self.peek_span(),
            )),
        }
    }

    // ---- 式 (優先順位登り法) ----

    fn parse_expr(&mut self) -> Result<Expr, Diagnostic> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.parse_and()?;
        while matches!(self.peek(), TokKind::Or) {
            let span = self.peek_span();
            self.bump();
            let right = self.parse_and()?;
            left = Expr::Logic {
                op: LogicOp::Or,
                left: Box::new(left),
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.parse_not()?;
        while matches!(self.peek(), TokKind::And) {
            let span = self.peek_span();
            self.bump();
            let right = self.parse_not()?;
            left = Expr::Logic {
                op: LogicOp::And,
                left: Box::new(left),
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    fn parse_not(&mut self) -> Result<Expr, Diagnostic> {
        if matches!(self.peek(), TokKind::Not) {
            let span = self.peek_span();
            self.bump();
            let expr = self.parse_not()?;
            Ok(Expr::Not {
                expr: Box::new(expr),
                span,
            })
        } else {
            self.parse_cmp()
        }
    }

    fn parse_cmp(&mut self) -> Result<Expr, Diagnostic> {
        let left = self.parse_add()?;
        let op = match self.peek() {
            TokKind::EqEq => CmpOp::Eq,
            TokKind::Ne => CmpOp::Ne,
            TokKind::Lt => CmpOp::Lt,
            TokKind::Le => CmpOp::Le,
            TokKind::Gt => CmpOp::Gt,
            TokKind::Ge => CmpOp::Ge,
            _ => return Ok(left),
        };
        let span = self.peek_span();
        self.bump();
        let right = self.parse_add()?;
        // 比較の連鎖 (a < b < c) は禁止して分かりやすくする
        if matches!(
            self.peek(),
            TokKind::EqEq | TokKind::Ne | TokKind::Lt | TokKind::Le | TokKind::Gt | TokKind::Ge
        ) {
            return Err(
                Diagnostic::error("比較演算子は連鎖できません", self.peek_span())
                    .with_note("`a < b and b < c` のように分ける"),
            );
        }
        Ok(Expr::Cmp {
            op,
            numeric: false,
            left: Box::new(left),
            right: Box::new(right),
            span,
        })
    }

    fn parse_add(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.parse_mul()?;
        loop {
            let op = match self.peek() {
                TokKind::Plus => ArithOp::Add,
                TokKind::Minus => ArithOp::Sub,
                _ => break,
            };
            let span = self.peek_span();
            self.bump();
            let right = self.parse_mul()?;
            left = Expr::Arith {
                op,
                left: Box::new(left),
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    fn parse_mul(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                TokKind::Star => ArithOp::Mul,
                TokKind::Slash => ArithOp::Div,
                TokKind::Percent => ArithOp::Mod,
                _ => break,
            };
            let span = self.peek_span();
            self.bump();
            let right = self.parse_unary()?;
            left = Expr::Arith {
                op,
                left: Box::new(left),
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, Diagnostic> {
        if matches!(self.peek(), TokKind::Minus) {
            let span = self.peek_span();
            self.bump();
            let expr = self.parse_unary()?;
            Ok(Expr::Neg {
                expr: Box::new(expr),
                span,
            })
        } else {
            self.parse_primary()
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, Diagnostic> {
        let span = self.peek_span();
        match self.peek().clone() {
            TokKind::Int(value) => {
                self.bump();
                Ok(Expr::Int { value, span })
            }
            TokKind::Str(parts) => {
                self.bump();
                Ok(Expr::Str { parts, span })
            }
            TokKind::True | TokKind::False => {
                // 真偽値リテラルは Cmp へ正規化 (1==1 / 1==0) して条件専用の Bool にする
                let is_true = matches!(self.bump().kind, TokKind::True);
                let one = Box::new(Expr::Int { value: 1, span });
                let rhs = Box::new(Expr::Int {
                    value: if is_true { 1 } else { 0 },
                    span,
                });
                Ok(Expr::Cmp {
                    op: CmpOp::Eq,
                    numeric: true,
                    left: one,
                    right: rhs,
                    span,
                })
            }
            TokKind::Ident(name) => {
                self.bump();
                if self.eat(&TokKind::LParen) {
                    let args = self.parse_call_args()?;
                    Ok(Expr::Call { name, args, span })
                } else {
                    Ok(Expr::Var { name, span })
                }
            }
            TokKind::LParen => {
                self.bump();
                let inner = self.parse_expr()?;
                self.expect(&TokKind::RParen)?;
                Ok(inner)
            }
            TokKind::LBracket => {
                self.bump();
                let mut items = Vec::new();
                while !matches!(self.peek(), TokKind::RBracket) {
                    items.push(self.parse_expr()?);
                    if !self.eat(&TokKind::Comma) {
                        break;
                    }
                }
                self.expect(&TokKind::RBracket)?;
                Ok(Expr::List { items, span })
            }
            other => Err(Diagnostic::error(
                format!("式を期待しましたが {} が見つかりました", other.describe()),
                span,
            )),
        }
    }

    fn parse_call_args(&mut self) -> Result<Vec<Expr>, Diagnostic> {
        let mut args = Vec::new();
        while !matches!(self.peek(), TokKind::RParen) {
            args.push(self.parse_expr()?);
            if !self.eat(&TokKind::Comma) {
                break;
            }
        }
        self.expect(&TokKind::RParen)?;
        Ok(args)
    }
}

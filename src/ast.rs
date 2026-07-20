//! OS 非依存の抽象構文木 (AST)。
//!
//! パーサが生成し、意味解析 (`sema`) が検証・脱糖 (desugar) してから、
//! 各バックエンドエミッタ (`emit::sh` / `emit::powershell`) が消費する。
//!
//! 設計原則:
//! - 値の世界は「文字列 (Str) / 整数 (Int) / リスト (List)」のみ。
//!   これは POSIX sh の最小公倍数に合わせた最下位共通分母である。
//! - 真偽値 (Bool) は条件文脈 (if/while の条件) にだけ存在し、値としては扱わない。
//!   sh に真偽型が無いため、値として持ち回ると両バックエンドで破綻するのを防ぐ。
//! - 比較演算の「数値比較か文字列比較か」は sema が型推論して `numeric` に確定させる。
//!   エミッタは推論を行わず `numeric` を読むだけ (sh/PS のマッピングを一致させるため)。

/// ソース上の位置 (エラー表示用、1 始まり)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub line: usize,
    pub col: usize,
}

impl Span {
    pub fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }
}

/// プログラム全体 = 文の列。
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub stmts: Vec<Stmt>,
}

/// 文のブロック。
pub type Block = Vec<Stmt>;

/// 文。
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// 変数の宣言/再代入: `let name = value`
    Let {
        name: String,
        value: Expr,
        span: Span,
    },
    /// 出力 (改行付き): `print expr`
    Print { value: Expr, span: Span },
    /// 式文 (戻り値を捨てる。主にコマンド実行やユーザ関数呼び出し用): `run("git","status")`
    ExprStmt { expr: Expr, span: Span },
    /// 条件分岐: `if cond { .. } else if cond { .. } else { .. }`
    If {
        branches: Vec<Branch>,
        otherwise: Option<Block>,
        span: Span,
    },
    /// 前置判定ループ: `while cond { .. }`
    While { cond: Expr, body: Block, span: Span },
    /// 反復ループ: `for x in <iter> { .. }`
    For {
        var: String,
        iter: ForIter,
        body: Block,
        span: Span,
    },
    /// 関数定義: `fn name(a, b) { .. }`
    Func {
        name: String,
        params: Vec<String>,
        body: Block,
        span: Span,
    },
    /// 関数からの復帰: `return` / `return expr`
    Return { value: Option<Expr>, span: Span },
    /// スクリプト終了: `exit` / `exit expr`
    Exit { code: Expr, span: Span },
}

/// if の 1 分岐 (条件 + 本体)。
#[derive(Debug, Clone, PartialEq)]
pub struct Branch {
    pub cond: Expr,
    pub body: Block,
}

/// for のイテレータ種別。
#[derive(Debug, Clone, PartialEq)]
pub enum ForIter {
    /// `for x in 1 to 10` (両端含む整数レンジ)
    Range { start: Expr, end: Expr },
    /// `for x in [a, b, c]` / リストを返す式
    Each(Expr),
}

/// 式。
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// 文字列リテラル (補間パーツの列)。
    Str { parts: Vec<StrPart>, span: Span },
    /// 整数リテラル。
    Int { value: i64, span: Span },
    /// リストリテラル。
    List { items: Vec<Expr>, span: Span },
    /// 変数参照。
    Var { name: String, span: Span },
    /// 比較。`numeric` は sema が確定 (true=数値比較 / false=文字列比較)。
    Cmp {
        op: CmpOp,
        numeric: bool,
        left: Box<Expr>,
        right: Box<Expr>,
        span: Span,
    },
    /// 算術 (整数のみ)。
    Arith {
        op: ArithOp,
        left: Box<Expr>,
        right: Box<Expr>,
        span: Span,
    },
    /// 論理積/論理和。
    Logic {
        op: LogicOp,
        left: Box<Expr>,
        right: Box<Expr>,
        span: Span,
    },
    /// 論理否定。
    Not { expr: Box<Expr>, span: Span },
    /// 単項マイナス。
    Neg { expr: Box<Expr>, span: Span },
    /// 関数呼び出し (組み込み or ユーザ定義)。
    Call {
        name: String,
        args: Vec<Expr>,
        span: Span,
    },
}

impl Expr {
    /// 式のソース位置。
    pub fn span(&self) -> Span {
        match self {
            Expr::Str { span, .. }
            | Expr::Int { span, .. }
            | Expr::List { span, .. }
            | Expr::Var { span, .. }
            | Expr::Cmp { span, .. }
            | Expr::Arith { span, .. }
            | Expr::Logic { span, .. }
            | Expr::Not { span, .. }
            | Expr::Neg { span, .. }
            | Expr::Call { span, .. } => *span,
        }
    }
}

/// 文字列リテラルの構成要素。
#[derive(Debug, Clone, PartialEq)]
pub enum StrPart {
    /// リテラル文字列片。
    Lit(String),
    /// 変数補間 `{name}`。
    Var(String),
}

/// 比較演算子。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// 算術演算子。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

/// 論理演算子。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicOp {
    And,
    Or,
}

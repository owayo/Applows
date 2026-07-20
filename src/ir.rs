//! Core IR。OS 非依存・正規化済みの中間表現。
//!
//! Source AST との違い:
//! - 変数/関数はすべて生成識別子 (`__ap_vN` / `__ap_fN`) に解決済み。
//! - 組み込み呼び出しは [`Builtin`] enum に解決済み、ユーザ関数呼び出しと区別済み。
//! - 比較の数値/文字列の別 (`numeric`) は確定済み。
//! - 真偽値は条件式にのみ現れる (`Cond`)。値の式 (`Value`) とは型で分離。
//! - リストは argv と for-each の反復のみに現れ、スカラ値の式には現れない。

use crate::ast::{ArithOp, CmpOp};
use crate::builtins::Builtin;

/// プログラム全体。
#[derive(Debug, Clone, PartialEq)]
pub struct IrProgram {
    pub funcs: Vec<IrFunc>,
    pub body: Vec<IrStmt>,
}

/// ユーザ定義関数。
#[derive(Debug, Clone, PartialEq)]
pub struct IrFunc {
    /// 生成名 `__ap_fN`。
    pub name: String,
    /// 生成したパラメータ変数名 `__ap_vN`。
    pub params: Vec<String>,
    pub body: Vec<IrStmt>,
}

/// 文。
#[derive(Debug, Clone, PartialEq)]
pub enum IrStmt {
    /// スカラ変数への代入。
    Let { var: String, value: Value },
    /// 改行付き出力。
    Print { value: Value },
    /// 値を捨てる式文 (副作用のある組み込み / ユーザ関数呼び出し)。
    Discard { call: Value },
    /// 条件分岐。
    If {
        branches: Vec<(Cond, Vec<IrStmt>)>,
        otherwise: Option<Vec<IrStmt>>,
    },
    /// 前置判定ループ。
    While { cond: Cond, body: Vec<IrStmt> },
    /// 整数レンジ反復 (両端含む)。
    ForRange {
        var: String,
        start: Value,
        end: Value,
        body: Vec<IrStmt>,
    },
    /// リスト反復。
    ForEach {
        var: String,
        list: List,
        body: Vec<IrStmt>,
    },
    /// 関数からの復帰 (Int ステータス)。
    Return { status: Value },
    /// スクリプト終了 (Int コード)。
    Exit { code: Value },
}

/// スカラ値の式 (Text または Int)。
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    /// 文字列 (補間パーツ列)。
    Str(Vec<StrPart>),
    /// スカラ変数参照 (生成名)。
    Var(String),
    /// 整数算術。
    Arith {
        op: ArithOp,
        left: Box<Value>,
        right: Box<Value>,
    },
    /// 外部コマンド実行。終了コードを返す。
    Run {
        argv: List,
    },
    /// 値を返す組み込み (env / read_text / upper / http_download / arg / argc / script_* / cwd ...)。
    Builtin {
        builtin: Builtin,
        args: Vec<Value>,
    },
    /// ユーザ関数呼び出し。Int ステータスを返す。
    Call {
        name: String,
        args: Vec<Value>,
    },
}

/// 文字列補間パーツ。
#[derive(Debug, Clone, PartialEq)]
pub enum StrPart {
    Lit(String),
    /// スカラ変数の補間 (生成名)。
    Var(String),
}

/// 条件式 (真偽値。条件文脈にのみ現れる)。
#[derive(Debug, Clone, PartialEq)]
pub enum Cond {
    /// 比較。`numeric=true` で数値比較、false で文字列比較。
    Cmp {
        op: CmpOp,
        numeric: bool,
        left: Value,
        right: Value,
    },
    And(Box<Cond>, Box<Cond>),
    Or(Box<Cond>, Box<Cond>),
    Not(Box<Cond>),
    /// 真偽値を返す組み込み (exists / is_file / is_dir)。
    Test {
        builtin: Builtin,
        args: Vec<Value>,
    },
}

/// リスト (argv / for-each 反復のみ)。
#[derive(Debug, Clone, PartialEq)]
pub enum List {
    /// リテラル。各要素は Text スカラ。
    Literal(Vec<Value>),
    /// `args()` — スクリプト引数全体。
    Args,
}

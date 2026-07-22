//! コード生成バックエンド間で共有する、ターゲット非依存の小さな変換。

use crate::ast::{ArithOp, CmpOp};
use crate::ir::{StrPart, Value};

/// env の第 1 引数 (リテラル文字列) から環境変数名を取り出す。
///
/// 通常は sema が単一リテラルであることを保証する。外部から不正な IR を渡された場合も
/// 生成コードの構文を壊さないよう、安全な名前へフォールバックする。
pub(super) fn literal_name(value: &Value) -> &str {
    if let Value::Str(parts) = value
        && let [StrPart::Lit(name)] = parts.as_slice()
    {
        return name;
    }
    "APPLOWS_UNKNOWN"
}

pub(super) fn arith_op(op: ArithOp) -> &'static str {
    match op {
        ArithOp::Add => "+",
        ArithOp::Sub => "-",
        ArithOp::Mul => "*",
        ArithOp::Div => "/",
        ArithOp::Mod => "%",
    }
}

/// POSIX test と PowerShell で共通する数値比較演算子。
pub(super) fn num_op(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Eq => "-eq",
        CmpOp::Ne => "-ne",
        CmpOp::Lt => "-lt",
        CmpOp::Le => "-le",
        CmpOp::Gt => "-gt",
        CmpOp::Ge => "-ge",
    }
}

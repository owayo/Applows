//! コード生成バックエンド。Core IR を各ターゲット構文へ変換する。

pub mod escape;
pub mod powershell;
pub mod sh;

pub use powershell::emit_powershell;
pub use sh::emit_sh;

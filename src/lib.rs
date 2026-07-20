//! Applows コンパイラライブラリ。
//!
//! シェル風の 1 ソース (`.aplo`) を、バニラ Windows 11 と macOS の双方で
//! 追加ランタイムなしに動く単一ポリグロットスクリプト (`.bat`) へコンパイルする。
//!
//! パイプライン: `lex → parse → sema(検査+lowering) → emit(sh/ps) → assemble → verify`。

pub mod ast;
pub mod bootstrap;
pub mod builtins;
pub mod diagnostic;
pub mod emit;
pub mod ir;
pub mod lexer;
pub mod parser;
pub mod sema;
pub mod skill;
pub mod token;
pub mod verify;

use diagnostic::Diagnostic;
use ir::IrProgram;

/// コンパイル成果物 (中間生成物も検査/テスト用に保持)。
#[derive(Debug, Clone)]
pub struct CompileResult {
    /// 最終ポリグロット出力 (LF・BOM 無し)。
    pub output: String,
    /// sh ペイロード (デバッグ/ゴールデン用)。
    pub sh_payload: String,
    /// PowerShell ペイロード (デバッグ/ゴールデン用)。
    pub ps_payload: String,
    /// Core IR (ゴールデン用)。
    pub ir: IrProgram,
}

/// ソースをコンパイルする。失敗時は診断の一覧を返す。
pub fn compile(source: &str) -> Result<CompileResult, Vec<Diagnostic>> {
    let tokens = lexer::lex(source).map_err(|d| vec![d])?;
    let program = parser::parse(tokens).map_err(|d| vec![d])?;
    let ir = sema::compile_to_ir(&program)?;

    let sh_payload = emit::emit_sh(&ir);
    let ps_payload = emit::emit_powershell(&ir);
    let output = bootstrap::assemble(&sh_payload, &ps_payload);

    verify::verify(&output)?;

    Ok(CompileResult {
        output,
        sh_payload,
        ps_payload,
        ir,
    })
}

/// ソースをコンパイルし、失敗時は人間可読な文字列にまとめて返す (CLI 用)。
pub fn compile_rendered(source: &str, filename: &str) -> Result<CompileResult, String> {
    compile(source).map_err(|diags| {
        diags
            .iter()
            .map(|d| d.render(source, filename))
            .collect::<Vec<_>>()
            .join("\n")
    })
}

//! ポリグロットブートストラップの組み立て。
//!
//! 検証済み固定テンプレート (macOS `/bin/sh`+zsh 実測済み) に、sh / PowerShell の
//! 2 ペイロードを差し込んで単一の `.bat` を生成する。

/// sh ヒアドキュメント / PowerShell here-string 内の Batch 区切り。
pub const BATCH_DELIM: &str = "APPLOWS_BATCH";

/// PowerShell here-string の終端 (この行が sh ペイロード内に現れてはならない)。
pub const PS_HEREDOC_END: &str = "'@";

/// 2 ペイロードを 3 環境共存テンプレートへ組み立てる。出力は LF のみ・BOM 無し。
pub fn assemble(sh_payload: &str, ps_payload: &str) -> String {
    let sh = sh_payload.trim_end_matches('\n');
    let ps = ps_payload.trim_end_matches('\n');

    let mut out = String::new();
    // --- ポリグロットヘッダ ---
    out.push_str("#!/bin/sh\n");
    out.push_str("function REM() { return; }\n");
    out.push_str("REM @'\n");
    out.push_str("REM '; : << '");
    out.push_str(BATCH_DELIM);
    out.push_str("'\n");
    // --- Batch セクション ---
    out.push_str("@echo off\n");
    out.push_str("set \"APPLOWS_SELF=%~f0\"\n");
    out.push_str(
        "powershell -NoProfile -ExecutionPolicy Bypass -Command \"$u=New-Object System.Text.UTF8Encoding $false; $s=[System.IO.File]::ReadAllText($env:APPLOWS_SELF,$u); $b=[ScriptBlock]::Create($s); & $b @args; exit $LASTEXITCODE\" %*\n",
    );
    out.push_str("exit /b %ERRORLEVEL%\n");
    out.push_str(BATCH_DELIM);
    out.push('\n');
    // --- sh ペイロード (macOS) ---
    out.push_str("# ==== Applows sh payload (macOS /bin/sh + zsh) ====\n");
    out.push_str(sh);
    out.push('\n');
    out.push_str("exit 0\n");
    out.push_str(PS_HEREDOC_END);
    out.push('\n');
    // --- PowerShell ペイロード (Windows) ---
    out.push_str("# ==== Applows PowerShell payload (Windows 11) ====\n");
    out.push_str(ps);
    out.push('\n');
    out
}

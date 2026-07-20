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
    // 引数転送は Codex レビューで判明した罠を回避する実装:
    //   `powershell -Command "..." %*` では %* が $args にならず、PowerShell が
    //   コマンド文字列末尾へ連結してしまう ("two words" で構文破壊)。
    // 対策: ネイティブのプロセス引数を [Environment]::GetCommandLineArgs() で取得し
    //   (index 6 以降がユーザ引数)、コマンド末尾の `#` で連結される %* をコメント化する。
    //   さらに setlocal DisableDelayedExpansion で引数中の `!` 消失を防ぎ、
    //   powershell.exe をフルパス起動、bootstrap 自体を try/catch で保護する。
    // ユーザ引数の開始位置は index 6 固定ではなく `-Command` を探して +2 で動的決定する
    // (grok レビュー: 起動フラグの増減で静かにずれる脆さを解消)。終了コードは
    // ペイロードが exit しなかった場合の保険として $global:__ap_ret を伝播する。
    out.push_str("@echo off\n");
    out.push_str("setlocal DisableDelayedExpansion\n");
    out.push_str("set \"APPLOWS_SELF=%~f0\"\n");
    out.push_str(
        "\"%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\" -NoProfile -ExecutionPolicy Bypass -Command \"try { $v=[Environment]::GetCommandLineArgs(); $i=0; while($i -lt $v.Length -and $v[$i] -ine '-Command'){$i++}; $j=$i+2; $a=@(); if($v.Length -gt $j){$a=@($v[$j..($v.Length-1)])}; $u=New-Object System.Text.UTF8Encoding $false; $s=[System.IO.File]::ReadAllText($env:APPLOWS_SELF,$u); $b=[ScriptBlock]::Create($s); & $b @a; exit [int]$global:__ap_ret } catch { [Console]::Error.WriteLine($_); exit 1 } #\" %*\n",
    );
    out.push_str("set \"__ap_rc=%ERRORLEVEL%\"\n");
    out.push_str("endlocal & exit /b %__ap_rc%\n");
    out.push_str(BATCH_DELIM);
    out.push('\n');
    // --- sh ペイロード (macOS) ---
    // sh が PowerShell 部 (`'@` 以降) へ到達しないよう必ず exit で終端する。
    // ペイロード末尾が既にトップレベルの exit なら二重出力しない。
    out.push_str("# ==== Applows sh payload (macOS /bin/sh + zsh) ====\n");
    out.push_str(sh);
    out.push('\n');
    let ends_with_exit = sh
        .lines()
        .last()
        .is_some_and(|l| l == "exit 0" || l.starts_with("exit "));
    if !ends_with_exit {
        out.push_str("exit 0\n");
    }
    out.push_str(PS_HEREDOC_END);
    out.push('\n');
    // --- PowerShell ペイロード (Windows) ---
    out.push_str("# ==== Applows PowerShell payload (Windows 11) ====\n");
    out.push_str(ps);
    out.push('\n');
    out
}

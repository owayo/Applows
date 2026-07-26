//! Applows CLI。
//!
//! 使い方: `applows build <input.aplo> [-o out.bat]`
//!         `applows check <input.aplo>`   (コンパイルの可否だけ検査)
//!         `applows emit <input.aplo> --target sh|powershell|ir`  (中間生成物を表示)

use applows::CompileResult;
use clap::{Parser, Subcommand, ValueEnum};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "applows", version, about = "Compile a shell-like language to a Windows/macOS polyglot script", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// ソースを単一ポリグロット .bat へコンパイルする
    Build {
        /// 入力 .aplo ファイル
        input: PathBuf,
        /// 出力先 (省略時は入力の拡張子を .bat にしたもの)
        #[arg(short = 'o', long)]
        output: Option<PathBuf>,
        /// 実際には書き込まず、生成結果を標準出力に表示する
        #[arg(short = 'n', long)]
        dry_run: bool,
    },
    /// コンパイルの可否だけを検査する (出力しない)
    Check {
        /// 入力 .aplo ファイル
        input: PathBuf,
    },
    /// 中間生成物 (sh / powershell / ir) を表示する
    Emit {
        /// 入力 .aplo ファイル
        input: PathBuf,
        /// 表示するターゲット
        #[arg(long, value_enum)]
        target: EmitTarget,
    },
    /// AI エージェント (Claude Code / Codex) 用の Applows 言語スキルをインストールする
    InstallSkill {
        /// インストール先: claude / codex / all
        #[arg(long, default_value = "claude")]
        target: String,
        /// インストール先ディレクトリを直接指定 (例: プロジェクトの .claude/skills)。指定時は --target を無視
        #[arg(long)]
        dir: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum EmitTarget {
    Sh,
    Powershell,
    Ir,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprint!("{msg}");
            if !msg.ends_with('\n') {
                eprintln!();
            }
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Build {
            input,
            output,
            dry_run,
        } => {
            let result = compile_file(&input)?;
            if dry_run {
                print!("{}", result.output);
                return Ok(());
            }
            let out_path = output.unwrap_or_else(|| default_output(&input));
            std::fs::write(&out_path, result.output.as_bytes())
                .map_err(|e| format!("出力の書き込みに失敗しました {}: {e}", out_path.display()))?;
            set_executable(&out_path)?;
            eprintln!("compiled: {} -> {}", input.display(), out_path.display());
            Ok(())
        }
        Command::Check { input } => {
            compile_file(&input)?;
            eprintln!("ok: {}", input.display());
            Ok(())
        }
        Command::Emit { input, target } => {
            let result = compile_file(&input)?;
            match target {
                EmitTarget::Sh => print!("{}", result.sh_payload),
                EmitTarget::Powershell => print!("{}", result.ps_payload),
                EmitTarget::Ir => println!("{:#?}", result.ir),
            }
            Ok(())
        }
        Command::InstallSkill { target, dir } => {
            applows::skill::install(&target, dir.as_deref())?;
            Ok(())
        }
    }
}

fn compile_file(input: &Path) -> Result<CompileResult, String> {
    let source = std::fs::read_to_string(input)
        .map_err(|e| format!("入力の読み込みに失敗しました {}: {e}", input.display()))?;
    let filename = input.display().to_string();
    applows::compile_rendered(&source, &filename)
}

fn default_output(input: &Path) -> PathBuf {
    input.with_extension("bat")
}

/// macOS/Unix では実行ビットを立て、`./out.bat` で直接起動できるようにする。
#[cfg(unix)]
fn set_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let meta = std::fs::metadata(path).map_err(|e| {
        format!(
            "出力ファイルの権限取得に失敗しました {}: {e}",
            path.display()
        )
    })?;
    let mut perms = meta.permissions();
    perms.set_mode(perms.mode() | 0o100);
    std::fs::set_permissions(path, perms).map_err(|e| {
        format!(
            "出力ファイルへの実行権限付与に失敗しました {}: {e}",
            path.display()
        )
    })
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::set_executable;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn set_executable_preserves_restricted_permissions() {
        let path =
            std::env::temp_dir().join(format!("applows-set-executable-{}", std::process::id()));
        std::fs::write(&path, b"test").expect("テストファイルを書き込める");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("事前の権限を設定できる");

        set_executable(&path).expect("実行権限を付与できる");

        let mode = std::fs::metadata(&path)
            .expect("付与後の権限を取得できる")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
        std::fs::remove_file(path).expect("テストファイルを削除できる");
    }
}

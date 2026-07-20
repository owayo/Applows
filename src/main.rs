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
            set_executable(&out_path);
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
fn set_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        perms.set_mode(perms.mode() | 0o755);
        let _ = std::fs::set_permissions(path, perms);
    }
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) {}

//! AI エージェント向け Applows スキルのインストール処理。
//!
//! `applows` を書くための言語リファレンス一式 (SKILL.md + 完全リファレンス) を、
//! Claude Code / Codex CLI のスキルディレクトリへ書き出す。

use std::path::{Path, PathBuf};

/// スキル本体 (AI 向けの凝縮リファレンス)。
const SKILL_CONTENT: &str = include_str!("../skills/SKILL.md");
/// 同梱する完全リファレンス (全構文・全組み込み・エッジケース)。
const LANGUAGE_REF: &str = include_str!("../docs/language.md");

/// スキル名 (ディレクトリ名)。
const SKILL_NAME: &str = "applows";

/// インストール先の AI エージェント。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Claude Code: `~/.claude/skills/applows/`
    Claude,
    /// Codex CLI: `~/.codex/skills/applows/`
    Codex,
}

impl Target {
    fn parse(s: &str) -> Result<Vec<Target>, String> {
        match s.to_lowercase().as_str() {
            "claude" | "claude-code" => Ok(vec![Target::Claude]),
            "codex" => Ok(vec![Target::Codex]),
            "all" => Ok(vec![Target::Claude, Target::Codex]),
            other => Err(format!(
                "未知のターゲット `{other}`。指定可能: claude / codex / all"
            )),
        }
    }

    /// ホームからの `.../skills` ベースディレクトリ。
    fn skills_base(self, home: &Path) -> PathBuf {
        match self {
            Target::Claude => home.join(".claude").join("skills"),
            Target::Codex => home.join(".codex").join("skills"),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Target::Claude => "Claude Code",
            Target::Codex => "Codex CLI",
        }
    }
}

/// スキルをインストールする。
///
/// - `target`: `claude` / `codex` / `all`
/// - `dir_override`: 指定時はこのディレクトリ直下の `applows/` へ入れる (target は無視)
///
/// 返り値は書き出した SKILL.md のパス一覧。
pub fn install(target: &str, dir_override: Option<&Path>) -> Result<Vec<PathBuf>, String> {
    if let Some(dir) = dir_override {
        let path = write_skill(&dir.join(SKILL_NAME))?;
        eprintln!("Installed applows skill at: {}", path.display());
        return Ok(vec![path]);
    }

    let home = home_dir()?;
    let targets = Target::parse(target)?;
    let mut written = Vec::new();
    for t in targets {
        let skill_dir = t.skills_base(&home).join(SKILL_NAME);
        let path = write_skill(&skill_dir)?;
        eprintln!(
            "Installed applows skill for {} at: {}",
            t.label(),
            path.display()
        );
        written.push(path);
    }
    Ok(written)
}

/// `<skill_dir>/SKILL.md` と `<skill_dir>/reference/language.md` を書き出し、SKILL.md のパスを返す。
fn write_skill(skill_dir: &Path) -> Result<PathBuf, String> {
    let reference_dir = skill_dir.join("reference");
    std::fs::create_dir_all(&reference_dir).map_err(|e| {
        format!(
            "ディレクトリを作成できません {}: {e}",
            reference_dir.display()
        )
    })?;

    let skill_path = skill_dir.join("SKILL.md");
    std::fs::write(&skill_path, SKILL_CONTENT)
        .map_err(|e| format!("書き込みに失敗 {}: {e}", skill_path.display()))?;

    let ref_path = reference_dir.join("language.md");
    std::fs::write(&ref_path, LANGUAGE_REF)
        .map_err(|e| format!("書き込みに失敗 {}: {e}", ref_path.display()))?;

    Ok(skill_path)
}

/// ホームディレクトリを環境変数から解決する (依存クレートを増やさない)。
fn home_dir() -> Result<PathBuf, String> {
    if let Ok(h) = std::env::var("HOME")
        && !h.is_empty()
    {
        return Ok(PathBuf::from(h));
    }
    // Windows フォールバック
    if let Ok(h) = std::env::var("USERPROFILE")
        && !h.is_empty()
    {
        return Ok(PathBuf::from(h));
    }
    Err("ホームディレクトリを判別できません ($HOME / $USERPROFILE が未設定)".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_targets() {
        assert_eq!(Target::parse("claude").unwrap(), vec![Target::Claude]);
        assert_eq!(Target::parse("codex").unwrap(), vec![Target::Codex]);
        assert_eq!(
            Target::parse("all").unwrap(),
            vec![Target::Claude, Target::Codex]
        );
        assert!(Target::parse("nope").is_err());
    }

    #[test]
    fn skills_base_paths() {
        let home = Path::new("/home/u");
        assert_eq!(
            Target::Claude.skills_base(home),
            Path::new("/home/u/.claude/skills")
        );
        assert_eq!(
            Target::Codex.skills_base(home),
            Path::new("/home/u/.codex/skills")
        );
    }

    #[test]
    fn write_skill_creates_skill_and_reference() {
        let tmp = std::env::temp_dir().join(format!("applows_skill_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let skill_dir = tmp.join("applows");
        let skill_path = write_skill(&skill_dir).unwrap();

        assert_eq!(skill_path, skill_dir.join("SKILL.md"));
        assert_eq!(std::fs::read_to_string(&skill_path).unwrap(), SKILL_CONTENT);
        let ref_path = skill_dir.join("reference").join("language.md");
        assert!(ref_path.exists());
        assert_eq!(std::fs::read_to_string(&ref_path).unwrap(), LANGUAGE_REF);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}

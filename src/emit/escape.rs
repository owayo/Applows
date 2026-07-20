//! ターゲット別の文字列エスケープ基本関数。
//!
//! 原則: ユーザ文字列は必ず single quote で包み、生成コードの構文として解釈させない。
//! 補間は「single quote リテラル + 変数参照」の連結で組み立て、double quote 内の
//! エスケープ地獄 (`$` `` ` `` `"` の差異) を回避する (連結の実装は各エミッタ側)。

/// sh の single-quoted 文字列。`'` は `'\''` (閉じ→エスケープした'→開き) で分割する。
pub fn sh_squote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// PowerShell の single-quoted 文字列。`'` は `''` で二重化する。
pub fn ps_squote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sh_basic() {
        assert_eq!(sh_squote("hello"), "'hello'");
        assert_eq!(sh_squote(""), "''");
        assert_eq!(sh_squote("it's"), "'it'\\''s'");
        assert_eq!(sh_squote("a\\b"), "'a\\b'"); // sh の single quote 内では \ はそのまま
        assert_eq!(sh_squote("$x `x` \"x\""), "'$x `x` \"x\"'");
    }

    #[test]
    fn ps_basic() {
        assert_eq!(ps_squote("hello"), "'hello'");
        assert_eq!(ps_squote(""), "''");
        assert_eq!(ps_squote("it's"), "'it''s'");
        assert_eq!(ps_squote("a`b$c"), "'a`b$c'"); // PS single quote 内は ` も $ もそのまま
    }
}

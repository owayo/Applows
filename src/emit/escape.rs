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

/// sh の文字列リテラル式。single quote は改行(LF)をそのまま含められるが、CR は
/// verify の「CR 禁止」に触れるため `"$(printf '\r')"` へ退避して連結する。
pub fn sh_lit(s: &str) -> String {
    if !s.contains('\r') {
        return sh_squote(s);
    }
    s.split('\r')
        .map(sh_squote)
        .collect::<Vec<_>>()
        .join("\"$(printf '\\r')\"")
}

/// PowerShell の文字列リテラル式。single-quoted 文字列は 1 行のみ (LF/CR を生で含めると
/// パースエラー) なので、LF/CR は `[char]10` / `[char]13` へ退避して連結する。
/// 先頭に `''` を置き、`[char]` 同士が整数加算されるのを防いで常に文字列連結にする。
pub fn ps_lit(s: &str) -> String {
    if !s.contains('\n') && !s.contains('\r') {
        return ps_squote(s);
    }
    let mut tokens = vec!["''".to_string()];
    let mut run = String::new();
    for c in s.chars() {
        match c {
            '\n' | '\r' => {
                if !run.is_empty() {
                    tokens.push(ps_squote(&run));
                    run.clear();
                }
                tokens.push(if c == '\n' {
                    "[char]10".to_string()
                } else {
                    "[char]13".to_string()
                });
            }
            _ => run.push(c),
        }
    }
    if !run.is_empty() {
        tokens.push(ps_squote(&run));
    }
    tokens.join(" + ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_chars() {
        // LF は sh の single quote に生で入る (CR のみ退避)
        assert_eq!(sh_lit("a\nb"), "'a\nb'");
        assert_eq!(sh_lit("a\rb"), "'a'\"$(printf '\\r')\"'b'");
        // PS は LF/CR とも [char] へ退避
        assert_eq!(ps_lit("a\nb"), "'' + 'a' + [char]10 + 'b'");
        assert_eq!(ps_lit("a\r\nb"), "'' + 'a' + [char]13 + [char]10 + 'b'");
        assert_eq!(ps_lit("plain"), "'plain'");
    }

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

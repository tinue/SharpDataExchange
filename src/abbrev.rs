//! Dotted keyword-abbreviation expansion (`P.` -> `PRINT`, `GOS.` -> `GOSUB`,
//! `INSTATT.` -> `INSTAT` + `THEN`). Faithful port of Java
//! `preprocess/AbbreviationExpander`, run per line before the scanner.
//!
//! No expansion happens inside string literals, after `REM`, or after `'` (the `REM`
//! shorthand). The PC-1600 registry defines no abbreviations, so this is a no-op there.

use crate::registry::Registry;

#[derive(PartialEq)]
enum State {
    Normal,
    InString,
    InComment,
}

/// Expand every registered `prefix.` form in `line`.
pub fn expand(line: &str, reg: &Registry) -> String {
    if line.is_empty() {
        return String::new();
    }
    let mut result = String::with_capacity(line.len());
    let mut word = String::new();
    let mut state = State::Normal;

    for c in line.chars() {
        match state {
            State::Normal => {
                if c == '"' {
                    result.push_str(&word);
                    word.clear();
                    result.push(c);
                    state = State::InString;
                } else if c.is_ascii_alphabetic() {
                    word.push(c);
                } else if c == '.' && !word.is_empty() {
                    expand_dotted(&word, reg, &mut result, &mut state);
                    word.clear();
                    if is_rem_start(&result) {
                        state = State::InComment;
                    }
                } else {
                    result.push_str(&word);
                    word.clear();
                    if c == '\'' || is_rem_start(&result) {
                        state = State::InComment;
                    }
                    result.push(c);
                }
            }
            State::InString => {
                result.push(c);
                if c == '"' {
                    state = State::Normal;
                }
            }
            State::InComment => result.push(c),
        }
    }
    result.push_str(&word);
    result
}

fn expand_dotted(word: &str, reg: &Registry, result: &mut String, state: &mut State) {
    // 1. Whole accumulated word as an abbreviation (e.g. "GOS." -> GOSUB).
    let full = format!("{word}.");
    if let Some(kw) = reg.lookup(&full) {
        result.push_str(kw.name);
        return;
    }
    // 2. Shortest suffix that is a registered abbreviation; emit unmatched prefix first.
    let n = word.len(); // ASCII letters only -> byte len == char len
    for suffix_len in (1..n).rev() {
        let suffix = format!("{}.", &word[n - suffix_len..]);
        if let Some(kw) = reg.lookup(&suffix) {
            result.push_str(&word[..n - suffix_len]);
            if is_rem_start(result) {
                *state = State::InComment;
                result.push_str(&suffix);
            } else {
                result.push_str(kw.name);
            }
            return;
        }
    }
    // 3. No match: leave the dotted form untouched.
    result.push_str(word);
    result.push('.');
}

/// True when `s` ends with `REM` (case-insensitive) at a statement boundary: preceded
/// by `:`, a digit, or start of line.
fn is_rem_start(s: &str) -> bool {
    let b = s.as_bytes();
    let len = b.len();
    if len < 3 {
        return false;
    }
    if !b[len - 3].eq_ignore_ascii_case(&b'R')
        || !b[len - 2].eq_ignore_ascii_case(&b'E')
        || !b[len - 1].eq_ignore_ascii_case(&b'M')
    {
        return false;
    }
    if len == 3 {
        return true;
    }
    let before = b[len - 4];
    before == b':' || before.is_ascii_digit()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry;

    fn ex(line: &str) -> String {
        expand(line, registry::pc1500())
    }

    #[test]
    fn simple_abbreviations() {
        assert_eq!(ex("10 P. \"HI\""), "10 PRINT \"HI\"");
        assert_eq!(ex("20 GOS. 100"), "20 GOSUB 100");
        assert_eq!(ex("30 F. I=1 TO 10"), "30 FOR I=1 TO 10");
    }

    #[test]
    fn no_expansion_in_string() {
        assert_eq!(ex("10 PRINT \"P.\""), "10 PRINT \"P.\"");
    }

    #[test]
    fn rem_comment_mode_matches_java_is_rem_start() {
        // Java's isRemStart only fires when REM is preceded by a digit or ':' (no space),
        // so `10 REM P.` (space after the line number) still expands the abbreviation...
        assert_eq!(ex("10 REM P. here"), "10 REM PRINT here");
        // ...but `10:REM P.` (colon right before REM) enters comment mode and is verbatim.
        assert_eq!(ex("10:REM P. here"), "10:REM P. here");
    }

    #[test]
    fn glued_suffix_expansion() {
        // "T." is THEN; "IF A=1T. 20" -> "IF A=1THEN 20"
        assert_eq!(ex("10 IF A=1T. 20"), "10 IF A=1THEN 20");
    }

    #[test]
    fn unknown_dotted_form_untouched() {
        assert_eq!(ex("10 ZZ. 1"), "10 ZZ. 1");
    }

    #[test]
    fn pc1600_is_noop() {
        assert_eq!(expand("10 P. 1", registry::pc1600()), "10 P. 1");
    }

    #[test]
    fn quote_comment_is_verbatim_like_rem() {
        assert_eq!(ex("10 'CALL P. HERE"), "10 'CALL P. HERE");
        assert_eq!(ex("10 X = 1 : 'CALL P. HERE"), "10 X = 1 : 'CALL P. HERE");
    }
}

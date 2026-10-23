//! A small dotenv reader for build scripts (`apps/note4c/build.rs`). No allocation: values
//! are slices of the file text.
//!
//! Rules (close to python-dotenv, without escapes, variable expansion or multi-line values):
//!
//! - One `KEY=value` per line. `export KEY=value` is accepted.
//! - Blank lines, lines starting with `#` and lines without `=` are skipped.
//! - A UTF-8 BOM at the start and CRLF line endings are ignored.
//! - Whitespace around the key and the value is trimmed.
//! - A value in matching quotes (`'…'` or `"…"`) is taken literally between them; one pair
//!   is stripped and anything after the closing quote is ignored.
//! - In an unquoted value, whitespace followed by `#` starts a comment. A `#` inside a
//!   word or at the very start of the value is kept (`pass#word`, `#word`).
//! - If a key appears more than once, the last one wins.

/// All `(key, value)` entries of a dotenv file, in file order.
pub fn entries(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.strip_prefix('\u{feff}')
        .unwrap_or(text)
        .lines()
        .filter_map(parse_line)
}

/// The value of `key` in a dotenv file (the last one if it appears more than once).
pub fn get<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    entries(text)
        .filter(|(k, _)| *k == key)
        .last()
        .map(|(_, v)| v)
}

/// Parse one line. `None` for blank lines, comments and lines without `=`.
pub fn parse_line(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let line = line
        .strip_prefix("export ")
        .map(str::trim_start)
        .unwrap_or(line);
    let (key, raw) = line.split_once('=')?;
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    Some((key, value(raw)))
}

fn value(raw: &str) -> &str {
    let v = raw.trim_start();
    for quote in ['"', '\''] {
        if let Some(rest) = v.strip_prefix(quote)
            && let Some(end) = rest.find(quote)
        {
            return &rest[..end];
        }
    }
    let mut prev_space = false; // `KEY=#x` is `#x`, `KEY= # comment` is empty
    for (i, c) in raw.char_indices() {
        if c == '#' && prev_space {
            return raw[..i].trim();
        }
        prev_space = c.is_whitespace();
    }
    raw.trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_and_export() {
        assert_eq!(parse_line("A=b"), Some(("A", "b")));
        assert_eq!(parse_line("  A = b  "), Some(("A", "b")));
        assert_eq!(parse_line("export A=b"), Some(("A", "b")));
        assert_eq!(parse_line("export   A=b"), Some(("A", "b")));
        assert_eq!(parse_line("A="), Some(("A", "")));
        assert_eq!(parse_line("A=b=c"), Some(("A", "b=c")));
    }

    #[test]
    fn skips_comments_blank_and_junk() {
        assert_eq!(parse_line(""), None);
        assert_eq!(parse_line("   "), None);
        assert_eq!(parse_line("# A=b"), None);
        assert_eq!(parse_line("no equals sign"), None);
        assert_eq!(parse_line("=value"), None);
    }

    #[test]
    fn quotes_strip_one_pair() {
        assert_eq!(parse_line("A=\"b c\""), Some(("A", "b c")));
        assert_eq!(parse_line("A='b c'"), Some(("A", "b c")));
        assert_eq!(parse_line("A=\"'b'\""), Some(("A", "'b'")));
        assert_eq!(parse_line("A=''b''"), Some(("A", "")));
        assert_eq!(
            parse_line("A=\"b # not a comment\" # comment"),
            Some(("A", "b # not a comment"))
        );
        // Unmatched quote: literal.
        assert_eq!(parse_line("A=\"b"), Some(("A", "\"b")));
        assert_eq!(parse_line("A=b\""), Some(("A", "b\"")));
    }

    #[test]
    fn inline_comments() {
        assert_eq!(parse_line("A=b # comment"), Some(("A", "b")));
        assert_eq!(parse_line("A=b\t# comment"), Some(("A", "b")));
        assert_eq!(parse_line("A=pass#word"), Some(("A", "pass#word")));
        assert_eq!(parse_line("A=#word"), Some(("A", "#word")));
        assert_eq!(parse_line("A= # comment"), Some(("A", "")));
    }

    #[test]
    fn bom_crlf_and_last_wins() {
        let text = "\u{feff}A=1\r\n# c\r\nB='x'\r\nA=2\r\n";
        assert_eq!(get(text, "A"), Some("2"));
        assert_eq!(get(text, "B"), Some("x"));
        assert_eq!(get(text, "C"), None);
        assert_eq!(entries(text).count(), 3);
    }
}

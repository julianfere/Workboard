//! Semántica de las funciones de texto de Python que usaba la v1. El formato de los README depende de
//! ellas en los bordes (qué es un fin de línea, qué es espacio), así que se replican exactas.

/// `str.isspace()` de Python: el White_Space de Unicode más los separadores \x1c-\x1f.
pub fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

pub fn strip(s: &str) -> &str {
    s.trim_matches(is_space)
}

pub fn lstrip(s: &str) -> &str {
    s.trim_start_matches(is_space)
}

/// `str.split()` sin argumentos: separa por espacios y descarta vacíos.
pub fn split_ws(s: &str) -> impl Iterator<Item = &str> {
    s.split(is_space).filter(|p| !p.is_empty())
}

/// Fin de línea para `str.splitlines()`.
fn is_line_break(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r'
            | '\u{0b}'
            | '\u{0c}'
            | '\u{1c}'
            | '\u{1d}'
            | '\u{1e}'
            | '\u{85}'
            | '\u{2028}'
            | '\u{2029}'
    )
}

/// `str.splitlines(keepends)`. `\r\n` cuenta como un solo fin de línea.
pub fn splitlines(s: &str, keepends: bool) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if !is_line_break(c) {
            continue;
        }
        let mut end = i + c.len_utf8();
        if c == '\r'
            && let Some(&(_, '\n')) = chars.peek()
        {
            chars.next();
            end += 1;
        }
        out.push(&s[start..if keepends { end } else { i }]);
        start = end;
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

/// `repr()` de un string de Python (comillas simples salvo que tenga `'` y no `"`).
pub fn repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out.push_str(&format!("\\x{:02x}", c as u32)),
            c if (0x80..0xa0).contains(&(c as u32)) => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitlines_matches_python() {
        assert_eq!(splitlines("a\nb\r\nc\rd", false), ["a", "b", "c", "d"]);
        assert_eq!(splitlines("a\nb\r\n", true), ["a\n", "b\r\n"]);
        assert_eq!(splitlines("", true), Vec::<&str>::new());
        assert_eq!(splitlines("\n", false), [""]);
        assert_eq!(splitlines("a\u{2028}b\x0cc", false), ["a", "b", "c"]);
    }

    #[test]
    fn repr_matches_python() {
        assert_eq!(repr("x"), "'x'");
        assert_eq!(repr("it's"), "\"it's\"");
        assert_eq!(repr("a'\"b"), "'a\\'\"b'");
        assert_eq!(repr("a\nb\\"), "'a\\nb\\\\'");
        assert_eq!(repr("ñ"), "'ñ'");
    }

    #[test]
    fn strip_matches_python() {
        assert_eq!(strip("\u{1c} a \u{3000}"), "a");
        assert_eq!(strip("\u{feff}a"), "\u{feff}a", "BOM is not whitespace in Python");
        assert_eq!(split_ws(" a\t b\n").collect::<Vec<_>>(), ["a", "b"]);
    }
}

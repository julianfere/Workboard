//! HTML de un tracker (descripción, criterios de aceptación) a Markdown simple, como `core.html_to_text`
//! de la v1: saltos de línea, listas y títulos; el resto de las etiquetas se quita.

use std::sync::OnceLock;

use regex::Regex;

use crate::entities::{HTML5, INVALID_CHARREFS, INVALID_CODEPOINTS};

struct Patterns {
    script: Regex,
    style: Regex,
    br: Regex,
    li: Regex,
    block_end: Regex,
    heading: Regex,
    tag: Regex,
    list_gap: Regex,
    blank_lines: Regex,
    entity: Regex,
}

fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| {
        let re = |s: &str| Regex::new(s).expect("valid regex");
        Patterns {
            script: re(r"(?is)<script.*?</script>"),
            style: re(r"(?is)<style.*?</style>"),
            br: re(r"(?i)<br\s*/?>"),
            li: re(r"(?i)<li[^>]*>"),
            block_end: re(r"(?i)</(p|div|li|ul|ol|h\d|tr|table)>"),
            heading: re(r"(?i)<h\d[^>]*>"),
            tag: re(r"<[^>]+>"),
            list_gap: re(r"\n\s*\n(- )"),
            blank_lines: re(r"\n{3,}"),
            // El mismo patrón que `html.unescape` de Python.
            entity: re(r"&(#[0-9]+;?|#[xX][0-9a-fA-F]+;?|[^\t\n\x0c <&#;]{1,32};?)"),
        }
    })
}

fn html5(name: &str) -> Option<&'static str> {
    HTML5.binary_search_by(|(k, _)| (*k).cmp(name)).ok().map(|i| HTML5[i].1)
}

/// Una referencia numérica, como `html._replace_charref` de Python.
fn charref(digits: &str, radix: u32) -> String {
    let n = u32::from_str_radix(digits.trim_end_matches(';'), radix).unwrap_or(u32::MAX);
    if let Some((_, v)) = INVALID_CHARREFS.iter().find(|(k, _)| *k == n) {
        return (*v).to_string();
    }
    if (0xD800..=0xDFFF).contains(&n) || n > 0x10FFFF {
        return '\u{FFFD}'.to_string();
    }
    if INVALID_CODEPOINTS.contains(&n) {
        return String::new();
    }
    char::from_u32(n).map(String::from).unwrap_or_default()
}

/// Entidades HTML como `html.unescape` de Python: todas las de HTML5 (también las que se aceptan sin `;`,
/// tomando el prefijo más largo: `&copy2` → `©2`) y las numéricas, con los reemplazos del navegador. Las
/// desconocidas quedan como están.
pub fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    patterns()
        .entity
        .replace_all(s, |c: &regex::Captures| {
            let name = &c[1];
            if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                return charref(hex, 16);
            }
            if let Some(dec) = name.strip_prefix('#') {
                return charref(dec, 10);
            }
            if let Some(v) = html5(name) {
                return v.to_string();
            }
            let cuts: Vec<usize> = name.char_indices().map(|(i, _)| i).skip(2).collect();
            for &cut in cuts.iter().rev() {
                if let Some(v) = html5(&name[..cut]) {
                    return format!("{v}{}", &name[cut..]);
                }
            }
            format!("&{name}")
        })
        .into_owned()
}

pub fn html_to_text(src: &str) -> String {
    if src.is_empty() {
        return String::new();
    }
    let p = patterns();
    let s = p.script.replace_all(src, "");
    let s = p.style.replace_all(&s, "");
    let s = p.br.replace_all(&s, "\n");
    let s = p.li.replace_all(&s, "\n- ");
    let s = p.block_end.replace_all(&s, "\n");
    let s = p.heading.replace_all(&s, "\n### ");
    let s = p.tag.replace_all(&s, "");
    let s = unescape(&s).replace('\u{a0}', " ");
    let joined = s.lines().map(str::trim_end).collect::<Vec<_>>().join("\n");
    let s = p.list_gap.replace_all(&joined, "\n$1");
    p.blank_lines.replace_all(&s, "\n\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_azure_html() {
        let html = "<div>Hola&nbsp;mundo</div><div><br></div><ul><li>uno</li><li>dos &amp; tres</li></ul>\
                    <h2>Título</h2><p>a&lt;b &#225; &#xe9; &unknown;</p><script>x()</script>";
        assert_eq!(html_to_text(html), "Hola mundo\n- uno\n- dos & tres\n\n### Título\na<b á é &unknown;");
        assert_eq!(html_to_text(""), "");
        assert_eq!(html_to_text("texto plano"), "texto plano");
    }

    #[test]
    fn unescapes_like_python() {
        // Resultados de `html.unescape` de Python 3.13.
        for (src, want) in [
            ("&amp;&lt;&gt;", "&<>"),
            ("&copy", "\u{a9}"),
            ("&notit;", "\u{ac}it;"),
            ("&notin;", "\u{2209}"),
            ("&amplt;", "&lt;"),
            ("&#128;", "\u{20ac}"),
            ("&#0;", "\u{fffd}"),
            ("&#x110000;", "\u{fffd}"),
            ("&#1;", ""),
            ("&unknown;", "&unknown;"),
            ("&Ccedil;&ccedil;&euro;&trade;&hearts;", "\u{c7}\u{e7}\u{20ac}\u{2122}\u{2665}"),
            ("&#225&#xe9", "\u{e1}\u{e9}"),
            ("a & b", "a & b"),
            ("&#99999999999;", "\u{fffd}"),
            ("&ensp;&thinsp;&zwj;", "\u{2002}\u{2009}\u{200d}"),
        ] {
            assert_eq!(unescape(src), want, "{src}");
        }
    }
}

//! HTML de un tracker (descripción, criterios de aceptación) a Markdown simple, como `core.html_to_text`
//! de la v1: saltos de línea, listas y títulos; el resto de las etiquetas se quita.

use std::sync::OnceLock;

use regex::Regex;

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
            entity: re(r"&(#[0-9]{1,7}|#[xX][0-9a-fA-F]{1,6}|[A-Za-z]{2,8});"),
        }
    })
}

const NAMED: &[(&str, &str)] = &[
    ("amp", "&"),
    ("lt", "<"),
    ("gt", ">"),
    ("quot", "\""),
    ("apos", "'"),
    ("nbsp", " "),
    ("ndash", "–"),
    ("mdash", "—"),
    ("hellip", "…"),
    ("laquo", "«"),
    ("raquo", "»"),
    ("ldquo", "“"),
    ("rdquo", "”"),
    ("lsquo", "‘"),
    ("rsquo", "’"),
    ("bull", "•"),
    ("middot", "·"),
    ("copy", "©"),
    ("reg", "®"),
    ("deg", "°"),
    ("iexcl", "¡"),
    ("iquest", "¿"),
    ("aacute", "á"),
    ("eacute", "é"),
    ("iacute", "í"),
    ("oacute", "ó"),
    ("uacute", "ú"),
    ("ntilde", "ñ"),
    ("uuml", "ü"),
    ("Aacute", "Á"),
    ("Eacute", "É"),
    ("Iacute", "Í"),
    ("Oacute", "Ó"),
    ("Uacute", "Ú"),
    ("Ntilde", "Ñ"),
    ("Uuml", "Ü"),
];

/// Entidades HTML (las numéricas y las con nombre más comunes); las desconocidas quedan como están.
pub fn unescape(s: &str) -> String {
    patterns()
        .entity
        .replace_all(s, |c: &regex::Captures| {
            let name = &c[1];
            let ch = if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                u32::from_str_radix(hex, 16).ok().and_then(char::from_u32).map(String::from)
            } else if let Some(dec) = name.strip_prefix('#') {
                dec.parse::<u32>().ok().and_then(char::from_u32).map(String::from)
            } else {
                NAMED.iter().find(|(n, _)| *n == name).map(|(_, v)| (*v).to_string())
            };
            ch.unwrap_or_else(|| c[0].to_string())
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
}

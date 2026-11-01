//! Lenient attribute-value unescaping for hand-made and legacy OPML.
//!
//! Strict XML knows five named entities; old exporters routinely write HTML
//! ones (`&nbsp;`, `&eacute;`) and bare ampersands (`Tom & Jerry`) into
//! attribute values, and a strict unescape rejects the whole document over
//! one title. [`unescape_lenient`] never fails: the XML entities, numeric
//! character references and the HTML 4 named entities (the set legacy
//! exporters actually emit) are resolved, and anything else is **kept as the
//! raw text** — a title that reads `R&D` or `&bogus;` is still a title.

/// HTML 4 named entities beyond the five XML ones: Latin-1 (`nbsp`…`yuml`)
/// plus the general punctuation and symbols legacy exporters emit. Sorted by
/// name (byte order) for binary search.
const HTML_ENTITIES: &[(&str, char)] = &[
    ("AElig", 'Æ'),
    ("Aacute", 'Á'),
    ("Acirc", 'Â'),
    ("Agrave", 'À'),
    ("Aring", 'Å'),
    ("Atilde", 'Ã'),
    ("Auml", 'Ä'),
    ("Ccedil", 'Ç'),
    ("Dagger", '‡'),
    ("ETH", 'Ð'),
    ("Eacute", 'É'),
    ("Ecirc", 'Ê'),
    ("Egrave", 'È'),
    ("Euml", 'Ë'),
    ("Iacute", 'Í'),
    ("Icirc", 'Î'),
    ("Igrave", 'Ì'),
    ("Iuml", 'Ï'),
    ("Ntilde", 'Ñ'),
    ("OElig", 'Œ'),
    ("Oacute", 'Ó'),
    ("Ocirc", 'Ô'),
    ("Ograve", 'Ò'),
    ("Oslash", 'Ø'),
    ("Otilde", 'Õ'),
    ("Ouml", 'Ö'),
    ("Prime", '″'),
    ("Scaron", 'Š'),
    ("THORN", 'Þ'),
    ("Uacute", 'Ú'),
    ("Ucirc", 'Û'),
    ("Ugrave", 'Ù'),
    ("Uuml", 'Ü'),
    ("Yacute", 'Ý'),
    ("Yuml", 'Ÿ'),
    ("aacute", 'á'),
    ("acirc", 'â'),
    ("acute", '´'),
    ("aelig", 'æ'),
    ("agrave", 'à'),
    ("aring", 'å'),
    ("atilde", 'ã'),
    ("auml", 'ä'),
    ("bdquo", '„'),
    ("brvbar", '¦'),
    ("bull", '•'),
    ("ccedil", 'ç'),
    ("cedil", '¸'),
    ("cent", '¢'),
    ("circ", 'ˆ'),
    ("copy", '©'),
    ("curren", '¤'),
    ("dagger", '†'),
    ("deg", '°'),
    ("divide", '÷'),
    ("eacute", 'é'),
    ("ecirc", 'ê'),
    ("egrave", 'è'),
    ("emsp", '\u{2003}'),
    ("ensp", '\u{2002}'),
    ("eth", 'ð'),
    ("euml", 'ë'),
    ("euro", '€'),
    ("fnof", 'ƒ'),
    ("frac12", '½'),
    ("frac14", '¼'),
    ("frac34", '¾'),
    ("hellip", '…'),
    ("iacute", 'í'),
    ("icirc", 'î'),
    ("iexcl", '¡'),
    ("igrave", 'ì'),
    ("iquest", '¿'),
    ("iuml", 'ï'),
    ("laquo", '«'),
    ("ldquo", '“'),
    ("lrm", '\u{200E}'),
    ("lsaquo", '‹'),
    ("lsquo", '‘'),
    ("macr", '¯'),
    ("mdash", '—'),
    ("micro", 'µ'),
    ("middot", '·'),
    ("nbsp", '\u{A0}'),
    ("ndash", '–'),
    ("not", '¬'),
    ("ntilde", 'ñ'),
    ("oacute", 'ó'),
    ("ocirc", 'ô'),
    ("oelig", 'œ'),
    ("ograve", 'ò'),
    ("ordf", 'ª'),
    ("ordm", 'º'),
    ("oslash", 'ø'),
    ("otilde", 'õ'),
    ("ouml", 'ö'),
    ("para", '¶'),
    ("permil", '‰'),
    ("plusmn", '±'),
    ("pound", '£'),
    ("prime", '′'),
    ("raquo", '»'),
    ("rdquo", '”'),
    ("reg", '®'),
    ("rlm", '\u{200F}'),
    ("rsaquo", '›'),
    ("rsquo", '’'),
    ("sbquo", '‚'),
    ("scaron", 'š'),
    ("sect", '§'),
    ("shy", '\u{AD}'),
    ("sup1", '¹'),
    ("sup2", '²'),
    ("sup3", '³'),
    ("szlig", 'ß'),
    ("thinsp", '\u{2009}'),
    ("thorn", 'þ'),
    ("tilde", '˜'),
    ("times", '×'),
    ("trade", '™'),
    ("uacute", 'ú'),
    ("ucirc", 'û'),
    ("ugrave", 'ù'),
    ("uml", '¨'),
    ("uuml", 'ü'),
    ("yacute", 'ý'),
    ("yen", '¥'),
    ("yuml", 'ÿ'),
    ("zwj", '\u{200D}'),
    ("zwnj", '\u{200C}'),
];

/// The longest reference body (`#x10FFFF`, `frac12`, …) worth scanning for
/// a closing `;` — anything longer is a bare `&` followed by prose.
const MAX_REFERENCE_LEN: usize = 10;

/// Resolves one reference body (between `&` and `;`), or `None` if it is not
/// one this module knows — the caller then keeps the raw text.
fn resolve(name: &str) -> Option<char> {
    if let Some(number) = name.strip_prefix('#') {
        let code = match number.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => number.parse::<u32>().ok()?,
        };
        // NUL is not a legal XML character, even by reference.
        return char::from_u32(code).filter(|c| *c != '\0');
    }
    let xml = match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => None,
    };
    xml.or_else(|| {
        HTML_ENTITIES
            .binary_search_by(|(entity, _)| entity.cmp(&name))
            .ok()
            .and_then(|index| HTML_ENTITIES.get(index))
            .map(|(_, c)| *c)
    })
}

/// Unescapes an attribute value without ever failing: known references are
/// resolved, unknown ones and bare `&` are kept verbatim, and literal tabs
/// and line breaks become spaces (XML attribute-value normalization).
pub(super) fn unescape_lenient(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(amp) = rest.find('&') {
        push_normalized(&mut out, &rest[..amp]);
        let after = &rest[amp + 1..];
        let resolved = after
            .char_indices()
            .take(MAX_REFERENCE_LEN + 1)
            .find(|(_, c)| *c == ';')
            .and_then(|(end, _)| resolve(&after[..end]).map(|c| (c, end)));
        if let Some((c, end)) = resolved {
            out.push(c);
            rest = &after[end + 1..];
        } else {
            out.push('&');
            rest = after;
        }
    }
    push_normalized(&mut out, rest);
    out
}

/// Appends `text`, mapping literal `\t`, `\n` and `\r` to a space.
fn push_normalized(out: &mut String, text: &str) {
    out.extend(text.chars().map(|c| {
        if matches!(c, '\t' | '\n' | '\r') {
            ' '
        } else {
            c
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entity_table_is_sorted_for_binary_search() {
        assert!(
            HTML_ENTITIES.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "HTML_ENTITIES must stay sorted by name"
        );
    }

    #[test]
    fn xml_numeric_and_html_references_resolve() {
        assert_eq!(unescape_lenient("a &amp; b &lt;&gt;"), "a & b <>");
        assert_eq!(unescape_lenient("&quot;q&quot; &apos;s&apos;"), "\"q\" 's'");
        assert_eq!(unescape_lenient("caf&eacute;&nbsp;bar"), "café\u{A0}bar");
        assert_eq!(unescape_lenient("&#233;&#xE9;&#XE9;"), "ééé");
        assert_eq!(unescape_lenient("&mdash;&hellip;&euro;"), "—…€");
    }

    #[test]
    fn unknown_or_broken_references_keep_the_raw_text() {
        assert_eq!(unescape_lenient("Tom & Jerry"), "Tom & Jerry");
        assert_eq!(unescape_lenient("R&D"), "R&D");
        assert_eq!(unescape_lenient("&bogus; &"), "&bogus; &");
        assert_eq!(
            unescape_lenient("&#xZZ; &#0; &#99999999;"),
            "&#xZZ; &#0; &#99999999;"
        );
        // A `;` far beyond the reference window is prose, not a reference.
        assert_eq!(
            unescape_lenient("a & a long sentence; then more"),
            "a & a long sentence; then more"
        );
        assert_eq!(unescape_lenient("trailing &"), "trailing &");
    }

    #[test]
    fn literal_whitespace_is_normalized_but_references_are_not() {
        assert_eq!(unescape_lenient("a\tb\nc\rd"), "a b c d");
        assert_eq!(unescape_lenient("a&#10;b"), "a\nb");
    }
}

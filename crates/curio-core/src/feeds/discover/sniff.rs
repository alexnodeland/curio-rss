//! The structural feed sniff: do these bytes look like a feed rather than
//! an HTML page? Content-Type is not exposed by the policed response, so
//! discovery inspects the bytes — the cheap gate in front of
//! [`crate::feeds::parse_feed`].

/// How much of the body the feed sniff looks at. Feeds declare their root
/// element (or JSON Feed `version`) early; scanning the whole body would
/// misfire on HTML that quotes a feed snippet far down the page.
pub(super) const SNIFF_WINDOW: usize = 2048;

/// A quick structural sniff: does the body look like a feed?
///
/// XML: past any BOM, whitespace, XML prolog, processing instructions,
/// comments and `<!DOCTYPE …>`, the *root element* must be `rss`, `feed` or
/// `rdf:RDF` (any namespace prefix, ASCII case-insensitive) — a `<feed`
/// substring elsewhere in an HTML page does not count. JSON: the body is an
/// object declaring a `jsonfeed.org` `version`.
pub(super) fn looks_like_feed(text: &str) -> bool {
    let head = sniff_head(text);
    if head.starts_with('{') {
        return declares_json_feed_version(head);
    }
    root_element_name(head).is_some_and(|name| {
        // `rsplit` always yields at least one piece; the fallback is inert.
        let local = name.rsplit(':').next().unwrap_or(name);
        ["rss", "feed", "rdf"]
            .iter()
            .any(|root| local.eq_ignore_ascii_case(root))
    })
}

/// A quick structural sniff: is the body an HTML page? Past any BOM,
/// whitespace, prolog, comments and `<!DOCTYPE html>`, the root element is
/// `html` (or, for markup that omits it, `head`/`body`), ASCII
/// case-insensitive.
pub(super) fn looks_like_html(text: &str) -> bool {
    root_element_name(sniff_head(text)).is_some_and(|name| {
        ["html", "head", "body"]
            .iter()
            .any(|root| name.eq_ignore_ascii_case(root))
    })
}

/// The first [`SNIFF_WINDOW`] bytes of `text`, past a BOM and whitespace.
fn sniff_head(text: &str) -> &str {
    // Cut on a char boundary: `text` is lossily decoded, so byte
    // `SNIFF_WINDOW` can fall inside a multi-byte char (CJK, emoji, U+FFFD).
    // (A hand-rolled `floor_char_boundary`: that is newer than the MSRV.)
    let mut cut = SNIFF_WINDOW.min(text.len());
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text[..cut].trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
}

/// The name of the first element in `xml`, skipping whitespace, processing
/// instructions (incl. the `<?xml …?>` prolog), comments and a doctype
/// (with or without an internal subset). `None` when the window ends before
/// a root element starts, or the head is not markup at all.
fn root_element_name(xml: &str) -> Option<&str> {
    let mut rest = xml;
    loop {
        rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix("<?") {
            rest = &after[after.find("?>")? + "?>".len()..];
        } else if let Some(after) = rest.strip_prefix("<!--") {
            rest = &after[after.find("-->")? + "-->".len()..];
        } else if let Some(after) = rest.strip_prefix("<!") {
            rest = skip_declaration(after)?;
        } else {
            let after = rest.strip_prefix('<')?;
            let end = after
                .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
                .unwrap_or(after.len());
            let name = &after[..end];
            return (!name.is_empty()).then_some(name);
        }
    }
}

/// Skips the rest of a `<!…>` declaration (`decl` starts just past the
/// `<!`), honouring a bracketed internal subset (`<!DOCTYPE rss [ … ]>`).
fn skip_declaration(decl: &str) -> Option<&str> {
    let close = decl.find('>')?;
    let close = match decl.find('[') {
        Some(open) if open < close => {
            let subset_end = open + decl[open..].find(']')?;
            subset_end + decl[subset_end..].find('>')?
        }
        _ => close,
    };
    Some(&decl[close + 1..])
}

/// Whether a JSON object's head carries `"version": "http(s)://jsonfeed.org…"`
/// (any whitespace around the colon).
fn declares_json_feed_version(json: &str) -> bool {
    json.match_indices("\"version\"").any(|(at, key)| {
        json[at + key.len()..]
            .trim_start()
            .strip_prefix(':')
            .map(str::trim_start)
            .is_some_and(|value| {
                value.starts_with("\"https://jsonfeed.org")
                    || value.starts_with("\"http://jsonfeed.org")
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_roots_are_recognised_past_prolog_comments_and_doctype() {
        let feeds = [
            r#"<rss version="2.0"><channel/></rss>"#,
            "\u{feff}  \n<?xml version=\"1.0\"?>\n<RSS version=\"0.91\"/>",
            r#"<?xml version="1.0"?><?xml-stylesheet href="s.xsl"?><!-- hi --><feed xmlns="http://www.w3.org/2005/Atom"/>"#,
            r#"<?xml version="1.0"?><!DOCTYPE rss PUBLIC "-//Netscape//DTD RSS 0.91//EN" "x.dtd"><rss/>"#,
            r#"<!DOCTYPE rss [ <!ENTITY nbsp "&#160;"> ]><rss version="2.0"/>"#,
            r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"></rdf:RDF>"#,
            r#"<atom:feed xmlns:atom="http://www.w3.org/2005/Atom"></atom:feed>"#,
            "<feed>",
            r#"{"version":"https://jsonfeed.org/version/1.1","title":"x"}"#,
            "{\n  \"title\": \"x\",\n  \"version\" :\t\"https://jsonfeed.org/version/1\"\n}",
        ];
        for feed in feeds {
            assert!(looks_like_feed(feed), "expected a feed: {feed:?}");
        }
    }

    #[test]
    fn html_mentioning_a_feed_tag_is_not_a_feed() {
        // The old substring sniff misfired on any `<feed`/`<rss` in the head.
        let pages = [
            "<html><head><title>How to write a <feed> element</title></head></html>",
            "<!DOCTYPE html><!-- <rss> --><html><body><rss></rss></body></html>",
            r#"<?xml version="1.0"?><html xmlns="http://www.w3.org/1999/xhtml"><feed/></html>"#,
            "<feedback>not a feed</feedback>",
            r#"{"title":"not a feed","note":"\"version\":\"https://jsonfeed.org\" is quoted"}"#,
            r#"{"version":"1.0"}"#,
            "<!-- unterminated comment <rss>",
            "<?xml version=\"1.0\"",
            "<!DOCTYPE rss [ <!ENTITY x \"y\">",
            "plain text <rss>",
            "",
            "<",
        ];
        for page in pages {
            assert!(!looks_like_feed(page), "expected not a feed: {page:?}");
        }
    }

    #[test]
    fn html_pages_are_recognised_and_feeds_are_not() {
        let pages = [
            "<!DOCTYPE html>\n<html lang=\"en\"><head></head></html>",
            "\u{feff}\n  <!-- built by hand --><HTML><body>hi</body></HTML>",
            r#"<?xml version="1.0"?><html xmlns="http://www.w3.org/1999/xhtml"/>"#,
            "<head><title>no html tag</title></head>",
            "<body>just a body</body>",
        ];
        for page in pages {
            assert!(looks_like_html(page), "expected a web page: {page:?}");
        }
        let not_pages = [
            r#"<rss version="2.0"><channel/></rss>"#,
            "<feed>",
            "<htmlish/>",
            r#"{"version":"https://jsonfeed.org/version/1.1"}"#,
            "plain text <html>",
            "",
        ];
        for body in not_pages {
            assert!(!looks_like_html(body), "expected not a web page: {body:?}");
        }
    }

    proptest::proptest! {
        #[test]
        fn the_sniff_never_panics_on_markup_shaped_text(
            body in "[ \u{feff}]{0,3}(<\\?[^>]{0,40}\\??>?|<!--.{0,40}(-->)?|<!DOCTYPE[^>]{0,30}(\\[.{0,30}\\]?)?>?|<[a-zA-Z:日本🎌]{0,10}|\\{.{0,40}|[日本語🎌👍🏽a]{0,700}){0,6}",
        ) {
            let _ = looks_like_feed(&body);
            let _ = looks_like_html(&body);
        }
    }
}

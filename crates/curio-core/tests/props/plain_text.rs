//! `content::plain_text`: escaped text comes back as itself (whitespace
//! normalized), and any output is single-spaced with no edge whitespace.

use curio_core::content::plain_text;
use proptest::prelude::*;

use crate::generators::{escape, hostile_html};

fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

proptest! {
    #[test]
    fn escaped_text_round_trips(text in "\\PC{0,400}") {
        // The HTML parser maps NUL and CR the way the spec says to; the
        // property is about text, so leave those to the next test.
        let text: String = text.chars().filter(|ch| !matches!(ch, '\0' | '\r')).collect();
        let html = format!("<p>{}</p>", escape(&text));
        prop_assert_eq!(plain_text(&html), normalize(&text));
    }

    #[test]
    fn output_is_whitespace_normalized(html in hostile_html()) {
        let text = plain_text(&html);
        prop_assert_eq!(text.trim(), text.as_str());
        prop_assert!(!text.contains("  "), "double space in {text:?}");
        prop_assert!(!text.contains(['\n', '\t', '\r']), "raw whitespace in {text:?}");
    }

    #[test]
    fn block_boundaries_split_words(a in "[a-z]{1,12}", b in "[a-z]{1,12}") {
        prop_assert_eq!(plain_text(&format!("<p>{a}</p><p>{b}</p>")), format!("{a} {b}"));
        prop_assert_eq!(plain_text(&format!("<b>{a}</b>{b}")), format!("{a}{b}"));
    }
}

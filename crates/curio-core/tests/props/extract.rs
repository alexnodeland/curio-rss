//! `content::extract_main_content`: picking the main content out of
//! sanitized HTML keeps it inert and only ever *removes* text — what it
//! returns is an in-order subsequence of what it was given.

use curio_core::content::{extract_main_content, plain_text, sanitize};
use proptest::prelude::*;

use crate::generators::hostile_html;
use crate::inert;

/// Whether `needle`'s characters appear in `haystack` in order.
fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut rest = haystack.chars();
    needle
        .chars()
        .all(|ch| rest.any(|candidate| candidate == ch))
}

fn squeeze(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_whitespace()).collect()
}

proptest! {
    #[test]
    fn extraction_stays_inert_and_invents_no_text(html in hostile_html()) {
        let clean = sanitize(&html, Some("https://example.com/posts/current"));
        let main = extract_main_content(&clean);
        inert::audit_html(&main).map_err(TestCaseError::fail)?;

        let kept = squeeze(&plain_text(&main));
        let source = squeeze(&plain_text(&clean));
        prop_assert!(
            is_subsequence(&kept, &source),
            "extraction produced text that was not in its input:\nin:  {source:?}\nout: {kept:?}"
        );
    }

    #[test]
    fn extraction_never_panics_on_arbitrary_text(text in "\\PC{0,2000}") {
        let _ = extract_main_content(&text);
    }
}

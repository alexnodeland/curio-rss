//! The export path, `content::process` → `content::to_markdown`: it
//! always converts, and the markdown that lands in the vault is inert.

use curio_core::content::{process, to_markdown};
use proptest::prelude::*;

use crate::generators::{exportable_html, hostile_html};
use crate::inert;

proptest! {
    #[test]
    fn exported_markdown_is_inert(html in exportable_html()) {
        let processed = process(&html, Some("https://example.com/posts/current"));
        inert::audit_html(&processed.html).map_err(TestCaseError::fail)?;
        let markdown = to_markdown(&processed.html)
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        inert::audit_markdown(&markdown).map_err(TestCaseError::fail)?;
        prop_assert_eq!(markdown.trim(), markdown.as_str());
    }

    #[test]
    fn word_count_matches_the_text(html in hostile_html()) {
        let processed = process(&html, None);
        let words = u32::try_from(processed.text.split_whitespace().count()).unwrap();
        prop_assert_eq!(processed.word_count, words);
    }
}

//! The export path, `content::process` → `content::to_markdown`: it
//! always converts, the markdown that lands in the vault is inert, and no
//! nesting depth aborts it.

use curio_core::content::{process, to_markdown};
use proptest::prelude::*;

use crate::generators::{deep_html, exportable_html, hostile_html};
use crate::inert;

/// The stack of a tokio worker — where the desktop head exports.
const WORKER_STACK: usize = 2 << 20;

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

proptest! {
    // Each case parses ~1000+ levels several times (quadratic in html5ever),
    // so fewer cases than the default keep the suite quick.
    #![proptest_config(ProptestConfig::with_cases(16))]

    #[test]
    fn deep_nesting_exports_with_its_text((html, words) in deep_html()) {
        let processed = process(&html, None);
        // On a worker-sized stack: a stack overflow aborts the whole test
        // binary — no `catch_unwind`, no shrinking — which is the bug (#244).
        let markdown = std::thread::Builder::new()
            .stack_size(WORKER_STACK)
            .spawn(move || to_markdown(&processed.html))
            .unwrap()
            .join()
            .unwrap()
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        inert::audit_markdown(&markdown).map_err(TestCaseError::fail)?;
        for word in &words {
            prop_assert!(markdown.contains(word.as_str()), "lost {:?}", word);
        }
    }
}

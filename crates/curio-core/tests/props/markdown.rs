//! The export path, `content::process` → `content::to_markdown`: it
//! always converts, the markdown that lands in the vault is inert, and no
//! nesting depth aborts it.

use curio_core::content::{process, to_markdown};
use proptest::prelude::*;

use crate::generators::{deep_html, hostile_html};
use crate::inert;

/// The stack of a tokio worker — where the desktop head exports.
const WORKER_STACK: usize = 2 << 20;

proptest! {
    #[test]
    fn exported_markdown_is_inert(html in hostile_html()) {
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

/// Sanitizes and exports deep `html` on a worker-sized stack, auditing the
/// markdown: a stack overflow aborts the whole test binary — no
/// `catch_unwind`, no shrinking — which is the bug (#244).
fn export_deep(html: &str) -> Result<String, TestCaseError> {
    let processed = process(html, None);
    let markdown = std::thread::Builder::new()
        .stack_size(WORKER_STACK)
        .spawn(move || to_markdown(&processed.html))
        .unwrap()
        .join()
        .unwrap()
        .map_err(|err| TestCaseError::fail(err.to_string()))?;
    inert::audit_markdown(&markdown).map_err(TestCaseError::fail)?;
    Ok(markdown)
}

/// The markdown's whitespace-separated words, stripped of their markup
/// (`*`, `>`, `-`, `1.`, `|`): only the generated text is lowercase.
fn words_of(markdown: &str) -> Vec<String> {
    markdown
        .split_whitespace()
        .map(|token| token.chars().filter(char::is_ascii_lowercase).collect())
        .filter(|word: &String| !word.is_empty())
        .collect()
}

proptest! {
    // Each case parses ~1000+ levels several times (quadratic in html5ever),
    // so fewer cases than the default keep the suite quick.
    #![proptest_config(ProptestConfig::with_cases(16))]

    #[test]
    fn deep_nesting_exports_all_its_text_in_order((html, words) in deep_html(true)) {
        // With spans in the mix htmd may join words (see `deep_html`), so
        // this pins the text itself: every letter, in order, none invented.
        let markdown = export_deep(&html)?;
        prop_assert_eq!(words_of(&markdown).concat(), words.concat());
    }

    #[test]
    fn deep_nesting_keeps_its_word_boundaries((html, words) in deep_html(false)) {
        // Every word, in order, none joined with or split from another —
        // above the cap and below it, wherever the cap lands.
        let exported = words_of(&export_deep(&html)?);
        if let Some(at) = (0..exported.len().max(words.len()))
            .find(|&i| exported.get(i) != words.get(i))
        {
            let around = |list: &[String]| {
                list[at.saturating_sub(2).min(list.len())..(at + 3).min(list.len())].to_vec()
            };
            return Err(TestCaseError::fail(format!(
                "word {at} differs: exported {:?}, source {:?}",
                around(&exported),
                around(&words)
            )));
        }
    }
}

//! `content::sanitize`: whatever goes in, what comes out is inert — and
//! sanitizing sanitized output changes nothing.

use curio_core::content::sanitize;
use proptest::prelude::*;

use crate::generators::hostile_html;
use crate::inert;

const BASES: &[Option<&str>] = &[
    None,
    Some("https://example.com/posts/current"),
    Some("http://plain.example/dir/"),
    Some("not a url at all"),
];

proptest! {
    #[test]
    fn sanitized_hostile_html_passes_the_audit(
        html in hostile_html(),
        base in prop::sample::select(BASES),
    ) {
        inert::audit_html(&sanitize(&html, base)).map_err(TestCaseError::fail)?;
    }

    #[test]
    fn sanitized_arbitrary_text_passes_the_audit(text in "\\PC{0,2000}") {
        inert::audit_html(&sanitize(&text, Some("https://example.com/")))
            .map_err(TestCaseError::fail)?;
    }

    /// Re-sanitizing stored content (a re-ingest, a migration) is safe and
    /// settles: the second pass may still normalize the serialization —
    /// HTML's parser re-nests what ammonia serialized from a misnested
    /// tree (`<h1><h1></h1></h1>` reparses as siblings) — but it stays
    /// inert, and a third pass changes nothing.
    #[test]
    fn sanitize_settles_by_the_second_pass(html in hostile_html()) {
        let base = Some("https://example.com/posts/current");
        let twice = sanitize(&sanitize(&html, base), base);
        inert::audit_html(&twice).map_err(TestCaseError::fail)?;
        prop_assert_eq!(sanitize(&twice, base), twice);
    }
}

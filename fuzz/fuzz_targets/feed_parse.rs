//! Feed bytes → `parse_feed` → every entry through the ingest content
//! pipeline and the export converter: the full path a hostile feed takes.
//! Any panic or abort, or stored content failing the inertness audit, is a
//! finding.

#![no_main]

// The markdown half of the audit is not run here yet (#245).
#[allow(dead_code)]
#[path = "../../crates/curio-core/tests/common/inert.rs"]
mod inert;

use curio_core::content::{first_image, process, to_markdown};
use curio_core::feeds::parse_feed;
use curio_core::fetch::screen_url;
use libfuzzer_sys::fuzz_target;

const BASE: &str = "https://fuzz.example/feed.xml";

fuzz_target!(|data: &[u8]| {
    let Ok(feed) = parse_feed(data, Some(BASE)) else {
        return;
    };
    // Only web addresses are stored as links (#262).
    let web_or_none = |url: Option<&str>| url.is_none_or(|url| screen_url(url).is_ok());
    assert!(
        web_or_none(feed.meta.site_url.as_deref()),
        "a non-http(s) site url"
    );
    for entry in feed.entries {
        assert!(!entry.dedupe_key.is_empty(), "the dedupe chain is total");
        assert!(
            web_or_none(entry.link.as_deref()),
            "a non-http(s) entry link"
        );
        let base = entry.link.as_deref().unwrap_or(BASE);
        let processed = process(&entry.content_html, Some(base));
        if let Err(violation) = inert::audit_html(&processed.html) {
            panic!("a feed entry stored live content: {violation}");
        }
        if let Some(image) = first_image(&processed.html) {
            assert!(image.starts_with("http://") || image.starts_with("https://"));
        }
        let _ = to_markdown(&processed.html);
    }
});

//! Feed bytes → `parse_feed` → every entry through the ingest content
//! pipeline and the export converter: the full path a hostile feed takes.
//! Any panic or abort, or stored content or exported markdown failing the
//! inertness audit, is a finding.

#![no_main]

#[path = "../../crates/curio-core/tests/common/inert.rs"]
mod inert;

use curio_core::content::{first_image, process, to_markdown};
use curio_core::feeds::parse_feed;
use libfuzzer_sys::fuzz_target;

const BASE: &str = "https://fuzz.example/feed.xml";

fuzz_target!(|data: &[u8]| {
    let Ok(feed) = parse_feed(data, Some(BASE)) else {
        return;
    };
    for entry in feed.entries {
        assert!(!entry.dedupe_key.is_empty(), "the dedupe chain is total");
        let base = entry.link.as_deref().unwrap_or(BASE);
        let processed = process(&entry.content_html, Some(base));
        if let Err(violation) = inert::audit_html(&processed.html) {
            panic!("a feed entry stored live content: {violation}");
        }
        if let Some(image) = first_image(&processed.html) {
            assert!(image.starts_with("http://") || image.starts_with("https://"));
        }
        if let Ok(markdown) = to_markdown(&processed.html)
            && let Err(violation) = inert::audit_markdown(&markdown)
        {
            panic!("a feed entry exported live markdown: {violation}");
        }
    }
});

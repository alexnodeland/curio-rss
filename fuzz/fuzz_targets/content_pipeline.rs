//! Arbitrary HTML → both content pipelines (feed fragment and full page)
//! and the export converter. Whatever is stored must pass the same
//! structural inertness audit the integration suites use, and so must the
//! markdown it exports.

#![no_main]

#[path = "../../crates/curio-core/tests/common/inert.rs"]
mod inert;

use curio_core::content::{plain_text, process, process_page, to_markdown};
use libfuzzer_sys::fuzz_target;

const BASE: &str = "https://fuzz.example/posts/current";

fuzz_target!(|data: &[u8]| {
    let html = String::from_utf8_lossy(data);
    let fragment = process(&html, Some(BASE));
    if let Err(violation) = inert::audit_html(&fragment.html) {
        panic!("feed-fragment pipeline stored live content: {violation}");
    }
    let _ = plain_text(&fragment.html);
    audit_export(&fragment.html, "feed-fragment");
    if let Ok(page) = process_page(&html, BASE) {
        if let Err(violation) = inert::audit_html(&page.content.html) {
            panic!("full-page pipeline stored live content: {violation}");
        }
        audit_export(&page.content.html, "full-page");
    }
});

/// Stored HTML exports to markdown that is inert too.
fn audit_export(stored: &str, pipeline: &str) {
    if let Ok(markdown) = to_markdown(stored)
        && let Err(violation) = inert::audit_markdown(&markdown)
    {
        panic!("{pipeline} pipeline exported live markdown: {violation}");
    }
}

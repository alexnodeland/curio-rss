//! Arbitrary HTML → both content pipelines (feed fragment and full page)
//! and the export converter. Whatever is stored must pass the same
//! structural inertness audit the integration suites use.

#![no_main]

// The markdown half of the audit is not run here yet (#245).
#[allow(dead_code)]
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
    let _ = to_markdown(&fragment.html);
    if let Ok(page) = process_page(&html, BASE) {
        if let Err(violation) = inert::audit_html(&page.content.html) {
            panic!("full-page pipeline stored live content: {violation}");
        }
        let _ = to_markdown(&page.content.html);
    }
});

//! Golden tests for the content pipeline: every `fixtures/html/*.html`
//! runs sanitize → extract → markdown and is pinned to a committed
//! snapshot (insta). The XSS corpus rides the same path, plus explicit
//! inertness assertions — the permanent CI regression gate for the
//! stored-XSS class (D6).

#![allow(clippy::unwrap_used)]

use curio_core::content::{process, to_markdown};

fn pipeline(raw_html: &str) -> String {
    let processed = process(raw_html, Some("https://example.com/posts/current"));
    to_markdown(&processed.html).unwrap()
}

#[test]
fn golden_html_to_markdown() {
    insta::glob!("../../../fixtures/html", "*.html", |path| {
        let raw = std::fs::read_to_string(path).unwrap();
        let markdown = pipeline(&raw);
        insta::assert_snapshot!(markdown);
    });
}

#[test]
fn xss_corpus_output_is_inert() {
    let raw = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/html/xss-corpus.html"
    ))
    .unwrap();
    let processed = process(&raw, Some("https://example.com/posts/current"));
    let markdown = to_markdown(&processed.html).unwrap();

    // Structural audit of the stored HTML: no forbidden elements, no
    // event-handler or style attributes, http(s)-only URL attributes.
    // (Substring scans would false-positive on inert anchor *text* like
    // "javascript: link", which sanitization correctly leaves behind.)
    let stored = scraper::Html::parse_fragment(&processed.html);
    let all = scraper::Selector::parse("*").unwrap();
    for element in stored.select(&all) {
        let name = element.value().name();
        assert!(
            ![
                "script", "svg", "iframe", "frame", "object", "embed", "form", "input", "button",
                "style", "base", "meta", "link", "audio", "video", "math",
            ]
            .contains(&name),
            "forbidden element <{name}> survived:\n{}",
            processed.html
        );
        for (attr, value) in element.value().attrs() {
            assert!(
                !attr.starts_with("on") && attr != "style" && attr != "http-equiv",
                "forbidden attribute {attr:?} survived on <{name}>"
            );
            if matches!(attr, "href" | "src") {
                assert!(
                    value.starts_with("http://") || value.starts_with("https://"),
                    "non-http(s) {attr} survived on <{name}>: {value:?}"
                );
            }
        }
    }

    // The markdown, which is what gets exported, carries no executable
    // remnants: no scriptable link/image destinations, no raw HTML.
    let lower = markdown.to_lowercase();
    for vector in [
        "](javascript:",
        "](data:",
        "<script",
        "onerror=",
        "onload=",
        "evil.example",
    ] {
        assert!(
            !lower.contains(vector),
            "markdown still contains {vector:?}:\n{markdown}"
        );
    }
    // The legitimate content around the attacks survives.
    assert!(markdown.contains("**content**"));
    assert!(markdown.contains("https://example.com/fine"));
}

#[test]
fn code_heavy_fixture_keeps_every_line() {
    let raw = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/html/code-heavy.html"
    ))
    .unwrap();
    let markdown = pipeline(&raw);
    for line in [
        "fn main() {",
        "let xs: Vec<i32> = (0..10).collect();",
        "println!(\"{sum}\");",
        "FROM articles",
        "ORDER BY rank;",
    ] {
        assert!(markdown.contains(line), "lost {line:?} in:\n{markdown}");
    }
}

#[test]
fn article_page_extraction_drops_chrome_keeps_story() {
    let raw = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/html/article-page.html"
    ))
    .unwrap();
    let markdown = pipeline(&raw);
    assert!(markdown.contains("write-ahead log"));
    assert!(markdown.contains("PRAGMA journal_mode = WAL;"));
    assert!(markdown.contains("![WAL architecture diagram]"));
    assert!(!markdown.contains("Related posts"), "aside leaked");
    assert!(!markdown.contains("© 2026"), "footer leaked");
    assert!(!markdown.contains("Archive"), "nav leaked");
}

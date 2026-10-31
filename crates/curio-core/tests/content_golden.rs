//! Golden tests for the content pipeline: every `fixtures/html/*.html`
//! runs sanitize → extract → markdown and is pinned to a committed
//! snapshot (insta). The XSS corpus rides the same path, plus explicit
//! inertness assertions — the permanent CI regression gate for the
//! stored-XSS class (D6).

#![allow(clippy::unwrap_used)]

mod common;

use curio_core::content::{plain_text, process, sanitize, to_markdown};

fn pipeline(raw_html: &str) -> String {
    let processed = process(raw_html, Some("https://example.com/posts/current"));
    to_markdown(&processed.html).unwrap()
}

fn html_fixture(name: &str) -> String {
    std::fs::read_to_string(
        common::manifest_dir()
            .join("../../fixtures/html")
            .join(name),
    )
    .unwrap()
}

/// insta locates `fixtures/html` (the glob base) and the committed
/// snapshots from the workspace root. Left to itself it derives that root
/// from the crate dir baked in at compile time, which is another worktree's
/// when cargo reuses this binary from a shared `CARGO_TARGET_DIR` (issue
/// #183). `.cargo/config.toml` hands it the root at run time instead.
///
/// Cargo discovers `.cargo/config.toml` from its working directory, not
/// from `--manifest-path`, so a cargo started outside the checkout leaves
/// the variable unset and insta falls back to its own lookup — correct
/// whenever the binary was built here. The config entry itself is pinned
/// unconditionally; the variable is checked whenever cargo passed it.
#[test]
fn insta_resolves_the_workspace_root_at_run_time() {
    let workspace_root = common::manifest_dir().join("../..").canonicalize().unwrap();

    let config = std::fs::read_to_string(workspace_root.join(".cargo/config.toml")).unwrap();
    assert!(
        config
            .lines()
            .any(|line| line.trim() == r#"INSTA_WORKSPACE_ROOT = { value = ".", relative = true }"#),
        ".cargo/config.toml [env] must set INSTA_WORKSPACE_ROOT relative to the checkout",
    );

    if let Some(root) = std::env::var_os("INSTA_WORKSPACE_ROOT") {
        assert_eq!(
            std::path::Path::new(&root).canonicalize().unwrap(),
            workspace_root,
        );
    }
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
    let raw = html_fixture("xss-corpus.html");
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
    let raw = html_fixture("code-heavy.html");
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
    let raw = html_fixture("article-page.html");
    let markdown = pipeline(&raw);
    assert!(markdown.contains("write-ahead log"));
    assert!(markdown.contains("PRAGMA journal_mode = WAL;"));
    assert!(markdown.contains("![WAL architecture diagram]"));
    assert!(!markdown.contains("Related posts"), "aside leaked");
    assert!(!markdown.contains("© 2026"), "footer leaked");
    assert!(!markdown.contains("Archive"), "nav leaked");
}

/// Feed fragments are sanitized only (#128): the stored text is exactly
/// the sanitized input's text — no sibling block, quote attribution,
/// figure caption or in-article aside is dropped by extraction.
#[test]
fn fragment_fixtures_keep_all_their_text() {
    for (name, phrases) in [
        (
            "multi-section-post.html",
            &[
                "Why we moved the queue",
                "deliberately the shorter",
                "What changed after the move",
                "on-call rotation",
                "Ghost-style callout card",
            ][..],
        ),
        (
            "intro-wrapped-div.html",
            &[
                "A one-line intro that sits outside the body wrapper.",
                "The main body lives inside a wrapper div",
                "A second body paragraph",
            ][..],
        ),
        (
            "blockquote-attribution.html",
            &[
                "Simplicity is prerequisite for reliability.",
                "— Edsger W. Dijkstra",
                "A program, before version control.",
                "Both the attribution and the caption",
            ][..],
        ),
        (
            "in-article-aside.html",
            &[
                "# Notes on backpressure",
                "By the editors, filed under systems.",
                "**Pull quote:** an unbounded queue",
                "Bounding the queue",
                "Corrections and follow-ups are welcome.",
            ][..],
        ),
    ] {
        let raw = html_fixture(name);
        let base = Some("https://example.com/posts/current");
        let processed = process(&raw, base);
        assert_eq!(
            processed.text,
            plain_text(&sanitize(&raw, base)),
            "{name}: extraction dropped text"
        );
        let markdown = to_markdown(&processed.html).unwrap();
        for phrase in phrases {
            assert!(
                markdown.contains(phrase),
                "{name}: lost {phrase:?} in:\n{markdown}"
            );
        }
    }
}

/// A feed item that is a whole page still loses its chrome, but every
/// content-bearing block around the top scorer survives (#128).
#[test]
fn page_fixture_drops_chrome_keeps_every_content_block() {
    let markdown = pipeline(&html_fixture("page-split-content.html"));
    for kept in [
        "A short standfirst",
        "## What shipped",
        "release highlights",
        "## What is next",
        "Ship small, ship often.",
        "— the release team",
    ] {
        assert!(markdown.contains(kept), "lost {kept:?} in:\n{markdown}");
    }
    for chrome in ["Example Blog", "Home", "Related posts", "© 2026"] {
        assert!(
            !markdown.contains(chrome),
            "chrome {chrome:?} leaked:\n{markdown}"
        );
    }
}

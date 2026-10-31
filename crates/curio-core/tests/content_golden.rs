//! Golden tests for the content pipeline: every `fixtures/html/*.html`
//! runs sanitize → extract → markdown and is pinned to a committed
//! snapshot (insta). The XSS corpus rides the same path, plus explicit
//! inertness assertions — the permanent CI regression gate for the
//! stored-XSS class (D6).

#![allow(clippy::unwrap_used)]

mod common;
#[path = "common/inert.rs"]
mod inert;

use curio_core::content::{process, to_markdown};

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

    // Structural audit of the stored HTML, then the exported markdown
    // (common/inert.rs — the shared definition the nasty corpus and the
    // proptests hold every other input to).
    inert::audit_html(&processed.html).unwrap();
    inert::audit_markdown(&markdown).unwrap();
    // This fixture leaves no trace of its attacks at all — not even as
    // escaped text — and none of them reach the attacker host.
    let lower = markdown.to_lowercase();
    for vector in ["<script", "onerror=", "onload=", "evil.example"] {
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

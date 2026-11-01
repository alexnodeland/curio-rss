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

/// Every attribute the sanitize gate may leave on stored HTML.
const ALLOWED_ATTRIBUTES: &[&str] = &[
    "href", "rel", "title", "src", "alt", "datetime", "colspan", "rowspan", "id", "dir", "lang",
    "start", "class",
];

/// The id/fragment token shape: ASCII alphanumerics and `-_.:`.
fn is_plain_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
}

/// The stored (sanitized + extracted) HTML for a fixture.
fn stored_html(name: &str) -> String {
    process(
        &html_fixture(name),
        Some("https://example.com/posts/current"),
    )
    .html
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
    // event-handler or style attributes, http(s)-only URL attributes
    // (plus prefixed in-page fragments), and every other surviving
    // attribute both allowlisted and value-filtered.
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
            assert!(
                ALLOWED_ATTRIBUTES.contains(&attr),
                "unexpected attribute {attr:?} survived on <{name}>"
            );
            if matches!(attr, "href" | "src") {
                let in_page_fragment = attr == "href"
                    && value
                        .strip_prefix("#user-content-")
                        .is_some_and(is_plain_token);
                assert!(
                    value.starts_with("http://")
                        || value.starts_with("https://")
                        || in_page_fragment,
                    "non-http(s) {attr} survived on <{name}>: {value:?}"
                );
            }
            match attr {
                "id" => assert!(
                    value
                        .strip_prefix("user-content-")
                        .is_some_and(is_plain_token),
                    "unprefixed or unfiltered id on <{name}>: {value:?}"
                ),
                "dir" => assert!(
                    matches!(value, "ltr" | "rtl" | "auto"),
                    "unfiltered dir on <{name}>: {value:?}"
                ),
                "lang" => assert!(
                    !value.is_empty()
                        && value
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
                    "unfiltered lang on <{name}>: {value:?}"
                ),
                "start" => assert!(
                    name == "ol" && value.parse::<i32>().is_ok(),
                    "unfiltered start on <{name}>: {value:?}"
                ),
                "class" => assert!(
                    matches!(name, "pre" | "code")
                        && value.strip_prefix("language-").is_some_and(|lang| {
                            !lang.is_empty()
                                && lang
                                    .bytes()
                                    .all(|b| b.is_ascii_alphanumeric() || b"-_+#.".contains(&b))
                        }),
                    "unfiltered class on <{name}>: {value:?}"
                ),
                _ => {}
            }
        }
    }

    // The markdown, which is what gets exported, carries no executable
    // remnants: no scriptable link/image destinations, no raw HTML.
    let lower = markdown.to_lowercase();
    for vector in [
        "](javascript:",
        "](data:",
        "](vbscript:",
        "](#",
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

#[test]
fn lazy_images_fixture_recovers_every_real_source_once() {
    let markdown = pipeline(&html_fixture("lazy-images.html"));
    for image in [
        "![from data-src](https://example.com/uploads/data-src.jpg)",
        "![from data-lazy-src](https://cdn.example.com/lazy-src.png)",
        "![from srcset](https://cdn.example.com/w_1200,h_900/srcset.jpg)",
        "![from noscript](https://cdn.example.com/noscript.webp)",
        "![plain](https://cdn.example.com/plain.png)",
    ] {
        assert_eq!(
            markdown.matches(image).count(),
            1,
            "expected {image:?} exactly once in:\n{markdown}"
        );
    }
    assert!(
        !markdown.contains("](data:"),
        "placeholder leaked:\n{markdown}"
    );
    assert!(
        !markdown.contains("spacer.gif"),
        "spacer leaked:\n{markdown}"
    );
    assert!(
        !markdown.contains("![placeholder]"),
        "placeholder kept beside its fallback:\n{markdown}"
    );
}

#[test]
fn footnotes_fixture_round_trips_to_commonmark_footnotes() {
    let markdown = pipeline(&html_fixture("footnotes.html"));
    assert!(
        markdown.contains("Pandoc-style references[^1] and a"),
        "{markdown}"
    );
    assert!(markdown.contains("kramdown-style one[^2]."), "{markdown}");
    assert!(
        markdown.contains("[^1]: The first note, with a [source](https://example.com/source)."),
        "{markdown}"
    );
    assert!(
        markdown
            .contains("[^2]: The second note spans two paragraphs.\n\n    This is the second one."),
        "{markdown}"
    );
    assert!(
        markdown.contains("A jump to a later section keeps its words."),
        "{markdown}"
    );
    assert!(markdown.contains("3.  third\n4.  fourth"), "{markdown}");
    assert!(
        !markdown.contains('↩'),
        "back-reference leaked:\n{markdown}"
    );
    assert!(!markdown.contains("](#"), "dead in-page link:\n{markdown}");
    assert!(
        !markdown.contains("example.com/posts/current#"),
        "fragment was absolutized:\n{markdown}"
    );

    // The stored HTML keeps working in-page links for the reader.
    let html = stored_html("footnotes.html");
    for needle in [
        "href=\"#user-content-fn1\"",
        "<li id=\"user-content-fn1\">",
        "href=\"#user-content-fnref1\"",
        "<sup id=\"user-content-fnref1\">",
        "href=\"#user-content-fn:2\"",
        "<li id=\"user-content-fn:2\">",
        "<h2 id=\"user-content-later\">",
        "href=\"#user-content-later\"",
        "<ol start=\"3\">",
    ] {
        assert!(html.contains(needle), "{needle} missing from:\n{html}");
    }
}

/// The `dir` / `lang` that apply to the first element matching `selector`
/// in stored HTML: its own, else the nearest ancestor's.
fn inherited_direction(html: &str, selector: &str) -> (Option<String>, Option<String>) {
    let doc = scraper::Html::parse_fragment(html);
    let selector = scraper::Selector::parse(selector).unwrap();
    let element = doc.select(&selector).next().unwrap();
    let inherited = |name: &str| {
        std::iter::once(element)
            .chain(element.ancestors().filter_map(scraper::ElementRef::wrap))
            .find_map(|el| el.value().attr(name))
            .map(ToOwned::to_owned)
    };
    (inherited("dir"), inherited("lang"))
}

#[test]
fn rtl_fixture_keeps_direction_and_language() {
    let rtl = (Some("rtl".to_owned()), Some("ar".to_owned()));
    // Direction set only on the container must still reach everything
    // that inherits it — the heading and the bare paragraph — both as a
    // feed fragment and as a whole-page item the extractor runs on.
    let fragment = stored_html("rtl.html");
    let page = process(
        &format!(
            "<html><body><nav><a href=\"/\">Home</a></nav>{}</body></html>",
            html_fixture("rtl.html")
        ),
        Some("https://example.com/posts/current"),
    )
    .html;
    assert!(!page.contains("Home"), "not extracted:\n{page}");
    for html in [&fragment, &page] {
        assert_eq!(inherited_direction(html, "h1"), rtl, "heading:\n{html}");
        assert_eq!(
            inherited_direction(html, "p"),
            rtl,
            "bare paragraph:\n{html}"
        );
        for needle in [
            "<p>هذه فقرة عربية ترث اتجاهها",
            "<p dir=\"ltr\" lang=\"en\">",
            "<bdi>إيان</bdi>",
            "<bdo dir=\"rtl\">reversed</bdo>",
        ] {
            assert!(html.contains(needle), "{needle} missing from:\n{html}");
        }
    }
    assert!(pipeline(&html_fixture("rtl.html")).contains("هذه فقرة عربية"));
}

#[test]
fn code_fence_fixture_keeps_each_language() {
    let markdown = pipeline(&html_fixture("code-fence-language.html"));
    assert!(
        markdown.contains("```rust\nfn main() {"),
        "rust fence lost:\n{markdown}"
    );
    assert!(
        markdown.contains("```python\nprint(\"hello\")"),
        "python fence lost:\n{markdown}"
    );
    assert!(
        markdown.contains("```\nplain text\n```"),
        "bare fence changed:\n{markdown}"
    );
}

#[test]
fn embeds_fixture_turns_every_visible_embed_into_a_link() {
    let markdown = pipeline(&html_fixture("embeds.html"));
    for link in [
        "[▶ Embedded media (open.spotify.com) ↗](https://open.spotify.com/embed/episode/7makk4oTQel546B0PZlDM5)",
        "[▶ Embedded media (w.soundcloud.com) ↗](https://w.soundcloud.com/player/?url=https%3A//api.soundcloud.com/tracks/293)",
        "[Embedded content (codepen.io) ↗](https://codepen.io/team/codepen/embed/PNaGbb?default-tab=result)",
        "[Embedded content (mastodon.social) ↗](https://mastodon.social/@Mastodon/109349437765541430/embed)",
        "[Embedded content (embed.bsky.app) ↗]",
        "[Embedded content (maps.example.org) ↗](https://maps.example.org/embed?pb=!1m18)",
    ] {
        assert!(markdown.contains(link), "{link} missing from:\n{markdown}");
    }
    assert!(
        !markdown.contains("track.example.net"),
        "tracking frame leaked:\n{markdown}"
    );
    for chrome in ["outbrain.com", "platform.twitter.com", "disqus.com"] {
        assert!(
            !markdown.contains(chrome),
            "{chrome} widget became a link:\n{markdown}"
        );
    }
    assert!(markdown.contains("The end."));
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

//! Readability extraction: find the main content, drop the page chrome.
//!
//! A deliberately small, deterministic, scraper-based port of the
//! readability idea — not a heuristic zoo. It only runs on feed items
//! that really are whole pages (see [`super::process`]); plain feed
//! fragments are sanitized and stored as-is.
//!
//! 1. **Chrome is detached first**: every `nav`, and every `header`,
//!    `footer` or `aside` that stands at page level. The same elements
//!    nested inside an `article`, `blockquote` or `figure` are content —
//!    an article's own header, a quote's `<footer>— Author</footer>`
//!    attribution, a figure credit, an in-article callout — and stay.
//! 2. **The tightest wrapper around the prose wins**: candidates are the
//!    sectioning containers (`article`, `main`, `section`, `div`, and
//!    `body` as the floor), each scored by the text mass of its *direct*
//!    content children (paragraphs, headings, lists, quotes, code,
//!    figures, tables).
//! 3. **Content-bearing siblings are never dropped**: the winner widens
//!    to its parent, level by level up to `body`, whenever a sibling
//!    carries prose of its own (text outside links). An intro paragraph
//!    beside the body `div`, or a post split across two `section`s,
//!    survives whole; link-only rows (share buttons, tag lists) are no
//!    reason to widen.

use ego_tree::{NodeId, NodeRef};
use scraper::{ElementRef, Html, Node, Selector};

/// Container elements considered as main-content candidates.
const CANDIDATES: &[&str] = &["article", "main", "section", "div", "body"];

/// Direct children that count as content mass when scoring a candidate.
const CONTENT_CHILDREN: &[&str] = &[
    "p",
    "pre",
    "blockquote",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "ul",
    "ol",
    "dl",
    "figure",
    "table",
];

/// Always page chrome, wherever it sits.
const ALWAYS_CHROME: &[&str] = &["nav"];

/// Page chrome only at page level: inside a [`CONTENT_SCOPES`] element
/// these are part of the content.
const SCOPED_CHROME: &[&str] = &["aside", "header", "footer"];

/// Containers whose own `header`/`footer`/`aside` belong to the content.
const CONTENT_SCOPES: &[&str] = &["article", "blockquote", "figure"];

/// Extracts the main content of an **already sanitized** HTML document
/// or fragment, returning its inner HTML. Page chrome is removed, the
/// tightest container around the prose is chosen, then widened so no
/// content-bearing sibling block is lost. When nothing scores (bare
/// text, unstructured markup), the whole body passes through.
#[must_use]
pub fn extract_main_content(sanitized_html: &str) -> String {
    let mut document = Html::parse_document(sanitized_html);

    detach_chrome(&mut document);

    let Some(best_id) = best_candidate(&document) else {
        return sanitized_html.to_owned();
    };
    let top_id = widen_over_content_siblings(&document, best_id);

    document
        .tree
        .get(top_id)
        .and_then(ElementRef::wrap)
        .map_or_else(
            || sanitized_html.to_owned(),
            |el| el.inner_html().trim().to_owned(),
        )
}

/// Whether an **already sanitized** fragment carries page chrome (a
/// `nav`, or a page-level `header`/`footer`/`aside`) — what separates a
/// whole page worth extracting from a feed fragment that merely arrived
/// wrapped in `<html>`/`<body>`.
#[must_use]
pub(crate) fn has_page_chrome(sanitized_html: &str) -> bool {
    Html::parse_fragment(sanitized_html)
        .root_element()
        .descendants()
        .filter_map(ElementRef::wrap)
        .any(is_chrome)
}

/// `nav` anywhere; `header`/`footer`/`aside` unless nested inside an
/// `article`, `blockquote` or `figure`.
fn is_chrome(element: ElementRef<'_>) -> bool {
    let name = element.value().name();
    if ALWAYS_CHROME.contains(&name) {
        return true;
    }
    SCOPED_CHROME.contains(&name)
        && !element
            .ancestors()
            .filter_map(ElementRef::wrap)
            .any(|ancestor| CONTENT_SCOPES.contains(&ancestor.value().name()))
}

/// Detaches every chrome element in the document.
fn detach_chrome(document: &mut Html) {
    let ids: Vec<NodeId> = document
        .root_element()
        .descendants()
        .filter_map(ElementRef::wrap)
        .filter(|el| is_chrome(*el))
        .map(|el| el.id())
        .collect();
    for id in ids {
        if let Some(mut node) = document.tree.get_mut(id) {
            node.detach();
        }
    }
}

/// The highest-scoring candidate's node id, preferring the *tightest*
/// wrapper: on equal scores a descendant beats its ancestor (document
/// order puts ancestors first, so `>` keeps the first-seen ancestor
/// only when it strictly outscores).
fn best_candidate(document: &Html) -> Option<NodeId> {
    let mut best: Option<(usize, NodeId)> = None;
    for name in CANDIDATES {
        let selector = Selector::parse(name).ok()?;
        for element in document.select(&selector) {
            let score = content_score(element);
            if score == 0 {
                continue;
            }
            let candidate = (score, element.id());
            best = match best {
                Some((top, id)) if top >= score => Some((top, id)),
                _ => Some(candidate),
            };
        }
    }
    if let Some((_, id)) = best {
        Some(id)
    } else {
        // Nothing scored: fall back to the whole body, if there is one.
        let body = Selector::parse("body").ok()?;
        document.select(&body).next().map(|el| el.id())
    }
}

/// Text mass of the element's direct content children.
fn content_score(element: ElementRef<'_>) -> usize {
    element
        .children()
        .filter_map(ElementRef::wrap)
        .filter(|child| CONTENT_CHILDREN.contains(&child.value().name()))
        .map(|child| child.text().map(str::len).sum::<usize>())
        .sum()
}

/// Walks from the winner up to `body`; whenever the node at some level
/// has a sibling that carries prose, the result widens to that level's
/// parent. The walk always reaches `body`, so content several wrappers
/// away (an intro beside an outer page `div`) is still found.
fn widen_over_content_siblings(document: &Html, winner: NodeId) -> NodeId {
    let mut top = winner;
    let Some(mut cursor) = document.tree.get(winner) else {
        return winner;
    };
    while !is_element_named(cursor, "body") {
        let Some(parent) = cursor.parent() else {
            break;
        };
        if !parent.value().is_element() {
            break;
        }
        let sibling_has_prose = parent
            .children()
            .filter(|sibling| sibling.id() != cursor.id())
            .any(carries_prose);
        if sibling_has_prose {
            top = parent.id();
        }
        cursor = parent;
    }
    top
}

fn is_element_named(node: NodeRef<'_, Node>, name: &str) -> bool {
    node.value()
        .as_element()
        .is_some_and(|element| element.name() == name)
}

/// Whether a node holds non-whitespace text outside any link. Link-only
/// blocks (share rows, tag lists, pagination) don't count; a paragraph
/// that merely contains a link does.
fn carries_prose(node: NodeRef<'_, Node>) -> bool {
    match node.value() {
        Node::Text(text) => !text.trim().is_empty(),
        Node::Element(element) if element.name() == "a" => false,
        Node::Element(_) => node.children().any(carries_prose),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn article_content_wins_over_navigation() {
        let html = "<body>\
            <nav><ul><li><a href=\"https://e.com/\">Home</a></li></ul></nav>\
            <article><p>The actual story, with enough words to matter for scoring.</p>\
            <p>And a second paragraph to seal it.</p></article>\
            <footer><p>tiny</p></footer></body>";
        let main = extract_main_content(html);
        assert!(main.contains("actual story"));
        assert!(!main.contains("Home"), "nav must not leak: {main}");
        assert!(!main.contains("tiny"), "page footer must not leak: {main}");
    }

    #[test]
    fn page_level_chrome_inside_the_winner_is_removed() {
        let html = "<main>\
            <header><h1>Site chrome</h1></header>\
            <p>Body text one, long enough to score decisively here.</p>\
            <p>Body text two, also long enough to add to the score.</p>\
            <aside><p>promo promo promo</p></aside>\
            <nav><a href=\"https://e.com/next\">Next post</a></nav>\
            </main>";
        let main = extract_main_content(html);
        assert!(main.contains("Body text one"));
        assert!(!main.contains("Site chrome"));
        assert!(!main.contains("promo"));
        assert!(!main.contains("Next post"));
    }

    #[test]
    fn header_footer_aside_inside_an_article_are_content() {
        let html = "<body><nav><a href=\"https://e.com/\">Home</a></nav><article>\
            <header><h1>The post title</h1><p>By A. Writer</p></header>\
            <p>Body text one, long enough to score decisively here.</p>\
            <aside><p>Callout: this matters.</p></aside>\
            <p>Body text two, also long enough to add to the score.</p>\
            <footer><p>Filed under essays.</p></footer>\
            </article></body>";
        let main = extract_main_content(html);
        for kept in [
            "The post title",
            "By A. Writer",
            "Callout: this matters.",
            "Filed under essays.",
        ] {
            assert!(main.contains(kept), "lost {kept:?}: {main}");
        }
        assert!(!main.contains("Home"), "nav must not leak: {main}");
    }

    #[test]
    fn quote_and_figure_footers_survive_outside_an_article() {
        let html = "<body><header><nav>Home</nav></header><div>\
            <p>Body text one, long enough to score decisively here.</p>\
            <blockquote><p>Simplicity is prerequisite for reliability.</p>\
            <footer>— Edsger Dijkstra</footer></blockquote>\
            <figure><img src=\"https://e.com/a.png\" alt=\"chart\">\
            <footer>Photo: the author</footer></figure>\
            </div></body>";
        let main = extract_main_content(html);
        assert!(main.contains("— Edsger Dijkstra"), "{main}");
        assert!(main.contains("Photo: the author"), "{main}");
        assert!(!main.contains("Home"), "{main}");
    }

    #[test]
    fn an_intro_beside_the_body_div_survives() {
        let html = "<body><nav>Home</nav><p>A short intro.</p><div>\
            <p>The longer body paragraph that outscores the intro on its own.</p>\
            <p>And another one to make the div the clear top scorer.</p>\
            </div></body>";
        let main = extract_main_content(html);
        assert!(main.contains("A short intro."), "intro dropped: {main}");
        assert!(main.contains("longer body paragraph"), "{main}");
        assert!(!main.contains("Home"), "{main}");
    }

    #[test]
    fn content_split_across_sections_survives() {
        let html = "<body><header>Site</header><section>\
            <p>Part one of the post, a little shorter than the second.</p>\
            </section><section>\
            <p>Part two of the post, which carries rather more text than part one does.</p>\
            </section><footer>© site</footer></body>";
        let main = extract_main_content(html);
        assert!(main.contains("Part one"), "first section dropped: {main}");
        assert!(main.contains("Part two"), "{main}");
        assert!(!main.contains("© site"), "{main}");
    }

    #[test]
    fn content_several_wrappers_away_still_widens_the_winner() {
        let html = "<body><p>Lead paragraph at page level.</p>\
            <div id=\"page\"><div id=\"content\">\
            <p>Deep inner paragraph, long enough to win the scoring outright.</p>\
            <p>Another deep inner paragraph adding to the inner score.</p>\
            </div></div></body>";
        let main = extract_main_content(html);
        assert!(main.contains("Lead paragraph"), "{main}");
        assert!(main.contains("Deep inner paragraph"), "{main}");
    }

    #[test]
    fn link_only_siblings_do_not_widen_the_winner() {
        let html = "<div id=\"page\"><div id=\"content\">\
            <p>Only this inner div holds the prose directly.</p>\
            <p>So it stays the winner despite the share row.</p>\
            </div><div><a href=\"https://e.com/share\">Share</a> \
            <a href=\"https://e.com/tweet\">Tweet</a></div></div>";
        let main = extract_main_content(html);
        assert!(main.starts_with("<p>"), "widened over a link row: {main}");
        assert!(!main.contains("Share"), "{main}");
    }

    #[test]
    fn a_tight_inner_wrapper_beats_its_ancestor() {
        let html = "<div id=\"page\"><div id=\"content\">\
            <p>Only this inner div holds the paragraphs directly.</p>\
            <p>So it must be the winner, not the outer shell.</p>\
            </div></div>";
        let main = extract_main_content(html);
        assert!(main.starts_with("<p>"), "outer shell leaked: {main}");
    }

    #[test]
    fn fragments_pass_through() {
        let html = "<p>Just a fragment.</p><p>Nothing fancy.</p>";
        let main = extract_main_content(html);
        assert!(main.contains("Just a fragment."));
        assert!(main.contains("Nothing fancy."));
    }

    #[test]
    fn unstructured_text_falls_back_to_the_body() {
        let main = extract_main_content("bare text, no elements at all");
        assert_eq!(main, "bare text, no elements at all");
    }

    #[test]
    fn page_chrome_detection() {
        assert!(has_page_chrome("<nav>Home</nav><p>x</p>"));
        assert!(has_page_chrome(
            "<header>Site</header><article><p>x</p></article>"
        ));
        assert!(has_page_chrome("<div><aside>Related</aside></div>"));
        assert!(!has_page_chrome("<p>x</p><div><p>y</p></div>"));
        assert!(!has_page_chrome(
            "<article><header><h1>t</h1></header><aside>c</aside><footer>f</footer></article>"
        ));
        assert!(!has_page_chrome(
            "<blockquote><p>q</p><footer>— A</footer></blockquote>"
        ));
        assert!(!has_page_chrome(
            "<figure><img src=\"https://e.com/a.png\"><footer>credit</footer></figure>"
        ));
    }
}

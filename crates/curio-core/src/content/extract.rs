//! Readability extraction: find the main content, drop the boilerplate.
//!
//! A deliberately small, deterministic, scraper-based port of the
//! readability idea — not a heuristic zoo. Candidates are the sectioning
//! containers (`article`, `main`, `section`, `div`, and `body` as the
//! floor); each is scored by the text mass of its *direct* content
//! children (paragraphs, headings, lists, quotes, code, figures,
//! tables), so the tightest wrapper around the prose wins. Recognized
//! boilerplate (`nav`, `aside`, `header`, `footer`) is detached before
//! serializing the winner.
//!
//! Feed fragments (already just content) pass through essentially
//! unchanged: their `body` is the best candidate.

use ego_tree::NodeId;
use scraper::{ElementRef, Html, Selector};

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

/// Boilerplate sections removed from the extracted content.
const BOILERPLATE: &[&str] = &["nav", "aside", "header", "footer"];

/// Extracts the main content of an **already sanitized** HTML document
/// or fragment, returning its inner HTML. Boilerplate sections inside
/// the winning container are removed. When nothing scores (bare text,
/// unstructured markup), the whole body passes through.
#[must_use]
pub fn extract_main_content(sanitized_html: &str) -> String {
    let mut document = Html::parse_document(sanitized_html);

    let Some(best_id) = best_candidate(&document) else {
        return sanitized_html.to_owned();
    };

    detach_boilerplate(&mut document, best_id);

    document
        .tree
        .get(best_id)
        .and_then(ElementRef::wrap)
        .map_or_else(
            || sanitized_html.to_owned(),
            |el| el.inner_html().trim().to_owned(),
        )
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

/// Detaches `nav`/`aside`/`header`/`footer` nodes under `root`.
fn detach_boilerplate(document: &mut Html, root: NodeId) {
    let ids: Vec<NodeId> = {
        let Some(root_ref) = document.tree.get(root).and_then(ElementRef::wrap) else {
            return;
        };
        root_ref
            .descendants()
            .filter_map(ElementRef::wrap)
            .filter(|el| BOILERPLATE.contains(&el.value().name()))
            .map(|el| el.id())
            .collect()
    };
    for id in ids {
        if let Some(mut node) = document.tree.get_mut(id) {
            node.detach();
        }
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
    }

    #[test]
    fn boilerplate_inside_the_winner_is_removed() {
        let html = "<article>\
            <header><h1>Site chrome</h1></header>\
            <p>Body text one, long enough to score decisively here.</p>\
            <p>Body text two, also long enough to add to the score.</p>\
            <aside><p>promo promo promo</p></aside>\
            </article>";
        let main = extract_main_content(html);
        assert!(main.contains("Body text one"));
        assert!(!main.contains("Site chrome"));
        assert!(!main.contains("promo"));
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
}

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
/// or fragment, returning its inner HTML — wrapped in a `<div dir lang>`
/// when a direction or language applied to the winning container, so
/// they survive. Boilerplate sections inside the winning container are
/// removed. When nothing scores (bare text,
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
            |el| with_direction(el, el.inner_html().trim()),
        )
}

/// `inner` (the winner's inner HTML), wrapped in a `<div>` carrying the
/// `dir`/`lang` that applied to the winner — its own, else the nearest
/// ancestor's. Taking only the inner HTML would otherwise drop them, and
/// container-level direction (`<article dir="rtl">`) is how RTL content
/// usually arrives. Values are re-filtered here, so the wrapper can never
/// carry more than the sanitize gate admits.
fn with_direction(winner: ElementRef<'_>, inner: &str) -> String {
    let inherited = |name: &str| {
        std::iter::once(winner)
            .chain(winner.ancestors().filter_map(ElementRef::wrap))
            .find_map(|el| el.value().attr(name))
            .map(str::trim)
    };
    let dir = inherited("dir")
        .map(str::to_ascii_lowercase)
        .filter(|dir| matches!(dir.as_str(), "ltr" | "rtl" | "auto"));
    let lang = inherited("lang").filter(|lang| {
        !lang.is_empty()
            && lang.len() <= 35
            && lang.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    });
    if dir.is_none() && lang.is_none() {
        return inner.to_owned();
    }
    let dir = dir.map_or_else(String::new, |dir| format!(" dir=\"{dir}\""));
    let lang = lang.map_or_else(String::new, |lang| format!(" lang=\"{lang}\""));
    format!("<div{dir}{lang}>{inner}</div>")
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
    fn the_winners_direction_and_language_wrap_its_content() {
        let html = "<article dir=\"RTL\" lang=\"ar\"><h1>عنوان</h1>\
            <p>فقرة طويلة بما يكفي لتسجيل النقاط في المستخرج.</p></article>";
        let main = extract_main_content(html);
        assert!(
            main.starts_with("<div dir=\"rtl\" lang=\"ar\"><h1>عنوان</h1>"),
            "{main}"
        );
        assert!(main.ends_with("</div>"), "{main}");
    }

    #[test]
    fn direction_is_inherited_from_an_ancestor_of_the_winner() {
        let html = "<section dir=\"rtl\"><div><p>First paragraph of real text.</p>\
            <p>Second paragraph of real text.</p></div></section>";
        let main = extract_main_content(html);
        assert!(main.starts_with("<div dir=\"rtl\"><p>First"), "{main}");
    }

    #[test]
    fn hostile_direction_values_do_not_reach_the_wrapper() {
        let html = "<article dir=\"rtl&quot; onclick=&quot;x\" lang=\"en x\">\
            <p>Paragraph text long enough to score.</p></article>";
        let main = extract_main_content(html);
        assert!(main.starts_with("<p>"), "{main}");
    }

    #[test]
    fn unstructured_text_falls_back_to_the_body() {
        let main = extract_main_content("bare text, no elements at all");
        assert_eq!(main, "bare text, no elements at all");
    }
}

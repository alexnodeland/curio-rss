//! Readability extraction: find the main content, drop the page chrome.
//!
//! A deliberately small, deterministic, scraper-based port of the
//! readability idea — not a heuristic zoo. It only runs on feed items
//! that really are whole pages (see [`super::process`]); plain feed
//! fragments are sanitized and stored as-is.
//!
//! 1. **Chrome is detached first**: every `nav`, and every `header`,
//!    `footer` or `aside` that stands at page level. HTML scopes a
//!    `header`/`footer` to its nearest sectioning element, so the same
//!    elements inside an `article`, `section`, `blockquote`, `figure`,
//!    `details` or `td` — or inside `main` — are content: an article's
//!    or section's own heading block, a quote's `<footer>— Author</footer>`
//!    attribution, a figure credit. An `aside` is content inside the same
//!    containers except `main`, where it is as often the related-posts
//!    sidebar as an in-article callout.
//! 2. **The tightest wrapper around the prose wins**: candidates are the
//!    sectioning containers (`article`, `main`, `section`, `div`, and
//!    `body` as the floor), each scored by the text mass of its *direct*
//!    content children (paragraphs, headings, lists, quotes, code,
//!    figures, tables).
//! 3. **Content-bearing siblings are never dropped**: the winner widens
//!    to its parent, level by level up to `body`, whenever a sibling
//!    carries prose of its own — words outside links. An intro paragraph
//!    beside the body `div`, or a post split across two `section`s,
//!    survives whole. Link rows (share buttons, tag lists, pagination),
//!    even with `·`/`|` separators between the links, are no reason to
//!    widen.
//!
//! Widening never climbs out of an `article`: the element is the page's
//! own declaration of a complete composition, so a comment thread or a
//! cookie notice beside it stays out. Without an `article` the rule is
//! deliberately generous — prose beside the winner cannot be told from
//! page furniture by structure alone (the sanitizer has already dropped
//! `class`/`id` hints), and keeping a stray comment block is the lesser
//! failure than silently losing part of the post (#128).

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

/// Page chrome only at page level: inside a [`HEADER_FOOTER_SCOPES`]
/// element these are that element's own heading or closing block.
const HEADER_FOOTER: &[&str] = &["header", "footer"];

/// Containers that own the `header`/`footer` inside them: HTML's
/// sectioning content and sectioning roots that survive sanitizing,
/// plus `main`, whose header is the post's title block.
const HEADER_FOOTER_SCOPES: &[&str] = &[
    "article",
    "section",
    "main",
    "blockquote",
    "figure",
    "details",
    "td",
];

/// Containers whose `aside` is an in-content callout or pull quote.
/// Unlike [`HEADER_FOOTER_SCOPES`] this leaves out `main`: an `aside`
/// directly in `main` is as often the related-posts sidebar.
const ASIDE_SCOPES: &[&str] = &[
    "article",
    "section",
    "blockquote",
    "figure",
    "details",
    "td",
];

/// Extracts the main content of an **already sanitized** HTML document
/// or fragment, returning its inner HTML — wrapped in a `<div dir lang>`
/// when a direction or language applied to the winning container, so
/// they survive. Page chrome is removed, the tightest container around
/// the prose is chosen, then widened so no content-bearing sibling block
/// is lost. When nothing scores (bare text, unstructured markup), the
/// whole body passes through.
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

/// Whether an **already sanitized** fragment carries page chrome (a
/// `nav`, or a page-level `header`/`footer`/`aside`) — what separates a
/// whole page worth extracting from a feed fragment that merely arrived
/// wrapped in `<html>`/`<body>`. A `section`'s own `<header>` is not
/// chrome, so a wrapped post of headed sections stays a fragment.
#[must_use]
pub(crate) fn has_page_chrome(sanitized_html: &str) -> bool {
    Html::parse_fragment(sanitized_html)
        .root_element()
        .descendants()
        .filter_map(ElementRef::wrap)
        .any(is_chrome)
}

/// Removes the page chrome from an **already sanitized** fragment — the
/// same rule [`extract_main_content`] applies, for HTML whose main content
/// another extractor already chose (the full-page door). Returns the
/// input untouched when it carries no chrome.
#[must_use]
pub(crate) fn strip_page_chrome(sanitized_html: &str) -> String {
    if !has_page_chrome(sanitized_html) {
        return sanitized_html.to_owned();
    }
    let mut fragment = Html::parse_fragment(sanitized_html);
    detach_chrome(&mut fragment);
    fragment.root_element().inner_html().trim().to_owned()
}

/// `nav` anywhere; `header`/`footer` unless inside a
/// [`HEADER_FOOTER_SCOPES`] element; `aside` unless inside an
/// [`ASIDE_SCOPES`] element.
fn is_chrome(element: ElementRef<'_>) -> bool {
    let name = element.value().name();
    if ALWAYS_CHROME.contains(&name) {
        return true;
    }
    let scopes = if HEADER_FOOTER.contains(&name) {
        HEADER_FOOTER_SCOPES
    } else if name == "aside" {
        ASIDE_SCOPES
    } else {
        return false;
    };
    !element
        .ancestors()
        .filter_map(ElementRef::wrap)
        .any(|ancestor| scopes.contains(&ancestor.value().name()))
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
/// parent. The walk reaches `body`, so content several wrappers away (an
/// intro beside an outer page `div`) is still found — unless it meets an
/// `article` first: a self-contained composition is never widened past.
fn widen_over_content_siblings(document: &Html, winner: NodeId) -> NodeId {
    let mut top = winner;
    let Some(mut cursor) = document.tree.get(winner) else {
        return winner;
    };
    while !is_element_named(cursor, "body") && !is_element_named(cursor, "article") {
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

/// Whether a node holds words outside any link: text with at least one
/// alphanumeric character. Link rows (share buttons, tag lists,
/// pagination) don't count, even with `·`, `|` or `/` separators between
/// the links; a paragraph that merely contains a link does.
fn carries_prose(node: NodeRef<'_, Node>) -> bool {
    match node.value() {
        Node::Text(text) => text.chars().any(char::is_alphanumeric),
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
        let html = "<body><div>\
            <header><h1>Site chrome</h1></header>\
            <p>Body text one, long enough to score decisively here.</p>\
            <p>Body text two, also long enough to add to the score.</p>\
            <aside><p>promo promo promo</p></aside>\
            <nav><a href=\"https://e.com/next\">Next post</a></nav>\
            </div></body>";
        let main = extract_main_content(html);
        assert!(main.contains("Body text one"));
        assert!(!main.contains("Site chrome"));
        assert!(!main.contains("promo"));
        assert!(!main.contains("Next post"));
    }

    #[test]
    fn a_main_header_is_the_post_title_but_a_main_aside_is_a_sidebar() {
        let html = "<body><nav><a href=\"https://e.com/\">Home</a></nav><main>\
            <header><h1>Post title</h1><p>By A. Writer</p></header>\
            <p>Body text one, long enough to score decisively here.</p>\
            <p>Body text two, also long enough to add to the score.</p>\
            <aside><p>Related posts you might enjoy.</p></aside>\
            <footer><p>Posted in essays.</p></footer>\
            </main></body>";
        let main = extract_main_content(html);
        for kept in [
            "Post title",
            "By A. Writer",
            "Body text one",
            "Posted in essays.",
        ] {
            assert!(main.contains(kept), "lost {kept:?}: {main}");
        }
        assert!(!main.contains("Related posts"), "sidebar leaked: {main}");
        assert!(!main.contains("Home"), "nav leaked: {main}");
    }

    #[test]
    fn a_sections_own_header_and_aside_are_content() {
        let html = "<body><header><p>Example Blog</p></header>\
            <section><header><h2>Part one title</h2></header>\
            <p>Part one of the post, a little shorter than the second.</p>\
            <aside><p>A callout inside part one.</p></aside></section>\
            <section><header><h2>Part two title</h2></header>\
            <p>Part two of the post, which carries rather more text than part one does.</p>\
            </section></body>";
        let main = extract_main_content(html);
        for kept in [
            "Part one title",
            "Part two title",
            "A callout inside part one.",
            "Part one of the post",
            "Part two of the post",
        ] {
            assert!(main.contains(kept), "lost {kept:?}: {main}");
        }
        assert!(!main.contains("Example Blog"), "page header leaked: {main}");
    }

    #[test]
    fn widening_never_climbs_out_of_an_article() {
        let html = "<body><nav><a href=\"https://e.com/\">Home</a></nav>\
            <article><p>The story itself, long enough to win the scoring outright.</p>\
            <p>A second paragraph of the story.</p></article>\
            <div><p>Great post! Buy cheap pills.</p></div>\
            <div><p>We use cookies.</p></div></body>";
        let main = extract_main_content(html);
        assert!(main.contains("The story itself"), "{main}");
        assert!(!main.contains("cheap pills"), "comments leaked: {main}");
        assert!(!main.contains("cookies"), "cookie notice leaked: {main}");
    }

    #[test]
    fn a_section_inside_an_article_still_widens_to_the_article() {
        let html = "<body><article><p>An intro inside the article.</p>\
            <section><p>The longer section body that outscores the intro on its own.</p>\
            <p>And another one to make the section the clear top scorer.</p></section>\
            </article><div><p>A comment below the article.</p></div></body>";
        let main = extract_main_content(html);
        assert!(main.contains("An intro inside the article."), "{main}");
        assert!(main.contains("longer section body"), "{main}");
        assert!(!main.contains("A comment below"), "{main}");
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
    fn separated_link_rows_do_not_widen_the_winner() {
        for separator in [" · ", " | ", " / ", " — "] {
            let html = format!(
                "<div id=\"page\"><div id=\"content\">\
                 <p>Only this inner div holds the prose directly.</p>\
                 <p>So it stays the winner despite the share row.</p>\
                 </div><div><a href=\"https://e.com/share\">Share</a>{separator}\
                 <a href=\"https://e.com/tweet\">Tweet</a></div></div>"
            );
            let main = extract_main_content(&html);
            assert!(
                main.starts_with("<p>"),
                "widened over {separator:?}: {main}"
            );
            assert!(!main.contains("Share"), "{main}");
        }
    }

    #[test]
    fn strip_page_chrome_keeps_content_scoped_elements() {
        let html = "<div><nav><a href=\"https://e.com/blog\">Blog</a> › \
            <a href=\"https://e.com/tech\">Tech</a></nav>\
            <header><p>Site banner</p></header>\
            <article><header><h1>Title</h1><p>By someone</p></header>\
            <p>The body.</p></article></div>";
        let stripped = strip_page_chrome(html);
        assert!(!stripped.contains("Blog"), "breadcrumbs leaked: {stripped}");
        assert!(!stripped.contains("Site banner"), "{stripped}");
        for kept in ["Title", "By someone", "The body."] {
            assert!(stripped.contains(kept), "lost {kept:?}: {stripped}");
        }
    }

    #[test]
    fn strip_page_chrome_leaves_chrome_free_html_untouched() {
        let html = "<p>A  fragment</p> <section><header>own</header></section>";
        assert_eq!(strip_page_chrome(html), html);
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
        assert!(!has_page_chrome(
            "<section><header><h2>t</h2></header><aside>c</aside><p>x</p></section>"
        ));
        assert!(!has_page_chrome(
            "<main><header><h1>t</h1></header><p>x</p><footer>f</footer></main>"
        ));
        assert!(!has_page_chrome(
            "<details><summary>s</summary><header>h</header></details>"
        ));
        assert!(!has_page_chrome(
            "<table><tr><td><footer>cell note</footer></td></tr></table>"
        ));
        assert!(
            has_page_chrome("<main><p>x</p><aside>Related posts</aside></main>"),
            "an aside directly in main is a sidebar"
        );
    }
}

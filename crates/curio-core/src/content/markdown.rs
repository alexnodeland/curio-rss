//! HTML → `CommonMark` via htmd's DOM walk (D5).
//!
//! The sketch's regex converter — which destroyed every element spanning
//! lines — is banned; conversion is a real DOM traversal. Input is
//! expected to be sanitized already (this module is called on stored
//! content at export time), but the skip list drops any script/style/
//! embed remnant defensively anyway.
//!
//! `header`, `footer` and `aside` are *not* skipped: whether one of them
//! is page chrome depends on where it sits in the page, and deciding that
//! is the ingest pipeline's job. Both doors ([`super::process`] and
//! [`super::process_full_page`]) drop the page-level ones before storage,
//! so what reaches export is content — a quote's `<footer>` attribution,
//! an article's or section's own header, an in-article callout — and
//! converting it is the point (#128).
//!
//! `nav` is skipped wherever it sits: it is a block of links into the
//! source site (breadcrumbs, a menu, an in-page table of contents whose
//! `#anchors` mean nothing in a note), and articles stored before the
//! full-page door stripped chrome may still carry it.
//!
//! Footnotes become `CommonMark` footnotes (`[^1]` / `[^1]: …`) and other
//! in-page links keep only their text (see `footnotes`); a `pre`/`code`
//! `language-*` class becomes the fence's info string.
//!
//! htmd's walk recurses once per nesting level, and a stack overflow is an
//! abort no `catch_unwind` can stop — so the depth of a hostile feed's
//! markup must never reach it. Before the walk, every element nested
//! deeper than [`MAX_DEPTH`] is unwrapped (an iterative pass): its text and
//! childless elements (`img`, `br`) stay, in document order, and only the
//! surplus wrappers go (#244).

use std::cell::RefCell;
use std::rc::Rc;

use htmd::{HtmlToMarkdown, Node};
use markup5ever_rcdom::NodeData;

use super::{ContentError, INLINE_ELEMENTS, footnotes};

/// The deepest element nesting htmd is handed, counted from the document
/// root (`html` is 1, `body` 2). Real articles stay far below it. Here the
/// walk's heaviest shape (nested `ol > li`, ~4 KB of stack a level in a
/// debug build) needs about half of a 1 MB stack — the smallest one the
/// conversion runs on (a Windows main thread; tokio workers and test
/// threads get 2 MB). `walk_fits_a_1mb_stack_at_the_cap` pins that.
const MAX_DEPTH: usize = 128;

/// Converts an HTML fragment to `CommonMark`.
///
/// # Errors
///
/// [`ContentError::Markdown`] if the DOM walk fails to serialize
/// (practically unreachable for parseable HTML).
pub fn to_markdown(html: &str) -> Result<String, ContentError> {
    let builder = HtmlToMarkdown::builder().skip_tags(vec![
        "script", "style", "iframe", "object", "embed", "form", "nav", "noscript", "svg", "canvas",
    ]);
    let converter = footnotes::register(builder, html).build();
    let tree = converter.html_to_tree(html)?;
    cap_depth(&tree, MAX_DEPTH);
    Ok(converter.tree_to_markdown(&tree).trim().to_owned())
}

/// Bounds the element nesting under `root` to `max_depth` (plus one level
/// of childless elements): each element at `max_depth` keeps its subtree's
/// text and childless elements as direct children, in document order, and
/// loses every element between. Iterative throughout, so it holds for any
/// depth the parser builds.
fn cap_depth(root: &Rc<Node>, max_depth: usize) {
    let mut stack = vec![(Rc::clone(root), 0_usize)];
    while let Some((node, depth)) = stack.pop() {
        if depth >= max_depth {
            flatten_subtree(&node);
            continue;
        }
        for child in node.children.borrow().iter() {
            if matches!(child.data, NodeData::Element { .. }) {
                stack.push((Rc::clone(child), depth + 1));
            }
        }
    }
}

/// Replaces `node`'s children with the leaves of its subtree (text,
/// comments, childless elements), unwrapping every element that has
/// children. A space stands in for each unwrapped block (non-inline)
/// element, so the words of two flattened blocks never run together — the
/// same boundary rule as [`super::plain_text`].
fn flatten_subtree(node: &Rc<Node>) {
    let children = std::mem::take(&mut *node.children.borrow_mut());
    // One iterator per element being unwrapped, plus whether it was a block.
    let mut levels = vec![(children.into_iter(), false)];
    let mut flat: Vec<Rc<Node>> = Vec::new();
    let mut pending_space = false;
    while let Some((level, _)) = levels.last_mut() {
        let Some(child) = level.next() else {
            if let Some((_, block)) = levels.pop() {
                pending_space |= block;
            }
            continue;
        };
        if let NodeData::Element { name, .. } = &child.data {
            let grandchildren = std::mem::take(&mut *child.children.borrow_mut());
            if !grandchildren.is_empty() {
                let block = !INLINE_ELEMENTS.contains(&&*name.local);
                pending_space |= block;
                levels.push((grandchildren.into_iter(), block));
                continue;
            }
        }
        if std::mem::take(&mut pending_space) && !flat.is_empty() {
            flat.push(space(node));
        }
        child.parent.set(Some(Rc::downgrade(node)));
        flat.push(child);
    }
    *node.children.borrow_mut() = flat;
}

/// A single-space text node, child of `parent`.
fn space(parent: &Rc<Node>) -> Rc<Node> {
    let text = Node::new(NodeData::Text {
        contents: RefCell::new(" ".into()),
    });
    text.parent.set(Some(Rc::downgrade(parent)));
    text
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn multiline_paragraphs_survive() {
        let md = to_markdown("<p>one\ntwo\nthree</p><p>four</p>").unwrap();
        assert!(md.contains("one"));
        assert!(md.contains("four"));
        assert!(!md.contains('<'));
    }

    #[test]
    fn multiline_code_blocks_become_fences() {
        let md = to_markdown("<pre><code>fn main() {\n    let x = 1;\n}</code></pre>").unwrap();
        assert!(md.contains("```"), "expected a fence: {md}");
        assert!(md.contains("let x = 1;"));
    }

    #[test]
    fn links_and_images_convert() {
        let md = to_markdown(
            "<p><a href=\"https://e.com/a\">link</a> and \
             <img src=\"https://e.com/i.png\" alt=\"pic\"></p>",
        )
        .unwrap();
        assert!(md.contains("[link](https://e.com/a)"));
        assert!(md.contains("![pic](https://e.com/i.png)"));
    }

    #[test]
    fn footnotes_become_commonmark_footnotes() {
        let md = to_markdown(
            "<p>Claim<sup id=\"user-content-fnref1\"><a href=\"#user-content-fn1\">1</a></sup> and \
             more<sup id=\"user-content-fnref2\"><a href=\"#user-content-fn2\">2</a></sup>.</p>\
             <hr><ol>\
             <li id=\"user-content-fn1\"><p>First note. <a href=\"#user-content-fnref1\">↩</a></p>\
             <p>Second paragraph.</p></li>\
             <li id=\"user-content-fn2\"><p>See <a href=\"https://e.com/src\">source</a>. \
             <a href=\"#user-content-fnref2\">↩</a></p></li></ol>",
        )
        .unwrap();
        assert!(md.contains("Claim[^1] and more[^2]."), "{md}");
        assert!(
            md.contains(
                "[^1]: First note.\n\n    Second paragraph.\n[^2]: See [source](https://e.com/src)."
            ),
            "{md}"
        );
        assert!(!md.contains('↩'), "back-reference leaked: {md}");
        assert!(!md.contains("](#"), "dead in-page link: {md}");
    }

    #[test]
    fn other_in_page_links_keep_their_text() {
        let md =
            to_markdown("<p>Jump to <a href=\"#user-content-part-2\">part two</a>.</p>").unwrap();
        assert_eq!(md, "Jump to part two.");
    }

    #[test]
    fn ordinary_ordered_lists_are_untouched_in_a_footnoted_doc() {
        let md = to_markdown(
            "<ol start=\"3\"><li>three</li><li>four</li></ol>\
             <p>x<a href=\"#user-content-fn1\">1</a></p><ol><li id=\"user-content-fn1\">n</li></ol>",
        )
        .unwrap();
        assert!(md.contains("3.  three\n4.  four"), "{md}");
        assert!(md.contains("x[^1]"), "{md}");
        assert!(md.contains("[^1]: n"), "{md}");
    }

    #[test]
    fn deeply_nested_lists_in_a_footnoted_doc_are_walked_once() {
        // Each list used to be walked twice (once to look for definitions,
        // once by htmd), so export cost doubled per nesting level: 64
        // levels would never finish. Walked once, this is instant.
        const DEPTH: usize = 64;
        let html = format!(
            "<p>x<a href=\"#user-content-fn1\">1</a></p>{}deep{}\
             <ol><li id=\"user-content-fn1\">n</li></ol>",
            "<ol><li>".repeat(DEPTH),
            "</li></ol>".repeat(DEPTH),
        );
        let md = to_markdown(&html).unwrap();
        assert!(md.contains("deep"), "{md}");
        assert!(md.contains("x[^1]"), "{md}");
        assert!(md.contains("[^1]: n"), "{md}");
    }

    #[test]
    fn in_page_links_inside_a_definition_keep_their_text() {
        let md = to_markdown(
            "<h2 id=\"user-content-part-2\">Part two</h2>\
             <p>x<sup id=\"user-content-fnref1\"><a href=\"#user-content-fn1\">1</a></sup></p>\
             <ol><li id=\"user-content-fn1\">See <a href=\"#user-content-part-2\">part two</a>. \
             <a href=\"#user-content-fnref1\">↩</a></li></ol>",
        )
        .unwrap();
        assert!(md.contains("[^1]: See part two."), "{md}");
        assert!(!md.contains('↩'), "back-reference leaked: {md}");
    }

    #[test]
    fn code_fence_keeps_its_language() {
        let md = to_markdown("<pre><code class=\"language-rust\">let x = 1;</code></pre>").unwrap();
        assert!(md.starts_with("```rust\n"), "{md}");
    }

    #[test]
    fn sectioning_elements_left_in_stored_content_convert() {
        let md = to_markdown(
            "<article><header><h1>Title</h1></header><p>Body.</p>\
             <aside><p>Callout.</p></aside>\
             <blockquote><p>Quote.</p><footer>— Author</footer></blockquote></article>",
        )
        .unwrap();
        for kept in ["# Title", "Body.", "Callout.", "> Quote.", "— Author"] {
            assert!(md.contains(kept), "lost {kept:?}: {md}");
        }
    }

    #[test]
    fn nav_is_never_exported() {
        let md = to_markdown(
            "<article><nav><a href=\"https://e.com/blog\">Blog</a> › \
             <a href=\"https://e.com/tech\">Tech</a></nav>\
             <header><h1>Title</h1></header><p>Body.</p></article>",
        )
        .unwrap();
        assert!(!md.contains("Blog"), "breadcrumbs exported: {md}");
        assert!(md.contains("# Title"), "{md}");
        assert!(md.contains("Body."), "{md}");
    }

    /// Runs `f` on a fresh thread with a `stack`-byte stack — the size of
    /// the thread an export really runs on.
    fn on_stack<T: Send + 'static>(stack: usize, f: impl FnOnce() -> T + Send + 'static) -> T {
        std::thread::Builder::new()
            .stack_size(stack)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap()
    }

    /// `to_markdown`'s conversion with the cap at `max_depth` instead.
    fn convert_capped(html: &str, max_depth: usize) -> String {
        let converter = HtmlToMarkdown::new();
        let tree = converter.html_to_tree(html).unwrap();
        cap_depth(&tree, max_depth);
        converter.tree_to_markdown(&tree)
    }

    /// The deepest element nesting under `root` (`root` itself is 0).
    fn max_depth_of(root: &Rc<Node>) -> usize {
        let mut deepest = 0;
        let mut stack = vec![(Rc::clone(root), 0)];
        while let Some((node, depth)) = stack.pop() {
            deepest = deepest.max(depth);
            for child in node.children.borrow().iter() {
                if matches!(child.data, NodeData::Element { .. }) {
                    stack.push((Rc::clone(child), depth + 1));
                }
            }
        }
        deepest
    }

    #[test]
    fn deep_nesting_converts_on_a_2mb_stack() {
        // #244: ~1000 nested divs overflowed a 2 MB stack (a tokio worker,
        // a test thread) in htmd's recursive walk and aborted the process.
        const LEVELS: usize = 2_000;
        let html = format!(
            "<p>before</p>{}deep{}<p>after</p>",
            "<div>".repeat(LEVELS),
            "</div>".repeat(LEVELS)
        );
        let md = on_stack(2 << 20, move || to_markdown(&html).unwrap());
        assert_eq!(md, "before\n\ndeep\n\nafter");
    }

    #[test]
    fn deep_tables_convert_on_a_2mb_stack() {
        // Table cells are parser scope boundaries, so 40 000 levels parse
        // in linear time — and still have to clear the cap to convert.
        const LEVELS: usize = 10_000;
        let html = format!(
            "{}deep{}",
            "<table><tr><td>".repeat(LEVELS),
            "</td></tr></table>".repeat(LEVELS)
        );
        let md = on_stack(2 << 20, move || to_markdown(&html).unwrap());
        assert!(md.contains("deep"), "{md}");
    }

    #[test]
    fn walk_fits_a_1mb_stack_at_the_cap() {
        // The heaviest shape htmd walks — nested ordered lists — capped at
        // MAX_DEPTH must still fit the smallest stack an export runs on (a
        // 1 MB Windows main thread), with room to spare: raising MAX_DEPTH
        // past what that stack holds fails here, not in a user's export.
        const PAIRS: usize = 600;
        let html = format!(
            "{}deep{}",
            "<ol><li>".repeat(PAIRS),
            "</li></ol>".repeat(PAIRS)
        );
        let md = on_stack(1 << 20, move || to_markdown(&html).unwrap());
        assert!(md.contains("deep"), "{md}");
    }

    #[test]
    fn the_cap_bounds_the_depth_htmd_walks() {
        let converter = HtmlToMarkdown::new();
        let html = format!(
            "{}<img src=\"x.png\">{}",
            "<div>".repeat(50),
            "</div>".repeat(50)
        );
        let tree = converter.html_to_tree(&html).unwrap();
        assert!(max_depth_of(&tree) > 50);
        cap_depth(&tree, 10);
        // Elements at the cap keep their childless children (the `img`).
        assert_eq!(max_depth_of(&tree), 11);
    }

    #[test]
    fn shallow_markup_is_left_alone() {
        let html = "<div><blockquote><p>a <b>b</b></p><ul><li>c</li></ul></blockquote></div>";
        assert_eq!(
            convert_capped(html, MAX_DEPTH),
            HtmlToMarkdown::new().convert(html).unwrap()
        );
    }

    #[test]
    fn flattened_text_keeps_its_order_and_word_boundaries() {
        // `html` 1, `body` 2, the outer div 3: everything inside it flattens.
        let md = convert_capped(
            "<div><div><p>one</p><p>two <b>bo</b>ld</p></div>\
             <ul><li>three</li><li>four<br>five</li></ul>\
             <img src=\"https://e.com/i.png\" alt=\"pic\"><!-- note --></div>",
            3,
        );
        // Blocks are word boundaries; inline elements are not; childless
        // elements (`br`, `img`) survive the unwrapping.
        assert_eq!(
            md,
            "one two bold three four  \nfive ![pic](https://e.com/i.png)"
        );
    }

    #[test]
    fn headings_and_lists_convert() {
        let md = to_markdown("<h2>title</h2><ul><li>a</li><li>b</li></ul>").unwrap();
        assert!(md.contains("## title"));
        let has_bullet = md
            .lines()
            .any(|line| matches!(line.trim_start().chars().next(), Some('-' | '*' | '+')));
        assert!(has_bullet, "list lost: {md}");
    }
}

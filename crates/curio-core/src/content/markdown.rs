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
//! in-page links keep only their text (see `footnotes`). `pre` and
//! `code` have their own handlers (see `code`): code text is written
//! unescaped, so it must land where a renderer reads it as code — a fence
//! longer than any backtick run, or a single-line inline span — and never
//! as live raw HTML (#245). A `pre`/`code` `language-*` class becomes the
//! fence's info string.
//!
//! htmd's walk recurses once per nesting level, and a stack overflow is an
//! abort no `catch_unwind` can stop — so the depth of a hostile feed's
//! markup must never reach it. Before the walk, every element nested
//! deeper than [`MAX_DEPTH`] is unwrapped (an iterative pass): its text and
//! childless elements (`img`, `br`) stay, in document order, and only the
//! surplus wrappers go (#244) — except what the skip list names, which goes
//! whole, exactly as an uncapped walk would drop it.

use std::cell::RefCell;
use std::rc::Rc;

use htmd::{HtmlToMarkdown, Node};
use markup5ever_rcdom::NodeData;

use super::{ContentError, INLINE_ELEMENTS, code, footnotes};

/// The deepest element nesting htmd is handed, counted from the document
/// root (`html` is 1, `body` 2). Real articles stay far below it. Here the
/// walk's heaviest shape (nested `ol > li`, ~4 KB of stack a level in a
/// debug build) needs about half of a 1 MB stack — the smallest one the
/// conversion runs on (a Windows main thread; tokio workers and test
/// threads get 2 MB). `walk_fits_a_1mb_stack_at_the_cap` pins that.
const MAX_DEPTH: usize = 128;

/// Elements dropped with their whole subtree — by htmd's walk above the
/// cap and by [`flatten_subtree`] below it, so the cap never exports text
/// an uncapped walk would not.
const SKIP_TAGS: &[&str] = &[
    "script", "style", "iframe", "object", "embed", "form", "nav", "noscript", "svg", "canvas",
];

/// Converts an HTML fragment to `CommonMark`.
///
/// # Errors
///
/// [`ContentError::Markdown`] if the DOM walk fails to serialize
/// (practically unreachable for parseable HTML).
pub fn to_markdown(html: &str) -> Result<String, ContentError> {
    let converter = converter(html);
    let tree = converter.html_to_tree(html)?;
    cap_depth(&tree, MAX_DEPTH);
    Ok(converter.tree_to_markdown(&tree).trim().to_owned())
}

/// The converter for `html`: the skip list plus the footnote and code
/// handlers.
fn converter(html: &str) -> HtmlToMarkdown {
    let builder = HtmlToMarkdown::builder().skip_tags(SKIP_TAGS.to_vec());
    code::register(footnotes::register(builder, html)).build()
}

/// Bounds the element nesting under `root` to `max_depth` (plus one level
/// of childless elements, and the item or cell a list or table holds them
/// in): each element at `max_depth` keeps its subtree's text and childless
/// elements as direct children, in document order, and loses every element
/// between. Iterative throughout, so it holds for any depth the parser
/// builds.
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
/// children and dropping every [`SKIP_TAGS`] element whole. A space stands
/// in for each unwrapped or dropped block (non-inline) element, so a block
/// boundary never joins two words — the same rule as [`super::plain_text`].
/// When `node` is inline, one stands at either end of the run too: there
/// it is all that separates the run from `node`'s siblings (a block `node`
/// is a boundary itself, and a space there would linger in the markdown).
///
/// A list (`ol`, `ul`) keeps the run as its one item and a `table` as its
/// one cell, the shapes htmd renders a run in: it joins an `ol`'s children
/// bare, one at a time (losing every space between the run's pieces), and
/// gives a `table` without cells no block boundary.
fn flatten_subtree(node: &Rc<Node>) {
    let (inline, item) = match &node.data {
        NodeData::Element { name, .. } => (
            INLINE_ELEMENTS.contains(&&*name.local),
            match &*name.local {
                "ol" | "ul" => element_like(node, "li"),
                "table" => element_like(node, "td"),
                _ => None,
            },
        ),
        _ => (false, None),
    };
    let holder = item.as_ref().unwrap_or(node);
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
            let block = !INLINE_ELEMENTS.contains(&&*name.local);
            if SKIP_TAGS.contains(&&*name.local) {
                pending_space |= block;
                continue;
            }
            let grandchildren = std::mem::take(&mut *child.children.borrow_mut());
            if !grandchildren.is_empty() {
                pending_space |= block;
                levels.push((grandchildren.into_iter(), block));
                continue;
            }
        }
        if std::mem::take(&mut pending_space) && (inline || !flat.is_empty()) {
            flat.push(space(holder));
        }
        child.parent.set(Some(Rc::downgrade(holder)));
        flat.push(child);
    }
    if pending_space && inline {
        flat.push(space(holder));
    }
    match item {
        Some(item) if !flat.is_empty() => {
            *item.children.borrow_mut() = flat;
            item.parent.set(Some(Rc::downgrade(node)));
            node.children.borrow_mut().push(item);
        }
        Some(_) => {}
        None => *node.children.borrow_mut() = flat,
    }
}

/// An empty `tag` element in `element`'s namespace (`None` if `element`
/// is not one).
fn element_like(element: &Rc<Node>, tag: &str) -> Option<Rc<Node>> {
    let NodeData::Element { name, .. } = &element.data else {
        return None;
    };
    let mut name = name.clone();
    name.local = tag.into();
    Some(Node::new(NodeData::Element {
        name,
        attrs: RefCell::default(),
        template_contents: RefCell::default(),
        mathml_annotation_xml_integration_point: false,
    }))
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

    /// The three #245 rows: escaped markup inside a `<pre>` or a
    /// block-spanning `<code>` stays code, never live raw HTML.
    #[test]
    fn escaped_markup_in_a_bare_pre_is_fenced() {
        assert_eq!(
            to_markdown("<pre>&lt;img src=x onerror=alert(1)&gt;</pre>").unwrap(),
            "```\n<img src=x onerror=alert(1)>\n```"
        );
        assert_eq!(
            to_markdown("<pre>&lt;script&gt;alert(1)&lt;/script&gt;</pre>").unwrap(),
            "```\n<script>alert(1)</script>\n```"
        );
    }

    #[test]
    fn a_block_spanning_code_is_one_inline_span() {
        assert_eq!(
            to_markdown("<code>&lt;script&gt;alert(1)&lt;/script&gt;<p>A</p></code>").unwrap(),
            "`<script>alert(1)</script> A`"
        );
        assert_eq!(
            to_markdown("<p><code>a\n\n&lt;b&gt;\r- &lt;i&gt;</code></p>").unwrap(),
            "`a <b> - <i>`"
        );
    }

    #[test]
    fn the_fence_outgrows_every_backtick_run() {
        let md = to_markdown("<pre>```\n&lt;script&gt;\n````</pre>").unwrap();
        assert_eq!(md, "`````\n```\n<script>\n````\n`````");
        let md = to_markdown("<pre><code>`x`</code></pre>").unwrap();
        assert_eq!(md, "```\n`x`\n```");
    }

    #[test]
    fn a_span_outgrows_every_backtick_run_and_pads_edge_backticks() {
        assert_eq!(to_markdown("<code>a`b</code>").unwrap(), "``a`b``");
        assert_eq!(to_markdown("<code>``x</code>").unwrap(), "``` ``x ```");
        assert_eq!(to_markdown("<code> </code>").unwrap(), "");
    }

    #[test]
    fn adjacent_spans_never_merge_their_delimiters() {
        // Unseparated, `a` + ` ``<x> ` would read as "`a```` ``<x> ```":
        // the merged run leaves every delimiter unpaired and `<x>` live.
        let md = to_markdown("<p><code>a</code><span><code>``&lt;x&gt;</code></span></p>").unwrap();
        assert_eq!(md, "`a` ``` ``<x> ```");
        let md = to_markdown("<p>x <code>a</code> <code>b</code></p>").unwrap();
        assert_eq!(md, "x `a` `b`");
        let md = to_markdown("<p>x<br><code>a</code></p>").unwrap();
        assert!(md.ends_with("\n`a`"), "{md:?}");
    }

    #[test]
    fn a_block_that_renders_nothing_still_separates_adjacent_spans() {
        // An empty block between two spans renders as nothing: its tag
        // alone must not count as a line break, or the delimiters merge.
        for separator in [
            "<pre></pre>",
            "<pre>\n</pre>",
            "<ul></ul>",
            "<table></table>",
        ] {
            let html = format!(
                "<div><code>a</code>{separator}<code>``&lt;script&gt;alert(1)&lt;/script&gt;</code></div>"
            );
            assert_eq!(
                to_markdown(&html).unwrap(),
                "`a` ``` ``<script>alert(1)</script> ```",
                "{html}"
            );
        }
        // A block that did render ends its line: no gap is needed.
        let md = to_markdown("<div><code>a</code><p>p</p><code>``&lt;x&gt;</code></div>").unwrap();
        assert_eq!(md, "`a`\n\np\n\n``` ``<x> ```");
        let md = to_markdown("<div><pre>p</pre><code>``&lt;x&gt;</code></div>").unwrap();
        assert_eq!(md, "```\np\n```\n\n``` ``<x> ```");
    }

    #[test]
    fn every_one_of_many_sibling_spans_is_judged_by_its_own_neighbour() {
        // Positions among siblings are indexed once per parent (the walk
        // stays linear), and each span still sees the sibling before it.
        const PAIRS: usize = 300;
        let html = format!(
            "<p>{}</p>",
            "<code>a</code><b></b><code>``b</code> t<code>c</code>. ".repeat(PAIRS)
        );
        let md = to_markdown(&html).unwrap();
        assert_eq!(md, "`a` ``` ``b ``` t`c`. ".repeat(PAIRS).trim_end());
    }

    #[test]
    fn a_span_after_an_inline_pre_keeps_its_distance() {
        // Where a `<pre>` becomes an inline span, it is no block boundary
        // to the span after it.
        for (html, expected) in [
            (
                "<em><pre>&lt;script&gt;alert(1)&lt;/script&gt;</pre><code>``x</code></em>",
                "*`<script>alert(1)</script>` ``` ``x ```*",
            ),
            (
                "<h2>T <pre>&lt;script&gt;alert(1)&lt;/script&gt;</pre><code>``x</code></h2>",
                "## T `<script>alert(1)</script>` ``` ``x ```",
            ),
            (
                "<table><tr><td><pre>&lt;x&gt;</pre><code>``y</code></td></tr></table>",
                "`<x>` ``` ``y ```",
            ),
            (
                "<a href=\"https://e.com/\"><pre>&lt;x&gt;</pre><code>``y</code></a>",
                "[`<x>` ``` ``y ```](https://e.com/)",
            ),
        ] {
            assert_eq!(to_markdown(html).unwrap(), expected, "{html}");
        }
        // A block flattened into an inline line is no boundary either.
        let md = to_markdown("<em><code>a</code><p><code>``x</code></p></em>").unwrap();
        assert_eq!(md, "*`a`\n\n ``` ``x ```*");
    }

    #[test]
    fn a_definition_that_opens_with_code_starts_below_its_label() {
        // On the label line, a fence would be inline text to a renderer
        // without footnotes, and its code text live raw HTML.
        let md = to_markdown(
            "<p>x<a href=\"#user-content-fn1\">1</a></p><ol><li id=\"user-content-fn1\">\
             <pre>&lt;script&gt;alert(1)&lt;/script&gt;</pre></li></ol>",
        )
        .unwrap();
        assert_eq!(
            md,
            "x[^1]\n\n[^1]:\n\n    ```\n    <script>alert(1)</script>\n    ```"
        );
        let md = to_markdown(
            "<p>x<a href=\"#user-content-fn1\">1</a></p><ol><li id=\"user-content-fn1\">\
             <blockquote><p>q</p><pre>&lt;x&gt;</pre></blockquote></li></ol>",
        )
        .unwrap();
        assert_eq!(
            md,
            "x[^1]\n\n[^1]:\n\n    > q\n    >\n    > ```\n    > <x>\n    > ```"
        );
        // Text first: the label line keeps it, and the fence follows a
        // blank line as before.
        let md = to_markdown(
            "<p>x<a href=\"#user-content-fn1\">1</a></p><ol><li id=\"user-content-fn1\">\
             <p>see</p><pre>&lt;x&gt;</pre></li></ol>",
        )
        .unwrap();
        assert_eq!(md, "x[^1]\n\n[^1]: see\n\n    ```\n    <x>\n    ```");
    }

    #[test]
    fn a_pre_that_cannot_stand_on_its_own_lines_is_a_span() {
        for (html, expected) in [
            ("<em><pre>&lt;x&gt;\ny</pre></em>", "*`<x> y`*"),
            ("<h2>T <pre>&lt;x&gt;</pre></h2>", "## T `<x>`"),
            (
                "<a href=\"https://e.com/\"><pre>&lt;x&gt;</pre></a>",
                "[`<x>`](https://e.com/)",
            ),
        ] {
            assert_eq!(to_markdown(html).unwrap(), expected, "{html}");
        }
    }

    #[test]
    fn a_pre_in_a_list_or_quote_stays_fenced() {
        let md = to_markdown("<ul><li><pre>&lt;x&gt;\ny</pre></li></ul>").unwrap();
        assert_eq!(md, "*   ```\n    <x>\n    y\n    ```");
        let md = to_markdown("<blockquote><pre>&lt;x&gt;</pre></blockquote>").unwrap();
        assert_eq!(md, "> ```\n> <x>\n> ```");
    }

    #[test]
    fn pre_text_keeps_breaks_and_block_lines() {
        let md = to_markdown("<pre>a<br>b<p>c</p>d</pre>").unwrap();
        assert_eq!(md, "```\na\nb\nc\nd\n```");
        assert_eq!(to_markdown("<pre>\n\n</pre>").unwrap(), "");
        let md = to_markdown("<pre>a&#13;&lt;x&gt;\r\nb</pre>").unwrap();
        assert_eq!(md, "```\na\n<x>\nb\n```");
        // The markdown property's minimal find against htmd's own handler:
        // block children used to turn the `pre` into raw HTML.
        let md = to_markdown("<pre><p>&lt;script&gt;alert(1)&lt;/script&gt;</p></pre>").unwrap();
        assert_eq!(md, "```\n<script>alert(1)</script>\n```");
    }

    #[test]
    fn code_fence_keeps_its_language() {
        let md = to_markdown("<pre><code class=\"language-rust\">let x = 1;</code></pre>").unwrap();
        assert!(md.starts_with("```rust\n"), "{md}");
        // A bare `<pre>` carries its own language; one that could end the
        // info string is dropped.
        let md = to_markdown("<pre class=\"language-c++\">x</pre>").unwrap();
        assert_eq!(md, "```c++\nx\n```");
        let md = to_markdown("<pre class=\"language-a`b\">x</pre>").unwrap();
        assert_eq!(md, "```\nx\n```");
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

    /// `to_markdown` with the cap at `max_depth` instead (`None`: no cap).
    fn convert_capped(html: &str, max_depth: Option<usize>) -> String {
        let converter = converter(html);
        let tree = converter.html_to_tree(html).unwrap();
        if let Some(max_depth) = max_depth {
            cap_depth(&tree, max_depth);
        }
        converter.tree_to_markdown(&tree).trim().to_owned()
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

    /// A list whose `li`s sit `wrappers + 4` deep (`html` 1, `body` 2, the
    /// wrapping divs, then `ul` and `li`), inside a footnoted document.
    fn list_under(wrappers: usize) -> String {
        format!(
            "<p>x<a href=\"#user-content-fn1\">1</a></p>{}<ul><li>a</li><li>b</li></ul>{}\
             <ol><li id=\"user-content-fn1\">n</li></ol>",
            "<div>".repeat(wrappers),
            "</div>".repeat(wrappers),
        )
    }

    #[test]
    fn markup_up_to_the_cap_exports_as_if_uncapped() {
        // The `li`s at MAX_DEPTH keep their structure, so the export is
        // exactly the uncapped walk's.
        let html = list_under(MAX_DEPTH - 4);
        let md = to_markdown(&html).unwrap();
        assert_eq!(md, convert_capped(&html, None));
        assert!(md.contains("a\n*   b"), "{md}");
        assert!(md.contains("x[^1]") && md.contains("[^1]: n"), "{md}");
        // One level deeper the `li`s are past the cap and unwrap.
        let md = to_markdown(&list_under(MAX_DEPTH - 3)).unwrap();
        assert!(md.contains("a b"), "{md}");
    }

    #[test]
    fn skipped_elements_stay_skipped_below_the_cap() {
        // `nav` survives sanitization; the uncapped walk drops it whole, and
        // so must the flattening when the cap lands on the div around it.
        let html = "<div><nav><p>menu</p></nav>body<svg><text>shape</text></svg></div>";
        assert_eq!(convert_capped(html, None), "body");
        assert_eq!(convert_capped(html, Some(3)), "body");
    }

    #[test]
    fn an_inline_element_at_the_cap_keeps_its_outer_word_boundaries() {
        // `html` 1, `body` 2, the div 3, the `em` 4: the blockquote inside
        // the `em` is a boundary on both sides, even once unwrapped.
        let html = "<div>alpha<em><blockquote>beta</blockquote></em>gamma</div>";
        assert_eq!(convert_capped(html, Some(4)), "alpha *beta* gamma");
    }

    #[test]
    fn flattened_text_keeps_its_order_and_word_boundaries() {
        // `html` 1, `body` 2, the outer div 3: everything inside it flattens.
        let md = convert_capped(
            "<div><div><p>one</p><p>two <b>bo</b>ld</p></div>\
             <ul><li>three</li><li>four<br>five</li></ul>\
             <img src=\"https://e.com/i.png\" alt=\"pic\"><!-- note --></div>",
            Some(3),
        );
        // Blocks are word boundaries; inline elements are not; childless
        // elements (`br`, `img`) survive the unwrapping.
        assert_eq!(
            md,
            "one two bold three four  \nfive ![pic](https://e.com/i.png)"
        );
    }

    #[test]
    fn a_list_at_the_cap_keeps_its_run_as_one_item() {
        // htmd joins an `ol`'s children without a separator, so a bare run
        // straight in it would lose every space between its pieces.
        let html = "<ol><li>one</li><li>two <b>bo</b>ld<div>three</div></li></ol>";
        assert_eq!(convert_capped(html, Some(3)), "1.  one two bold three");
        let html = "<ul><li>one</li><li>two</li></ul>";
        assert_eq!(convert_capped(html, Some(3)), "*   one two");
        // A list whose run is all skipped content keeps no empty item.
        assert_eq!(
            convert_capped("<p>a</p><ol><li><nav>menu</nav></li></ol>", Some(3)),
            "a"
        );
    }

    #[test]
    fn a_table_at_the_cap_keeps_its_run_as_one_cell() {
        // A table of bare text would run into what follows it.
        let html = "<div><table><tr><td>one<div>two</div></td></tr></table>three</div>";
        assert_eq!(convert_capped(html, Some(4)), "one two\n\nthree");
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

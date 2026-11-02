//! The depth cap on HTML handed to htmd (#244).
//!
//! htmd's walk recurses once per nesting level, and a stack overflow is an
//! abort no `catch_unwind` can stop — so the depth of a hostile feed's
//! markup must never reach it. Before the walk, every element nested
//! deeper than [`MAX_DEPTH`] is unwrapped (an iterative pass): its text and
//! childless elements (`img`, `br`) stay, in document order, and only the
//! surplus wrappers go — except what the skip list names, which goes whole,
//! exactly as an uncapped walk would drop it.

use std::cell::RefCell;
use std::rc::Rc;

use htmd::Node;
use markup5ever_rcdom::NodeData;

use super::INLINE_ELEMENTS;
use super::markdown::SKIP_TAGS;

/// The deepest element nesting htmd is handed, counted from the document
/// root (`html` is 1, `body` 2). Real articles stay far below it. Here the
/// walk's heaviest shape (nested `ol > li`, ~4 KB of stack a level in a
/// debug build) needs about half of a 1 MB stack — the smallest one the
/// conversion runs on (a Windows main thread; tokio workers and test
/// threads get 2 MB). `walk_fits_a_1mb_stack_at_the_cap` pins that.
pub(super) const MAX_DEPTH: usize = 128;

/// Bounds the element nesting under `root` to `max_depth` (plus one level
/// of childless elements, and the item or cell a list or table holds them
/// in): each element at `max_depth` keeps its subtree's text and childless
/// elements as direct children, in document order, and loses every element
/// between. Iterative throughout, so it holds for any depth the parser
/// builds.
pub(super) fn cap_depth(root: &Rc<Node>, max_depth: usize) {
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
    use htmd::HtmlToMarkdown;

    use crate::content::markdown::converter;
    use crate::content::to_markdown;

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
}

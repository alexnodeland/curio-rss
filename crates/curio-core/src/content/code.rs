//! `<pre>` and `<code>` in exported markdown (#245).
//!
//! Code is the one place where markdown stops escaping: a renderer shows
//! the text of a fenced block or an inline span literally, so the text
//! goes in unescaped — `&lt;T&gt;` is written as `<T>`. That is only
//! inert if the renderer really does read the result as code. htmd's own
//! handlers do not make sure of it: a `<pre>` without a sole `<code>`
//! child became bare text (its decoded `<img onerror>` live raw HTML), and
//! a `<code>` whose text spans a blank line or block children became
//! backticks a renderer does not pair, with the tag between them live.
//!
//! So both handlers here build their output from the element's text alone
//! and never hand it back to htmd's walk (which would also leave it
//! unescaped: htmd stops escaping anywhere under a `pre` or `code`):
//!
//! - a `<pre>`, with or without a `<code>` child, is a fenced block whose
//!   fence is longer than any backtick run in the text, so no line of it
//!   can close the block — where a fence can stand on its own lines. Under
//!   an inline element, a heading or a table cell it cannot, and the text
//!   becomes an inline span instead;
//! - a `<code>` outside a `<pre>` is an inline span on a single line (its
//!   whitespace collapsed, as a browser shows it), delimited by a backtick
//!   run longer than any in the text. Nothing on one line can break a
//!   span open; a space keeps it from touching an earlier span, whose
//!   delimiters it would otherwise merge with. What was rendered before
//!   it is judged conservatively (see `before`): an earlier element earns
//!   the space unless what it actually rendered ended a line — a block
//!   tag alone proves nothing, since an empty `<pre>`, `<ul>` or `<table>`
//!   renders as nothing and a `<pre>` under an inline element is itself a
//!   span — and only the start of a block that really stands on lines of
//!   its own counts as a fresh line.
//!
//! A `language-*` class on the `code` (or else the `pre`) becomes the
//! fence's info string.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};

use htmd::{
    Element, HtmlToMarkdownBuilder, Node,
    element_handler::{HandlerResult, Handlers},
};
use markup5ever_rcdom::NodeData;

/// Registers the `pre` and `code` handlers on `builder`, plus a recorder
/// over the other block elements: it hands each to the handler it would
/// have had (htmd's, or `footnotes`') and notes whether what came back
/// ended a line, which is how a later span knows it starts one.
///
/// The handlers share a [`Tree`] of lookups keyed by node address, so a
/// converter built from `builder` converts one document only
/// (`to_markdown` builds one per call, as `footnotes` already requires).
pub(super) fn register(builder: HtmlToMarkdownBuilder) -> HtmlToMarkdownBuilder {
    let tree = Arc::new(Mutex::new(Tree::default()));
    let for_blocks = Arc::clone(&tree);
    let for_pre = Arc::clone(&tree);
    let recorded = BLOCK_BOUNDARIES
        .iter()
        .copied()
        .filter(|&name| name != "pre")
        .collect();
    builder
        .add_handler(
            recorded,
            move |handlers: &dyn Handlers, element: Element<'_>| {
                let key = address(element.node);
                // The lock is taken only once the block has rendered: its
                // children's own handlers take it too.
                let rendered = handlers.fallback(element);
                if rendered
                    .as_ref()
                    .is_some_and(|rendered| rendered.content.ends_with('\n'))
                {
                    lock(&for_blocks).ended_lines.insert(key);
                }
                rendered
            },
        )
        .add_handler(
            vec!["pre"],
            move |_: &dyn Handlers, element: Element<'_>| Some(pre(&element, &mut lock(&for_pre))),
        )
        .add_handler(
            vec!["code"],
            move |_: &dyn Handlers, element: Element<'_>| {
                Some(inline_code(element.node, &mut lock(&tree)))
            },
        )
}

/// The shared [`Tree`]; a poisoned lock still holds usable lookups.
fn lock(tree: &Mutex<Tree>) -> std::sync::MutexGuard<'_, Tree> {
    tree.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the converter's `skip_tags` drops: never part of a code text either.
const SKIPPED: &[&str] = &[
    "script", "style", "iframe", "object", "embed", "form", "nav", "noscript", "svg", "canvas",
];

/// Ancestors a fenced block can sit in: htmd renders each of them as
/// whole lines (block-handler tags, list items, block quotes), so the
/// fence still opens and closes on lines of its own.
const FENCE_CONTAINERS: &[&str] = &[
    "html",
    "body",
    "address",
    "article",
    "aside",
    "blockquote",
    "center",
    "dd",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figure",
    "footer",
    "header",
    "li",
    "main",
    "menu",
    "ol",
    "search",
    "section",
    "ul",
];

/// Elements whose text a browser lays out as separate lines: inside a
/// `<pre>`, each is kept on lines of its own.
const LINE_BREAKING: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "dd",
    "div",
    "dl",
    "dt",
    "figcaption",
    "figure",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "li",
    "ol",
    "p",
    "pre",
    "section",
    "table",
    "tr",
    "ul",
];

/// Elements that start a new line or block in htmd's output — where they
/// themselves stand on lines of their own (see [`Tree::on_own_lines`]) —
/// so nothing rendered before them can touch an inline span inside them.
const BLOCK_BOUNDARIES: &[&str] = &[
    "html",
    "body",
    "address",
    "article",
    "aside",
    "blockquote",
    "caption",
    "dd",
    "div",
    "dl",
    "dt",
    "figcaption",
    "figure",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "li",
    "ol",
    "p",
    "pre",
    "section",
    "table",
    "td",
    "th",
    "tr",
    "ul",
];

/// A `<pre>`: a fenced block where one can stand, else an inline span.
fn pre(element: &Element<'_>, tree: &mut Tree) -> HandlerResult {
    let mut text = String::new();
    collect_text(element.node, &mut text);
    let text = normalize_line_endings(&text);
    let text = text.trim_end_matches('\n');
    if text.trim().is_empty() {
        return String::new().into();
    }
    if !tree.on_own_lines(element.node) {
        // htmd trims the spaces before a block element; a browser would
        // break the line there, so a span keeps at least a space.
        let gap = before(element.node, tree) != Before::Nothing;
        return inline_span(text, gap);
    }
    let fence = "`".repeat((longest_backtick_run(text) + 1).max(3));
    let language = language(element).unwrap_or_default();
    tree.ended_lines.insert(address(element.node));
    format!("\n\n{fence}{language}\n{text}\n{fence}\n\n").into()
}

/// A `<code>` outside a `<pre>`: an inline span.
fn inline_code(node: &Rc<Node>, tree: &mut Tree) -> HandlerResult {
    let mut text = String::new();
    collect_text(node, &mut text);
    inline_span(&text, before(node, tree) == Before::Element)
}

/// `text` as a single-line inline span, its whitespace collapsed, after a
/// space when `gap`; nothing for blank text.
fn inline_span(text: &str, gap: bool) -> HandlerResult {
    let text = text.split_ascii_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return String::new().into();
    }
    let delimiter = "`".repeat(longest_backtick_run(&text) + 1);
    // A span whose text starts or ends with a backtick needs a space
    // inside each delimiter (the renderer strips one from each side).
    let pad = if text.starts_with('`') || text.ends_with('`') {
        " "
    } else {
        ""
    };
    let gap = if gap { " " } else { "" };
    format!("{gap}{delimiter}{pad}{text}{pad}{delimiter}").into()
}

/// The text a browser shows for `node`'s subtree: text nodes as they are,
/// `<br>` as a line break, line-breaking blocks on lines of their own.
fn collect_text(node: &Rc<Node>, out: &mut String) {
    for child in node.children.borrow().iter() {
        match &child.data {
            NodeData::Text { contents } => out.push_str(&contents.borrow()),
            NodeData::Element { name, .. } => {
                let name = &*name.local;
                if SKIPPED.contains(&name) {
                    continue;
                }
                if name == "br" {
                    out.push('\n');
                    continue;
                }
                let breaks = LINE_BREAKING.contains(&name);
                if breaks && !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                collect_text(child, out);
                if breaks && !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            _ => {}
        }
    }
}

/// `CommonMark` ends a line at `\n`, `\r\n` or a lone `\r` (which a
/// `&#13;` reference puts in the text): all of them become `\n`, so the
/// fence-length and single-line checks see every line.
fn normalize_line_endings(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// The longest run of consecutive backticks in `text`.
fn longest_backtick_run(text: &str) -> usize {
    text.split(|c| c != '`').map(str::len).max().unwrap_or(0)
}

/// The fence info string: the `language-*` class of the `pre`'s `code`
/// child, else of the `pre` itself. The sanitizer already reduced the
/// class to that one token; the character check keeps a backtick (which
/// would end the info string) or a space out regardless.
fn language(element: &Element<'_>) -> Option<String> {
    let from_code = element.node.children.borrow().iter().find_map(|child| {
        (tag(child) == Some("code"))
            .then(|| class_language(child))
            .flatten()
    });
    from_code.or_else(|| class_language(element.node))
}

fn class_language(node: &Rc<Node>) -> Option<String> {
    let NodeData::Element { attrs, .. } = &node.data else {
        return None;
    };
    let attrs = attrs.borrow();
    let class = attrs.iter().find(|attr| &*attr.name.local == "class")?;
    class
        .value
        .split_ascii_whitespace()
        .filter_map(|token| token.strip_prefix("language-"))
        .find(|name| {
            !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_+#.".contains(&b))
        })
        .map(ToOwned::to_owned)
}

fn is_fence_container(tag: &str) -> bool {
    FENCE_CONTAINERS.contains(&tag)
}

/// What htmd renders just before `node` on the same line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Before {
    /// Nothing: `node` starts a line or block.
    Nothing,
    /// Text, which can never end in a live backtick: htmd escapes one in
    /// text as `` \` ``.
    Text,
    /// An earlier element in the same inline run, whose output may end in
    /// a backtick (a span from a sibling `<code>` or an inline `<pre>`, or
    /// one nested in a sibling) — or be empty, leaving whatever came
    /// before *it* touching `node`.
    Element,
}

/// What comes before `node`. A handler cannot see htmd's output so far, so
/// this is judged from the tree and from what earlier blocks were recorded
/// rendering (see [`register`]), every doubt resolved toward a gap — which
/// costs at most a space at the start of a line:
///
/// - an earlier sibling element is [`Before::Element`], unless it is a
///   `<br>` or `<hr>` (always a line break or a rule) or a block whose
///   output really ended a line where lines survive. A block tag alone
///   proves nothing: an empty `<pre>`, `<ul>` or `<table>` renders as
///   nothing, leaving whatever came before it touching `node`, and a
///   `<pre>` under an inline element is itself a span. Elements htmd
///   skips render nothing and are passed over;
/// - with no earlier sibling, `node` starts a line only when its parent is
///   a block that itself stands on lines of its own: a `<p>` under an
///   `<em>`, in a heading or in a table cell is flattened into the line
///   around it, so the walk goes on up.
fn before(node: &Rc<Node>, tree: &mut Tree) -> Before {
    let mut current = Rc::clone(node);
    loop {
        let Some(parent) = parent(&current) else {
            return Before::Nothing;
        };
        let at = tree.position(&parent, &current);
        let children = parent.children.borrow();
        for earlier in children[..at].iter().rev() {
            match &earlier.data {
                NodeData::Element { name, .. } if SKIPPED.contains(&&*name.local) => {}
                NodeData::Element { name, .. } if matches!(&*name.local, "br" | "hr") => {
                    return Before::Nothing;
                }
                NodeData::Element { .. } => {
                    let ended_line =
                        tree.ended_lines.contains(&address(earlier)) && tree.on_own_lines(earlier);
                    return if ended_line {
                        Before::Nothing
                    } else {
                        Before::Element
                    };
                }
                NodeData::Text { contents } if !contents.borrow().trim().is_empty() => {
                    return Before::Text;
                }
                _ => {}
            }
        }
        drop(children);
        if tag(&parent).is_some_and(|name| BLOCK_BOUNDARIES.contains(&name))
            && tree.on_own_lines(&parent)
        {
            return Before::Nothing;
        }
        current = parent;
    }
}

/// Lookups over the document being converted, cached so the handlers stay
/// linear in its size: uncached, every `<code>` would rescan its parent's
/// children to find itself (quadratic in the sibling count), and every
/// element would walk its whole ancestor chain again.
#[derive(Debug, Default)]
struct Tree {
    /// Each indexed node's position among its parent's children.
    positions: HashMap<usize, usize>,
    /// Each visited node's [`Tree::on_own_lines`] answer.
    own_lines: HashMap<usize, bool>,
    /// The blocks rendered so far whose output ended in a line break.
    ended_lines: HashSet<usize>,
}

impl Tree {
    /// `child`'s index among `parent`'s children. A parent's children are
    /// indexed together on first use, and a cached index is checked
    /// against the tree before it is trusted.
    fn position(&mut self, parent: &Rc<Node>, child: &Rc<Node>) -> usize {
        let children = parent.children.borrow();
        if let Some(&at) = self.positions.get(&address(child))
            && children
                .get(at)
                .is_some_and(|found| Rc::ptr_eq(found, child))
        {
            return at;
        }
        let mut position = 0;
        for (at, sibling) in children.iter().enumerate() {
            self.positions.insert(address(sibling), at);
            if Rc::ptr_eq(sibling, child) {
                position = at;
            }
        }
        position
    }

    /// Whether every element ancestor of `node` is a fence container, so
    /// what htmd renders for `node` keeps lines of its own: a fence can
    /// open there, and a block there starts a fresh line.
    fn on_own_lines(&mut self, node: &Rc<Node>) -> bool {
        // Every node on the way up shares the answer of the first one that
        // is already known, is the root, or has a non-container parent.
        let mut pending = Vec::new();
        let mut current = Rc::clone(node);
        let answer = loop {
            if let Some(&known) = self.own_lines.get(&address(&current)) {
                break known;
            }
            pending.push(address(&current));
            match parent(&current) {
                None => break true,
                Some(parent) if tag(&parent).is_some_and(is_fence_container) => current = parent,
                Some(_) => break false,
            }
        };
        for key in pending {
            self.own_lines.insert(key, answer);
        }
        answer
    }
}

/// A node's identity for the [`Tree`] lookups: its address, stable while
/// the document is being converted.
fn address(node: &Rc<Node>) -> usize {
    Rc::as_ptr(node).addr()
}

fn tag(node: &Rc<Node>) -> Option<&str> {
    match &node.data {
        NodeData::Element { name, .. } => Some(&name.local),
        _ => None,
    }
}

/// `node`'s parent element (htmd's tree keeps it as a weak reference).
fn parent(node: &Rc<Node>) -> Option<Rc<Node>> {
    let weak = node.parent.take();
    let parent = weak.as_ref().and_then(std::rc::Weak::upgrade);
    node.parent.set(weak);
    parent.filter(|parent| matches!(parent.data, NodeData::Element { .. }))
}

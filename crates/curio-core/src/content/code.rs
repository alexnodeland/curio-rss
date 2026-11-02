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
//!   delimiters it would otherwise merge with.
//!
//! A `language-*` class on the `code` (or else the `pre`) becomes the
//! fence's info string.

use std::rc::Rc;

use htmd::{
    Element, HtmlToMarkdownBuilder, Node,
    element_handler::{HandlerResult, Handlers},
};
use markup5ever_rcdom::NodeData;

/// Registers the `pre` and `code` handlers on `builder`.
pub(super) fn register(builder: HtmlToMarkdownBuilder) -> HtmlToMarkdownBuilder {
    builder
        .add_handler(vec!["pre"], |_: &dyn Handlers, element: Element<'_>| {
            Some(pre(&element))
        })
        .add_handler(vec!["code"], |_: &dyn Handlers, element: Element<'_>| {
            Some(inline_code(element.node))
        })
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

/// Elements that start a new line or block in htmd's output, so nothing
/// rendered before them can touch an inline span inside them.
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
fn pre(element: &Element<'_>) -> HandlerResult {
    let mut text = String::new();
    collect_text(element.node, &mut text);
    let text = normalize_line_endings(&text);
    let text = text.trim_end_matches('\n');
    if text.trim().is_empty() {
        return String::new().into();
    }
    if !ancestors(element.node).all(|ancestor| tag(&ancestor).is_some_and(is_fence_container)) {
        // htmd trims the spaces before a block element; a browser would
        // break the line there, so a span keeps at least a space.
        let gap = before(element.node) != Before::Nothing;
        return inline_span(text, gap);
    }
    let fence = "`".repeat((longest_backtick_run(text) + 1).max(3));
    let language = language(element).unwrap_or_default();
    format!("\n\n{fence}{language}\n{text}\n{fence}\n\n").into()
}

/// A `<code>` outside a `<pre>`: an inline span.
fn inline_code(node: &Rc<Node>) -> HandlerResult {
    let mut text = String::new();
    collect_text(node, &mut text);
    inline_span(&text, before(node) == Before::Element)
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
    /// a backtick (a span from a sibling `<code>`, or one nested in a
    /// sibling).
    Element,
}

fn before(node: &Rc<Node>) -> Before {
    let mut current = Rc::clone(node);
    loop {
        let Some(parent) = parent(&current) else {
            return Before::Nothing;
        };
        let children = parent.children.borrow();
        let at = children
            .iter()
            .position(|child| Rc::ptr_eq(child, &current))
            .unwrap_or(0);
        for earlier in children[..at].iter().rev() {
            match &earlier.data {
                // A line break, or a block htmd ends with blank lines.
                NodeData::Element { name, .. }
                    if matches!(&*name.local, "br" | "hr")
                        || BLOCK_BOUNDARIES.contains(&&*name.local) =>
                {
                    return Before::Nothing;
                }
                NodeData::Element { .. } => return Before::Element,
                NodeData::Text { contents } if !contents.borrow().trim().is_empty() => {
                    return Before::Text;
                }
                _ => {}
            }
        }
        if tag(&parent).is_none_or(|name| BLOCK_BOUNDARIES.contains(&name)) {
            return Before::Nothing;
        }
        drop(children);
        current = parent;
    }
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

/// `node`'s element ancestors, nearest first.
fn ancestors(node: &Rc<Node>) -> impl Iterator<Item = Rc<Node>> {
    std::iter::successors(parent(node), parent)
}

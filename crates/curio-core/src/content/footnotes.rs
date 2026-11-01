//! Footnotes and in-page links in exported markdown.
//!
//! The sanitizer keeps footnote structure in-page (prefixed `id`s, and
//! fragment-only `href`s rewritten to match). htmd would turn that into
//! `[1](#user-content-fn1)` — a dead link in a notes vault, where element
//! ids don't exist. This module recognizes the structure every footnote
//! generator shares (a reference `<a href="#X">` pointing at an
//! `<li id="X">` of an `<ol>`, outside that item) and has htmd emit
//! `CommonMark` footnotes instead: `[^1]` at each reference, `[^1]: …`
//! definitions where the list was, back-reference links dropped (a link
//! from a definition back to the element holding one of its references —
//! the `↩`). Any other fragment-only link keeps its text and loses the
//! dead destination.
//!
//! Which `<ol>`s are footnote lists is decided up front (by the id of a
//! list's first item), so every list is walked exactly once, however
//! deeply ordinary lists nest.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};

use htmd::{
    Element, HtmlToMarkdownBuilder, Node,
    element_handler::{HandlerResult, Handlers},
};
use markup5ever_rcdom::NodeData;
use scraper::{ElementRef, Html, Selector};

/// The footnote structure of one document.
#[derive(Debug, Default)]
struct Footnotes {
    /// Definition `li` id → footnote label (1-based, document order).
    labels: HashMap<String, usize>,
    /// The id of the first `li` of each footnote `<ol>` — how the `ol`
    /// handler recognizes a footnote list without walking it.
    list_heads: HashSet<String>,
    /// `href`s of the back-reference links inside definitions.
    backlinks: HashSet<String>,
}

impl Footnotes {
    fn detect(html: &str) -> Self {
        let (Ok(anchor_sel), Ok(ol_sel)) = (Selector::parse("a[href^='#']"), Selector::parse("ol"))
        else {
            return Self::default();
        };
        let doc = Html::parse_fragment(html);
        let anchors: Vec<ElementRef<'_>> = doc.select(&anchor_sel).collect();
        // The anchors that reference `id` from outside the item `li`.
        let references = |li: &ElementRef<'_>, id: &str| -> Vec<ElementRef<'_>> {
            anchors
                .iter()
                .filter(|a| {
                    a.value().attr("href").and_then(|h| h.strip_prefix('#')) == Some(id)
                        && !a.ancestors().any(|node| node.id() == li.id())
                })
                .copied()
                .collect()
        };

        let mut footnotes = Self::default();
        let mut definitions = Vec::new();
        for ol in doc.select(&ol_sel) {
            let items: Vec<ElementRef<'_>> = ol
                .children()
                .filter_map(ElementRef::wrap)
                .filter(|child| child.value().name() == "li")
                .collect();
            let all_definitions = !items.is_empty()
                && items.iter().all(|li| {
                    li.value()
                        .id()
                        .is_some_and(|id| !references(li, id).is_empty())
                });
            if !all_definitions {
                continue;
            }
            if let Some(head) = items.first().and_then(|li| li.value().id()) {
                footnotes.list_heads.insert(head.to_owned());
            }
            for li in items {
                if let Some(id) = li.value().id()
                    && !footnotes.labels.contains_key(id)
                {
                    let label = footnotes.labels.len() + 1;
                    footnotes.labels.insert(id.to_owned(), label);
                    definitions.push((li, id));
                }
            }
        }
        for (li, id) in definitions {
            // What a back-reference may target: a reference anchor itself
            // or an element around it (`<sup id="fnref1"><a href="#fn1">`).
            let holders: HashSet<&str> = references(&li, id)
                .iter()
                .flat_map(|reference| {
                    std::iter::once(*reference)
                        .chain(reference.ancestors().filter_map(ElementRef::wrap))
                        .filter_map(|el| el.value().id())
                })
                .collect();
            for a in li.select(&anchor_sel) {
                if let Some(href) = a.value().attr("href")
                    && holders.contains(&href[1..])
                {
                    footnotes.backlinks.insert(href.to_owned());
                }
            }
        }
        footnotes
    }

    /// Whether `list` is a footnote list: its first `li` is a list head.
    fn is_footnote_list(&self, list: &Rc<Node>) -> bool {
        first_item_id(list).is_some_and(|id| self.list_heads.contains(&id))
    }
}

/// The `id` of the first `li` child of `list`, if it has one.
fn first_item_id(list: &Rc<Node>) -> Option<String> {
    list.children
        .borrow()
        .iter()
        .find_map(|child| match &child.data {
            NodeData::Element { name, attrs, .. } if &*name.local == "li" => Some(
                attrs
                    .borrow()
                    .iter()
                    .find(|attr| &*attr.name.local == "id")
                    .map(|attr| attr.value.to_string()),
            ),
            _ => None,
        })
        .flatten()
}

/// Registers the footnote-aware `a`/`li`/`ol` handlers for `html` on
/// `builder`. Each falls back to htmd's own handler for anything that is
/// not part of a footnote or an in-page link.
pub(super) fn register(builder: HtmlToMarkdownBuilder, html: &str) -> HtmlToMarkdownBuilder {
    let footnotes = Arc::new(Footnotes::detect(html));
    // Definitions rendered by the `li` handler, collected by the enclosing
    // `ol` handler (htmd hands each handler only its own element).
    let pending: Arc<Mutex<Vec<String>>> = Arc::default();

    let for_anchor = Arc::clone(&footnotes);
    let for_item = Arc::clone(&footnotes);
    let item_sink = Arc::clone(&pending);
    builder
        .add_handler(
            vec!["a"],
            move |handlers: &dyn Handlers, element: Element<'_>| {
                anchor(&for_anchor, handlers, element)
            },
        )
        .add_handler(
            vec!["li"],
            move |handlers: &dyn Handlers, element: Element<'_>| {
                list_item(&for_item, &item_sink, handlers, element)
            },
        )
        .add_handler(
            vec!["ol"],
            move |handlers: &dyn Handlers, element: Element<'_>| {
                ordered_list(&footnotes, &pending, handlers, element)
            },
        )
}

fn attr<'e>(element: &Element<'e>, name: &str) -> Option<&'e str> {
    element
        .attrs
        .iter()
        .find(|attr| &*attr.name.local == name)
        .map(|attr| &*attr.value)
}

/// A footnote reference → `[^n]`; a back-reference → nothing; any other
/// fragment-only link → its text. Real links go to htmd.
fn anchor(
    footnotes: &Footnotes,
    handlers: &dyn Handlers,
    element: Element<'_>,
) -> Option<HandlerResult> {
    let Some(href) = attr(&element, "href").filter(|href| href.starts_with('#')) else {
        return handlers.fallback(element);
    };
    if let Some(label) = footnotes.labels.get(&href[1..]) {
        return Some(format!("[^{label}]").into());
    }
    if footnotes.backlinks.contains(href) {
        return Some(String::new().into());
    }
    Some(handlers.walk_children(element.node))
}

/// A definition item → `[^n]: …` (continuation lines indented four
/// spaces), handed to the enclosing list; other items go to htmd.
fn list_item(
    footnotes: &Footnotes,
    pending: &Mutex<Vec<String>>,
    handlers: &dyn Handlers,
    element: Element<'_>,
) -> Option<HandlerResult> {
    let Some(label) = attr(&element, "id").and_then(|id| footnotes.labels.get(id)) else {
        return handlers.fallback(element);
    };
    let body = handlers.walk_children(element.node).content;
    let mut definition = format!("[^{label}]: ");
    for (index, line) in body.trim().lines().enumerate() {
        if index > 0 {
            definition.push('\n');
            if !line.trim().is_empty() {
                definition.push_str("    ");
            }
        }
        definition.push_str(line.trim_end());
    }
    pending
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(definition);
    Some(String::new().into())
}

/// A footnote list → its definitions as a block; any other list → htmd.
/// Which kind a list is was decided by [`Footnotes::detect`], so each list
/// is walked exactly once (nested ordinary lists cost linear time).
fn ordered_list(
    footnotes: &Footnotes,
    pending: &Mutex<Vec<String>>,
    handlers: &dyn Handlers,
    element: Element<'_>,
) -> Option<HandlerResult> {
    let before = pending.lock().unwrap_or_else(PoisonError::into_inner).len();
    let take_definitions = || {
        let mut pending = pending.lock().unwrap_or_else(PoisonError::into_inner);
        let at = before.min(pending.len());
        pending.split_off(at)
    };
    if !footnotes.is_footnote_list(element.node) {
        let rendered = handlers.fallback(element);
        // A definition id repeated inside an ordinary list (duplicate ids)
        // must not leak into the next footnote list.
        drop(take_definitions());
        return rendered;
    }
    let walked = handlers.walk_children(element.node);
    let definitions = take_definitions();
    if definitions.is_empty() {
        return Some(walked);
    }
    Some(format!("\n\n{}\n\n", definitions.join("\n")).into())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    const DOC: &str = "<p>Claim<sup id=\"user-content-fnref1\">\
        <a href=\"#user-content-fn1\">1</a></sup> \
        and again<sup><a href=\"#user-content-fn2\">2</a></sup>.</p>\
        <ol><li id=\"user-content-fn1\"><p>First note. \
        <a href=\"#user-content-fnref1\">↩</a></p></li>\
        <li id=\"user-content-fn2\">Second note.</li></ol>";

    #[test]
    fn detects_definitions_and_backlinks() {
        let footnotes = Footnotes::detect(DOC);
        assert_eq!(footnotes.labels.get("user-content-fn1"), Some(&1));
        assert_eq!(footnotes.labels.get("user-content-fn2"), Some(&2));
        assert!(footnotes.list_heads.contains("user-content-fn1"));
        assert!(footnotes.backlinks.contains("#user-content-fnref1"));
    }

    #[test]
    fn a_backlink_may_target_the_reference_anchor_itself() {
        let footnotes = Footnotes::detect(
            "<p>x<a id=\"r1\" href=\"#n1\">1</a></p>\
             <ol><li id=\"n1\">note <a href=\"#r1\">↩</a></li></ol>",
        );
        assert!(footnotes.backlinks.contains("#r1"));
    }

    #[test]
    fn other_in_page_links_inside_a_definition_are_not_backlinks() {
        let footnotes = Footnotes::detect(
            "<h2 id=\"part-2\">Part two</h2>\
             <p>x<sup id=\"r1\"><a href=\"#n1\">1</a></sup></p>\
             <ol><li id=\"n1\">See <a href=\"#part-2\">part two</a>. \
             <a href=\"#r1\">↩</a></li></ol>",
        );
        assert!(footnotes.backlinks.contains("#r1"));
        assert!(!footnotes.backlinks.contains("#part-2"));
    }

    #[test]
    fn an_ordinary_list_with_ids_is_not_footnotes() {
        // Ids alone don't make footnotes: nothing outside references them.
        let footnotes = Footnotes::detect("<ol><li id=\"a\">x</li><li id=\"b\">y</li></ol>");
        assert!(footnotes.labels.is_empty());
        assert!(footnotes.list_heads.is_empty());
        // A list referenced only partly is not a footnote list either.
        let footnotes = Footnotes::detect(
            "<a href=\"#a\">a</a><ol><li id=\"a\">x</li><li id=\"b\">y</li></ol>",
        );
        assert!(footnotes.labels.is_empty());
    }

    #[test]
    fn a_self_reference_inside_the_item_does_not_count() {
        let footnotes = Footnotes::detect("<ol><li id=\"a\"><a href=\"#a\">x</a></li></ol>");
        assert!(footnotes.labels.is_empty());
    }
}

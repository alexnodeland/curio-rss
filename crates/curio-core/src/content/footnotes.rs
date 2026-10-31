//! Footnotes and in-page links in exported markdown.
//!
//! The sanitizer keeps footnote structure in-page (prefixed `id`s, and
//! fragment-only `href`s rewritten to match). htmd would turn that into
//! `[1](#curio-fn1)` — a dead link in a notes vault, where element ids don't
//! exist. This module recognizes the structure every footnote generator
//! shares (a reference `<a href="#X">` pointing at an `<li id="X">` of an
//! `<ol>`, outside that item) and has htmd emit `CommonMark` footnotes
//! instead: `[^1]` at each reference, `[^1]: …` definitions where the list
//! was, back-reference links (`↩`) dropped. Any other fragment-only link
//! keeps its text and loses the dead destination.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use htmd::{
    Element, HtmlToMarkdownBuilder,
    element_handler::{HandlerResult, Handlers},
};
use scraper::{ElementRef, Html, Selector};

/// The footnote structure of one document.
#[derive(Debug, Default)]
struct Footnotes {
    /// Definition `li` id → footnote label (1-based, document order).
    labels: HashMap<String, usize>,
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
        let referenced_from_outside = |li: &ElementRef<'_>, id: &str| {
            anchors.iter().any(|a| {
                a.value().attr("href").and_then(|h| h.strip_prefix('#')) == Some(id)
                    && !a.ancestors().any(|node| node.id() == li.id())
            })
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
                        .is_some_and(|id| referenced_from_outside(li, id))
                });
            if !all_definitions {
                continue;
            }
            for li in items {
                if let Some(id) = li.value().id()
                    && !footnotes.labels.contains_key(id)
                {
                    let label = footnotes.labels.len() + 1;
                    footnotes.labels.insert(id.to_owned(), label);
                    definitions.push(li);
                }
            }
        }
        for li in definitions {
            for a in li.select(&anchor_sel) {
                if let Some(href) = a.value().attr("href")
                    && !footnotes.labels.contains_key(&href[1..])
                {
                    footnotes.backlinks.insert(href.to_owned());
                }
            }
        }
        footnotes
    }
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
    let has_footnotes = !footnotes.labels.is_empty();
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
                if has_footnotes {
                    ordered_list(&pending, handlers, element)
                } else {
                    handlers.fallback(element)
                }
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
/// Whether this `ol` holds definitions is only known once its items have
/// been walked, so a non-footnote list is walked once here and once more
/// by htmd (only in documents that have footnotes at all).
fn ordered_list(
    pending: &Mutex<Vec<String>>,
    handlers: &dyn Handlers,
    element: Element<'_>,
) -> Option<HandlerResult> {
    let before = pending.lock().unwrap_or_else(PoisonError::into_inner).len();
    let _ = handlers.walk_children(element.node);
    let definitions = {
        let mut pending = pending.lock().unwrap_or_else(PoisonError::into_inner);
        let at = before.min(pending.len());
        pending.split_off(at)
    };
    if definitions.is_empty() {
        return handlers.fallback(element);
    }
    Some(format!("\n\n{}\n\n", definitions.join("\n")).into())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    const DOC: &str = "<p>Claim<sup id=\"curio-fnref1\"><a href=\"#curio-fn1\">1</a></sup> \
        and again<sup><a href=\"#curio-fn2\">2</a></sup>.</p>\
        <ol><li id=\"curio-fn1\"><p>First note. <a href=\"#curio-fnref1\">↩</a></p></li>\
        <li id=\"curio-fn2\">Second note.</li></ol>";

    #[test]
    fn detects_definitions_and_backlinks() {
        let footnotes = Footnotes::detect(DOC);
        assert_eq!(footnotes.labels.get("curio-fn1"), Some(&1));
        assert_eq!(footnotes.labels.get("curio-fn2"), Some(&2));
        assert!(footnotes.backlinks.contains("#curio-fnref1"));
    }

    #[test]
    fn an_ordinary_list_with_ids_is_not_footnotes() {
        // Ids alone don't make footnotes: nothing outside references them.
        let footnotes = Footnotes::detect("<ol><li id=\"a\">x</li><li id=\"b\">y</li></ol>");
        assert!(footnotes.labels.is_empty());
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

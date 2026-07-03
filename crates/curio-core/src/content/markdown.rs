//! HTML → `CommonMark` via htmd's DOM walk (D5).
//!
//! The sketch's regex converter — which destroyed every element spanning
//! lines — is banned; conversion is a real DOM traversal. Input is
//! expected to be sanitized already (this module is called on stored
//! content at export time), but the skip list drops any script/style/
//! boilerplate defensively anyway.

use htmd::HtmlToMarkdown;

use super::ContentError;

/// Converts an HTML fragment to `CommonMark`.
///
/// # Errors
///
/// [`ContentError::Markdown`] if the DOM walk fails to serialize
/// (practically unreachable for parseable HTML).
pub fn to_markdown(html: &str) -> Result<String, ContentError> {
    let converter = HtmlToMarkdown::builder()
        .skip_tags(vec![
            "script", "style", "iframe", "object", "embed", "form", "nav", "aside", "header",
            "footer", "noscript", "svg", "canvas",
        ])
        .build();
    Ok(converter.convert(html)?.trim().to_owned())
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
    fn headings_and_lists_convert() {
        let md = to_markdown("<h2>title</h2><ul><li>a</li><li>b</li></ul>").unwrap();
        assert!(md.contains("## title"));
        let has_bullet = md
            .lines()
            .any(|line| matches!(line.trim_start().chars().next(), Some('-' | '*' | '+')));
        assert!(has_bullet, "list lost: {md}");
    }
}

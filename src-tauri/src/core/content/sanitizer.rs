//! HTML sanitization for safe rendering.

use ammonia::Builder;
use std::collections::HashSet;

/// Sanitize HTML content for safe display
pub fn sanitize_html(html: &str) -> String {
    Builder::default()
        .tags(allowed_tags())
        .tag_attributes(tag_attributes())
        .url_schemes(url_schemes())
        .link_rel(Some("noopener noreferrer"))
        .clean(html)
        .to_string()
}

/// Tags allowed in sanitized HTML
fn allowed_tags() -> HashSet<&'static str> {
    [
        // Block elements
        "p", "div", "br", "hr",
        // Headings
        "h1", "h2", "h3", "h4", "h5", "h6",
        // Lists
        "ul", "ol", "li",
        // Text formatting
        "strong", "b", "em", "i", "u", "s", "strike", "del", "ins", "mark",
        // Code
        "code", "pre", "kbd", "samp", "var",
        // Links and media
        "a", "img",
        // Quotes and semantic
        "blockquote", "q", "cite", "abbr", "dfn",
        // Tables
        "table", "thead", "tbody", "tfoot", "tr", "th", "td", "caption", "colgroup", "col",
        // Other
        "figure", "figcaption", "details", "summary", "time",
        // Allow spans for basic styling
        "span",
    ]
    .into_iter()
    .collect()
}

/// Attributes allowed per tag
fn tag_attributes() -> std::collections::HashMap<&'static str, HashSet<&'static str>> {
    let mut map = std::collections::HashMap::new();

    map.insert("a", ["href", "title", "rel"].into_iter().collect());
    map.insert("img", ["src", "alt", "title", "width", "height", "loading"].into_iter().collect());
    map.insert("td", ["colspan", "rowspan"].into_iter().collect());
    map.insert("th", ["colspan", "rowspan", "scope"].into_iter().collect());
    map.insert("time", ["datetime"].into_iter().collect());
    map.insert("abbr", ["title"].into_iter().collect());
    map.insert("ol", ["start", "type"].into_iter().collect());
    map.insert("blockquote", ["cite"].into_iter().collect());
    map.insert("q", ["cite"].into_iter().collect());
    map.insert("code", ["class"].into_iter().collect()); // For syntax highlighting
    map.insert("pre", ["class"].into_iter().collect());

    map
}

/// URL schemes allowed in links and images
fn url_schemes() -> HashSet<&'static str> {
    ["http", "https", "mailto"].into_iter().collect()
}

/// Strip all HTML tags, returning plain text
pub fn strip_html(html: &str) -> String {
    Builder::default()
        .tags(HashSet::new())
        .clean(html)
        .to_string()
}

/// Extract text content from HTML, preserving some structure
pub fn extract_text(html: &str) -> String {
    // First sanitize, then strip
    let sanitized = sanitize_html(html);

    // Replace block elements with newlines for readability
    let text = sanitized
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("</p>", "\n\n")
        .replace("</div>", "\n")
        .replace("</li>", "\n")
        .replace("</h1>", "\n\n")
        .replace("</h2>", "\n\n")
        .replace("</h3>", "\n\n")
        .replace("</h4>", "\n")
        .replace("</h5>", "\n")
        .replace("</h6>", "\n")
        .replace("</blockquote>", "\n\n");

    // Strip remaining tags
    strip_html(&text)
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Truncate text to a maximum length, respecting word boundaries
pub fn truncate_text(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        return text.to_string();
    }

    let truncated = &text[..max_len];

    // Find last space to avoid cutting mid-word
    if let Some(last_space) = truncated.rfind(' ') {
        format!("{}...", &truncated[..last_space])
    } else {
        format!("{}...", truncated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_basic_html() {
        let html = "<p>Hello <strong>world</strong>!</p>";
        let result = sanitize_html(html);
        assert!(result.contains("<p>"));
        assert!(result.contains("<strong>"));
    }

    #[test]
    fn test_sanitize_removes_scripts() {
        let html = "<p>Text</p><script>alert('xss')</script>";
        let result = sanitize_html(html);
        assert!(!result.contains("<script>"));
        assert!(!result.contains("alert"));
        assert!(result.contains("<p>Text</p>"));
    }

    #[test]
    fn test_sanitize_removes_style() {
        let html = "<p>Text</p><style>body { display: none; }</style>";
        let result = sanitize_html(html);
        assert!(!result.contains("<style>"));
        assert!(!result.contains("display"));
    }

    #[test]
    fn test_sanitize_removes_onclick() {
        let html = r#"<a href="https://example.com" onclick="alert('xss')">Link</a>"#;
        let result = sanitize_html(html);
        assert!(result.contains("href="));
        assert!(!result.contains("onclick"));
    }

    #[test]
    fn test_sanitize_removes_javascript_urls() {
        let html = r#"<a href="javascript:alert('xss')">Link</a>"#;
        let result = sanitize_html(html);
        assert!(!result.contains("javascript:"));
    }

    #[test]
    fn test_sanitize_allows_images() {
        let html = r#"<img src="https://example.com/img.jpg" alt="Image" />"#;
        let result = sanitize_html(html);
        assert!(result.contains("<img"));
        assert!(result.contains("src="));
        assert!(result.contains("alt="));
    }

    #[test]
    fn test_sanitize_adds_link_rel() {
        let html = r#"<a href="https://example.com">Link</a>"#;
        let result = sanitize_html(html);
        assert!(result.contains("rel=\"noopener noreferrer\""));
    }

    #[test]
    fn test_sanitize_allows_code_class() {
        let html = r#"<code class="language-rust">let x = 1;</code>"#;
        let result = sanitize_html(html);
        assert!(result.contains("class=\"language-rust\""));
    }

    #[test]
    fn test_strip_html() {
        let html = "<p>Hello <strong>world</strong>!</p>";
        let result = strip_html(html);
        assert_eq!(result, "Hello world!");
    }

    #[test]
    fn test_strip_html_complex() {
        let html = r#"<div><h1>Title</h1><p>Some <em>emphasized</em> text</p></div>"#;
        let result = strip_html(html);
        assert_eq!(result, "TitleSome emphasized text");
    }

    #[test]
    fn test_extract_text() {
        let html = "<h1>Title</h1><p>First paragraph.</p><p>Second paragraph.</p>";
        let result = extract_text(html);
        assert!(result.contains("Title"));
        assert!(result.contains("First paragraph."));
        assert!(result.contains("Second paragraph."));
    }

    #[test]
    fn test_extract_text_with_list() {
        let html = "<ul><li>Item 1</li><li>Item 2</li></ul>";
        let result = extract_text(html);
        assert!(result.contains("Item 1"));
        assert!(result.contains("Item 2"));
    }

    #[test]
    fn test_truncate_text_short() {
        let text = "Hello world";
        let result = truncate_text(text, 50);
        assert_eq!(result, "Hello world");
    }

    #[test]
    fn test_truncate_text_long() {
        let text = "Hello world this is a long text that needs truncation";
        let result = truncate_text(text, 20);
        assert!(result.len() <= 23); // 20 + "..."
        assert!(result.ends_with("..."));
        assert!(!result.contains("truncation"));
    }

    #[test]
    fn test_truncate_text_word_boundary() {
        let text = "Hello world";
        let result = truncate_text(text, 8);
        // Should truncate to "Hello..." not "Hello wo..."
        assert_eq!(result, "Hello...");
    }

    #[test]
    fn test_sanitize_preserves_tables() {
        let html = "<table><tr><th>Header</th></tr><tr><td>Cell</td></tr></table>";
        let result = sanitize_html(html);
        assert!(result.contains("<table>"));
        assert!(result.contains("<th>"));
        assert!(result.contains("<td>"));
    }
}

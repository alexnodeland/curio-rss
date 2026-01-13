//! Markdown export for articles.

use crate::core::models::{Article, ExportSettings, ImageExportMode, LinkStyle};
use chrono::{DateTime, Utc};

/// Export an article to Markdown format
pub fn article_to_markdown(article: &Article, settings: &ExportSettings) -> String {
    let mut output = String::new();

    // Frontmatter
    if settings.include_frontmatter {
        output.push_str("---\n");
        for field in &settings.frontmatter_fields {
            match field.as_str() {
                "title" => {
                    output.push_str(&format!("title: \"{}\"\n", escape_yaml(&article.title)));
                }
                "author" => {
                    if let Some(ref author) = article.author {
                        output.push_str(&format!("author: \"{}\"\n", escape_yaml(author)));
                    }
                }
                "source" => {
                    if let Some(ref url) = article.url {
                        output.push_str(&format!("source: \"{}\"\n", url));
                    }
                }
                "published" => {
                    if let Some(published) = article.published_at {
                        output.push_str(&format!(
                            "published: {}\n",
                            published.format("%Y-%m-%d")
                        ));
                    }
                }
                "saved" => {
                    output.push_str(&format!("saved: {}\n", Utc::now().format("%Y-%m-%d")));
                }
                "feed" => {
                    // Feed title would need to be passed in; skip for now
                }
                _ => {}
            }
        }
        output.push_str("---\n\n");
    }

    // Title
    output.push_str(&format!("# {}\n\n", article.title));

    // Source link
    if settings.include_source_link {
        if let Some(ref url) = article.url {
            output.push_str(&format!("*[Original article]({})*\n\n", url));
        }
    }

    // Content
    if let Some(ref html) = article.content_html {
        let markdown = html_to_markdown(html, settings);
        output.push_str(&markdown);
    } else if let Some(ref text) = article.content_text {
        output.push_str(text);
    } else if let Some(ref summary) = article.summary {
        output.push_str(&html_to_markdown(summary, settings));
    }

    output
}

/// Convert HTML to Markdown
pub fn html_to_markdown(html: &str, settings: &ExportSettings) -> String {
    // Use htmd crate for conversion
    // For now, implement a simple converter
    let mut result = html.to_string();

    // Basic HTML to Markdown conversions
    result = result.replace("<br>", "\n");
    result = result.replace("<br/>", "\n");
    result = result.replace("<br />", "\n");

    // Headers
    result = regex_replace(&result, r"<h1[^>]*>(.*?)</h1>", "# $1\n\n");
    result = regex_replace(&result, r"<h2[^>]*>(.*?)</h2>", "## $1\n\n");
    result = regex_replace(&result, r"<h3[^>]*>(.*?)</h3>", "### $1\n\n");
    result = regex_replace(&result, r"<h4[^>]*>(.*?)</h4>", "#### $1\n\n");
    result = regex_replace(&result, r"<h5[^>]*>(.*?)</h5>", "##### $1\n\n");
    result = regex_replace(&result, r"<h6[^>]*>(.*?)</h6>", "###### $1\n\n");

    // Text formatting
    result = regex_replace(&result, r"<strong>(.*?)</strong>", "**$1**");
    result = regex_replace(&result, r"<b>(.*?)</b>", "**$1**");
    result = regex_replace(&result, r"<em>(.*?)</em>", "*$1*");
    result = regex_replace(&result, r"<i>(.*?)</i>", "*$1*");
    result = regex_replace(&result, r"<code>(.*?)</code>", "`$1`");
    result = regex_replace(&result, r"<del>(.*?)</del>", "~~$1~~");
    result = regex_replace(&result, r"<s>(.*?)</s>", "~~$1~~");

    // Links
    match settings.link_style {
        LinkStyle::Inline => {
            result = regex_replace(&result, r#"<a[^>]*href="([^"]*)"[^>]*>(.*?)</a>"#, "[$2]($1)");
        }
        LinkStyle::Reference => {
            // For reference style, we'd need to collect links and add at bottom
            // For simplicity, use inline for now
            result = regex_replace(&result, r#"<a[^>]*href="([^"]*)"[^>]*>(.*?)</a>"#, "[$2]($1)");
        }
    }

    // Images
    match settings.image_handling {
        ImageExportMode::Strip => {
            result = regex_replace(&result, r"<img[^>]*>", "");
        }
        _ => {
            result = regex_replace(
                &result,
                r#"<img[^>]*src="([^"]*)"[^>]*alt="([^"]*)"[^>]*/?>"#,
                "![$2]($1)",
            );
            result = regex_replace(&result, r#"<img[^>]*src="([^"]*)"[^>]*/?>"#, "![]($1)");
        }
    }

    // Paragraphs
    result = result.replace("</p>", "\n\n");
    result = regex_replace(&result, r"<p[^>]*>", "");

    // Blockquotes
    result = regex_replace(&result, r"<blockquote[^>]*>(.*?)</blockquote>", "> $1\n\n");

    // Code blocks
    result = regex_replace(
        &result,
        r#"<pre[^>]*><code[^>]*class="language-([^"]*)"[^>]*>(.*?)</code></pre>"#,
        "```$1\n$2\n```\n\n",
    );
    result = regex_replace(&result, r"<pre[^>]*><code[^>]*>(.*?)</code></pre>", "```\n$1\n```\n\n");
    result = regex_replace(&result, r"<pre[^>]*>(.*?)</pre>", "```\n$1\n```\n\n");

    // Lists (simple)
    result = regex_replace(&result, r"<li[^>]*>(.*?)</li>", "- $1\n");
    result = regex_replace(&result, r"</?[ou]l[^>]*>", "\n");

    // Remove remaining tags
    result = regex_replace(&result, r"<[^>]+>", "");

    // Clean up whitespace
    result = result
        .lines()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n");

    // Remove excessive newlines
    while result.contains("\n\n\n") {
        result = result.replace("\n\n\n", "\n\n");
    }

    // Decode HTML entities
    result = decode_html_entities(&result);

    result.trim().to_string()
}

/// Simple regex replacement helper
fn regex_replace(text: &str, pattern: &str, replacement: &str) -> String {
    // Using simple string replacement for common cases
    // In production, use the regex crate
    let re = regex::Regex::new(pattern).ok();
    match re {
        Some(r) => r.replace_all(text, replacement).to_string(),
        None => text.to_string(),
    }
}

/// Escape special characters for YAML
fn escape_yaml(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

/// Decode common HTML entities
fn decode_html_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&mdash;", "—")
        .replace("&ndash;", "–")
        .replace("&hellip;", "...")
        .replace("&rsquo;", "'")
        .replace("&lsquo;", "'")
        .replace("&rdquo;", """)
        .replace("&ldquo;", """)
}

/// Generate a filename from article and template
pub fn generate_filename(article: &Article, template: &str) -> String {
    let date = article
        .published_at
        .unwrap_or_else(Utc::now)
        .format("%Y-%m-%d")
        .to_string();

    let title = sanitize_filename(&article.title);

    template
        .replace("{date}", &date)
        .replace("{title}", &title)
        .replace("{id}", &article.id.to_string())
}

/// Sanitize a string for use as a filename
fn sanitize_filename(s: &str) -> String {
    let invalid_chars = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

    let cleaned: String = s
        .chars()
        .map(|c| if invalid_chars.contains(&c) { '-' } else { c })
        .collect();

    // Truncate to reasonable length
    let truncated = if cleaned.len() > 100 {
        cleaned.chars().take(100).collect()
    } else {
        cleaned
    };

    // Remove leading/trailing spaces and dots
    truncated.trim().trim_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn make_test_article() -> Article {
        let mut article = Article::new(Uuid::new_v4(), "guid-123", "Test Article Title");
        article.url = Some("https://example.com/article".to_string());
        article.author = Some("John Doe".to_string());
        article.content_html = Some("<p>This is the <strong>content</strong>.</p>".to_string());
        article.published_at = Some(
            DateTime::parse_from_rfc3339("2024-01-15T10:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        );
        article
    }

    #[test]
    fn test_article_to_markdown_with_frontmatter() {
        let article = make_test_article();
        let settings = ExportSettings::default();

        let result = article_to_markdown(&article, &settings);

        assert!(result.starts_with("---\n"));
        assert!(result.contains("title: \"Test Article Title\""));
        assert!(result.contains("author: \"John Doe\""));
        assert!(result.contains("source: \"https://example.com/article\""));
        assert!(result.contains("published: 2024-01-15"));
        assert!(result.contains("# Test Article Title"));
    }

    #[test]
    fn test_article_to_markdown_without_frontmatter() {
        let article = make_test_article();
        let mut settings = ExportSettings::default();
        settings.include_frontmatter = false;

        let result = article_to_markdown(&article, &settings);

        assert!(!result.starts_with("---"));
        assert!(result.starts_with("# Test Article Title"));
    }

    #[test]
    fn test_html_to_markdown_headers() {
        let settings = ExportSettings::default();
        let html = "<h1>Title</h1><h2>Subtitle</h2>";
        let result = html_to_markdown(html, &settings);

        assert!(result.contains("# Title"));
        assert!(result.contains("## Subtitle"));
    }

    #[test]
    fn test_html_to_markdown_formatting() {
        let settings = ExportSettings::default();
        let html = "<strong>bold</strong> and <em>italic</em> and <code>code</code>";
        let result = html_to_markdown(html, &settings);

        assert!(result.contains("**bold**"));
        assert!(result.contains("*italic*"));
        assert!(result.contains("`code`"));
    }

    #[test]
    fn test_html_to_markdown_links() {
        let settings = ExportSettings::default();
        let html = r#"<a href="https://example.com">Example</a>"#;
        let result = html_to_markdown(html, &settings);

        assert!(result.contains("[Example](https://example.com)"));
    }

    #[test]
    fn test_html_to_markdown_images() {
        let settings = ExportSettings::default();
        let html = r#"<img src="https://example.com/img.jpg" alt="Description">"#;
        let result = html_to_markdown(html, &settings);

        assert!(result.contains("![Description](https://example.com/img.jpg)"));
    }

    #[test]
    fn test_html_to_markdown_strip_images() {
        let mut settings = ExportSettings::default();
        settings.image_handling = ImageExportMode::Strip;

        let html = r#"<p>Text</p><img src="https://example.com/img.jpg"><p>More text</p>"#;
        let result = html_to_markdown(html, &settings);

        assert!(!result.contains("img.jpg"));
        assert!(result.contains("Text"));
        assert!(result.contains("More text"));
    }

    #[test]
    fn test_html_to_markdown_entities() {
        let settings = ExportSettings::default();
        let html = "<p>Hello &amp; goodbye &mdash; that&#39;s all</p>";
        let result = html_to_markdown(html, &settings);

        assert!(result.contains("Hello & goodbye — that's all"));
    }

    #[test]
    fn test_generate_filename() {
        let article = make_test_article();

        let result = generate_filename(&article, "{date}-{title}");
        assert!(result.starts_with("2024-01-15-"));
        assert!(result.contains("Test Article Title"));
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("Normal Title"), "Normal Title");
        assert_eq!(sanitize_filename("Title: With Colon"), "Title- With Colon");
        assert_eq!(sanitize_filename("Title/With/Slashes"), "Title-With-Slashes");
        assert_eq!(sanitize_filename("  Spaces  "), "Spaces");
    }

    #[test]
    fn test_sanitize_filename_long() {
        let long_title = "A".repeat(150);
        let result = sanitize_filename(&long_title);
        assert!(result.len() <= 100);
    }

    #[test]
    fn test_escape_yaml() {
        assert_eq!(escape_yaml("Simple"), "Simple");
        assert_eq!(escape_yaml("With \"quotes\""), "With \\\"quotes\\\"");
        assert_eq!(escape_yaml("With\nnewline"), "With\\nnewline");
    }
}

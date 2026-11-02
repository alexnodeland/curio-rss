//! Shared proptest generators: hostile HTML fragments and the escaping
//! helpers the round-trip properties build documents with.

use std::fmt::Write as _;

use proptest::prelude::*;

/// Elements the generator draws from: what sanitization keeps, what it
/// must strip, and a few it has never heard of.
const ELEMENTS: &[&str] = &[
    "p",
    "div",
    "span",
    "a",
    "img",
    "b",
    "em",
    "strong",
    "pre",
    "code",
    "ul",
    "li",
    "table",
    "td",
    "h1",
    "h2",
    "blockquote",
    "br",
    "article",
    "nav",
    "aside",
    "footer",
    "script",
    "style",
    "iframe",
    "svg",
    "math",
    "form",
    "input",
    "button",
    "object",
    "embed",
    "base",
    "meta",
    "link",
    "noscript",
    "template",
    "video",
    "audio",
    "frame",
    "x-custom",
];

/// URL-attribute values, weighted toward the ones the scheme allowlist
/// exists for — including the case/whitespace/entity disguises.
const URLS: &[&str] = &[
    "https://ok.example/page",
    "http://ok.example/img.png",
    "relative/path",
    "/root-relative",
    "//protocol-relative.example/x",
    "#fragment",
    "javascript:alert(1)",
    "JaVaScRiPt:alert(1)",
    "  javascript:alert(1)",
    "java\tscript:alert(1)",
    "&#106;avascript:alert(1)",
    "javascript&colon;alert(1)",
    "vbscript:msgbox(1)",
    "data:text/html;base64,PHNjcmlwdD4=",
    "data:image/svg+xml,<svg onload=alert(1)>",
    "file:///etc/passwd",
    "mailto:a@b.example",
    "foo://custom.example/x",
    // WHATWG strips leading C0 controls and spaces and drops tabs and
    // newlines before reading the scheme (the `content_pipeline` fuzz find).
    "\u{2}https://ok.example/c0",
    "\u{1}javascript:alert(1)",
    "\u{1f} data:text/html,x",
    "HTTPS://ok.example/upper",
    "https:no-slashes.example",
    "ht\ntps://ok.example/split",
    "",
];

/// Non-URL attributes, handlers and styling included.
const ATTRS: &[&str] = &[
    "class",
    "id",
    "title",
    "alt",
    "onclick",
    "onerror",
    "onload",
    "onmouseover",
    "style",
    "srcdoc",
    "formaction",
    "xlink:href",
    "http-equiv",
    "data-x",
    "lang",
    "dir",
];

/// Tag-soup fragments a structured generator would never produce.
const SOUP: &[&str] = &[
    "<scr<script>ipt>alert(1)</script>",
    "<img src=x onerror=alert(1)//",
    "<!-- <script>alert(1)</script> -->",
    "<![CDATA[<script>alert(1)</script>]]>",
    "</p></div></body></html>",
    "<svg><script>alert(1)</script></svg>",
    "<math><mtext><table><mglyph><style><img src=x onerror=alert(1)>",
    "<noscript><p title=\"</noscript><img src=x onerror=alert(1)>\"></noscript>",
    "<a href=\"javascript:alert(1)\"<b>unterminated",
    "<p style=\"background:url(javascript:alert(1))\">",
    "<base href=\"https://evil.example/\">",
    "<meta http-equiv=\"refresh\" content=\"0;url=javascript:alert(1)\">",
    "&lt;script&gt;alert(1)&lt;/script&gt;",
    "&amp;&lt;&gt;&quot;&#0;&#x110000;&bogus;",
    "\u{0}\u{1b}\u{202e}\u{feff}",
];

/// Free text: printable unicode plus the characters markup cares about —
/// backticks included, since code spans and fences are sized by the
/// backtick runs in their text (#245).
pub fn text() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-zA-Z0-9 .,;:!?'\"()&<>/=\\-_`日本語🎌]{0,24}",
        "\\PC{0,16}",
    ]
}

/// Backtick runs next to an escaped payload: what makes the delimiters of
/// adjacent code spans merge, or a fence close early, if any length or
/// gap is misjudged (#245). Random text almost never lines them up.
fn code_text() -> impl Strategy<Value = String> {
    (
        prop::sample::select(&["", "`", "``", "```", "````"][..]),
        prop::sample::select(
            &[
                "",
                "&lt;script&gt;alert(1)&lt;/script&gt;",
                "&lt;img src=x onerror=alert(1)&gt;",
                "x",
            ][..],
        ),
        prop::sample::select(&["", "`", "``", "```", " ", "\n\n"][..]),
    )
        .prop_map(|(lead, payload, tail)| format!("{lead}{payload}{tail}"))
}

/// Containers that change how code renders: inline (a `<pre>` becomes a
/// span), flattened into one line (heading, table cell, link), whole lines
/// (quote, list item) and a footnote definition (#245).
const CODE_CONTAINERS: &[(&str, &str)] = &[
    ("", ""),
    ("<div>", "</div>"),
    ("<em>", "</em>"),
    ("<h2>T ", "</h2>"),
    ("<table><tr><td>", "</td></tr></table>"),
    ("<a href=\"https://ok.example/\">", "</a>"),
    ("<blockquote>", "</blockquote>"),
    ("<ul><li>", "</li></ul>"),
    (
        "<p>x<a href=\"#fn1\">1</a></p><ol><li id=\"fn1\">",
        "</li></ol>",
    ),
];

/// Sibling `<code>` and `<pre>` elements around the blocks that render as
/// nothing or as a line break, in one of [`CODE_CONTAINERS`]: the shapes
/// where a span's gap or a fence's placement is decided (#245).
fn code_siblings() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        2 => code_text().prop_map(|text| format!("<code>{text}</code>")),
        1 => code_text().prop_map(|text| format!("<pre>{text}</pre>")),
        1 => prop::sample::select(
            &["<pre></pre>", "<ul></ul>", "<table></table>", "<p></p>", "<p>p</p>", "<br>", " ", "t"][..]
        )
        .prop_map(ToOwned::to_owned),
    ];
    (
        prop::sample::select(CODE_CONTAINERS),
        prop::collection::vec(piece, 1..5),
    )
        .prop_map(|((open, close), pieces)| format!("{open}{}{close}", pieces.concat()))
}

/// Escapes text for an XML/HTML text node or a double-quoted attribute.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

fn attribute() -> impl Strategy<Value = String> {
    prop_oneof![
        (
            prop::sample::select(&["href", "src", "action", "poster"][..]),
            prop::sample::select(URLS)
        )
            .prop_map(|(name, url)| format!(" {name}=\"{url}\"")),
        (prop::sample::select(ATTRS), text())
            .prop_map(|(name, value)| format!(" {name}=\"{}\"", escape(&value))),
    ]
}

/// A hostile HTML fragment: nested elements from [`ELEMENTS`] with
/// attributes from [`URLS`] / [`ATTRS`], text, and raw tag soup.
pub fn hostile_html() -> impl Strategy<Value = String> {
    html_from(ELEMENTS)
}

fn html_from(elements: &'static [&'static str]) -> impl Strategy<Value = String> {
    let leaf = prop_oneof![
        3 => text().prop_map(|text| escape(&text)),
        1 => text(),
        1 => code_text(),
        1 => code_siblings(),
        1 => prop::sample::select(SOUP).prop_map(ToOwned::to_owned),
    ];
    leaf.prop_recursive(4, 48, 6, move |inner| {
        (
            prop::sample::select(elements),
            prop::collection::vec(attribute(), 0..4),
            prop::collection::vec(inner, 0..6),
            any::<bool>(),
        )
            .prop_map(|(name, attrs, children, close)| {
                let mut out = format!("<{name}{}>", attrs.concat());
                out.push_str(&children.concat());
                if close {
                    write!(out, "</{name}>").unwrap();
                }
                out
            })
    })
}

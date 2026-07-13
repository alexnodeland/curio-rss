//! The Reddit enrichment provider (`enrich-reddit` cargo feature).
//!
//! Reddit's RSS ships a stub (footer links, truncated selftext,
//! thumbnail-only media). This provider fetches the post's public JSON
//! (`<permalink>.json?raw_json=1`) through the policed client and builds
//! a full body: the complete selftext HTML, gallery images in their
//! declared order, an inline figure for image posts, and a plain link
//! for external link posts. The output is RAW HTML — the caller pushes
//! it through `content::process`, so ammonia remains the storage gate
//! (the hostile fixture proves a scripted selftext stores clean).
//!
//! Recognition is by *path shape* (`/r/<sub>/comments/<id>/…`), not by
//! host, so the provider stays hermetically testable against loopback
//! fixtures; a false positive merely costs one failed JSON fetch and
//! falls back to the generic readability path.

use serde_json::Value;
use url::Url;

use super::{EnrichError, Enriched, escape_attr, escape_text};
use crate::fetch::{FetchRequest, PolicedClient};

/// Whether a source URL looks like a reddit post permalink
/// (`…/r/<subreddit>/comments/<id>…`).
#[must_use]
pub fn is_reddit_post(source_url: &str) -> bool {
    json_endpoint(source_url).is_some()
}

/// The post's JSON endpoint: the permalink path with `.json?raw_json=1`
/// appended, on the same origin (which is what makes loopback test
/// fixtures possible). `None` when the URL is not a post permalink.
#[must_use]
pub fn json_endpoint(source_url: &str) -> Option<String> {
    let url = Url::parse(source_url).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let segments: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    // /r/<sub>/comments/<id>[/<slug>]
    if segments.len() < 4 || segments[0] != "r" || segments[2] != "comments" {
        return None;
    }
    let mut endpoint = url.clone();
    endpoint.set_query(None);
    endpoint.set_fragment(None);
    let path = endpoint.path().trim_end_matches('/').to_owned();
    endpoint.set_path(&format!("{path}.json"));
    endpoint.set_query(Some("raw_json=1"));
    Some(endpoint.to_string())
}

/// Authentication for [`fetch_post`]: a bearer token plus the API
/// origin it belongs to (`oauth.reddit.com`, or a loopback double in
/// tests). `None` = the public unauthenticated endpoint.
#[derive(Debug, Clone, Copy)]
pub struct PostAuth<'a> {
    /// The OAuth bearer token.
    pub bearer: &'a str,
    /// The origin authenticated calls are made against.
    pub api_origin: &'a str,
}

/// Fetches and parses the post JSON for a permalink — authenticated
/// against the API origin when `auth` is given (the 100 QPM tier), else
/// the public same-origin `.json` endpoint (~10 QPM).
///
/// # Errors
///
/// [`EnrichError`] on fetch, HTTP, or payload-shape failures — all soft
/// (the caller falls back to the generic path).
pub async fn fetch_post(
    client: &PolicedClient,
    source_url: &str,
    allow_private_network: bool,
    auth: Option<PostAuth<'_>>,
) -> Result<Enriched, EnrichError> {
    let (endpoint, bearer) = match auth {
        Some(auth) => match super::reddit_auth::authed_json_endpoint(auth.api_origin, source_url) {
            Some(endpoint) => (Some(endpoint), Some(auth.bearer.to_owned())),
            None => (None, None),
        },
        None => (None, None),
    };
    let endpoint = endpoint
        .or_else(|| json_endpoint(source_url))
        .ok_or_else(|| {
            EnrichError::Malformed(format!("not a reddit post permalink: {source_url}"))
        })?;
    let response = client
        .fetch(&FetchRequest {
            url: endpoint,
            allow_private_network,
            bearer,
            ..FetchRequest::default()
        })
        .await?;
    if !response.is_success() {
        return Err(EnrichError::Http {
            status: response.status,
            retry_after: response.retry_after,
        });
    }
    parse_post(&response.body)
}

/// Parses a reddit post-JSON document into the enriched body. Split out
/// of [`fetch_post`] so the fixtures test the parser hermetically.
///
/// # Errors
///
/// [`EnrichError::Malformed`] when the payload is not a post listing.
pub fn parse_post(body: &[u8]) -> Result<Enriched, EnrichError> {
    let value: Value =
        serde_json::from_slice(body).map_err(|err| EnrichError::Malformed(err.to_string()))?;
    let post = value
        .pointer("/0/data/children/0/data")
        .ok_or_else(|| EnrichError::Malformed("no post in listing".to_owned()))?;

    let mut html = String::new();
    if let Some(selftext) = post.get("selftext_html").and_then(Value::as_str) {
        html.push_str(selftext);
    }
    if post.get("is_gallery").and_then(Value::as_bool) == Some(true) {
        push_gallery(&mut html, post);
    } else {
        push_link_media(&mut html, post);
    }

    Ok(Enriched {
        html,
        author: post
            .get("author")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    })
}

/// Appends the gallery's images in their declared order
/// (`gallery_data.items` → `media_metadata[id].s.u`).
fn push_gallery(html: &mut String, post: &Value) {
    let Some(items) = post
        .pointer("/gallery_data/items")
        .and_then(Value::as_array)
    else {
        return;
    };
    for item in items {
        let Some(media_id) = item.get("media_id").and_then(Value::as_str) else {
            continue;
        };
        let media = post.pointer(&format!("/media_metadata/{media_id}/s"));
        let source = media
            .and_then(|s| s.get("u").or_else(|| s.get("gif")))
            .and_then(Value::as_str);
        if let Some(src) = source {
            push_figure(html, src);
        }
    }
}

/// Appends an image/link post's media: an inline figure for an image
/// post (or the preview image when there is one), plus a plain anchor to
/// an external target so the link survives into the markdown export.
fn push_link_media(html: &mut String, post: &Value) {
    let permalink = post.get("permalink").and_then(Value::as_str).unwrap_or("");
    let target = post
        .get("url_overridden_by_dest")
        .or_else(|| post.get("url"))
        .and_then(Value::as_str)
        // A non-http(s) target never enters the built HTML at all — not
        // even as inert link text (defense in depth over the ammonia
        // scheme filter that would strip its href anyway).
        .filter(|url| {
            let lower = url.trim_start().to_ascii_lowercase();
            lower.starts_with("http://") || lower.starts_with("https://")
        })
        .unwrap_or("");
    let is_self = target.is_empty() || (!permalink.is_empty() && target.ends_with(permalink));
    let hint = post.get("post_hint").and_then(Value::as_str).unwrap_or("");

    if hint == "image" && !is_self {
        push_figure(html, target);
        return;
    }
    if let Some(preview) = post
        .pointer("/preview/images/0/source/url")
        .and_then(Value::as_str)
    {
        push_figure(html, preview);
    }
    if !is_self {
        use std::fmt::Write as _;
        let _ = write!(
            html,
            "<p><a href=\"{href}\">{text}</a></p>",
            href = escape_attr(target),
            text = escape_text(target),
        );
    }
}

fn push_figure(html: &mut String, src: &str) {
    use std::fmt::Write as _;
    let _ = write!(
        html,
        "<figure><img src=\"{src}\" alt=\"\"></figure>",
        src = escape_attr(src),
    );
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    const SELF_POST: &[u8] = include_bytes!("../../../../fixtures/feeds/reddit_post.json");
    const GALLERY: &[u8] = include_bytes!("../../../../fixtures/feeds/reddit_gallery.json");
    const HOSTILE: &[u8] = include_bytes!("../../../../fixtures/nasty/reddit_hostile.json");

    #[test]
    fn recognizes_post_permalinks_by_path_shape() {
        assert!(is_reddit_post(
            "https://www.reddit.com/r/rust/comments/abc123/title/"
        ));
        assert!(is_reddit_post(
            "http://127.0.0.1:8080/r/rust/comments/abc123/title/"
        ));
        assert!(!is_reddit_post("https://www.reddit.com/r/rust/"));
        assert!(!is_reddit_post("https://example.com/blog/comments/"));
        assert!(!is_reddit_post("not a url"));
    }

    #[test]
    fn the_json_endpoint_keeps_the_origin_and_appends_json() {
        assert_eq!(
            json_endpoint("https://www.reddit.com/r/rust/comments/abc123/title/?share=1").unwrap(),
            "https://www.reddit.com/r/rust/comments/abc123/title.json?raw_json=1"
        );
    }

    #[test]
    fn a_self_post_yields_its_full_selftext_and_preview_figure() {
        let enriched = parse_post(SELF_POST).unwrap();
        assert!(enriched.html.contains("cut compile times in half"));
        assert!(
            enriched
                .html
                .contains("https://preview.redd.it/buildchart.png"),
            "preview image inlined: {}",
            enriched.html
        );
        assert_eq!(enriched.author.as_deref(), Some("helpful_dev"));
    }

    #[test]
    fn a_gallery_yields_every_image_in_declared_order() {
        let enriched = parse_post(GALLERY).unwrap();
        let first = enriched.html.find("https://i.redd.it/m1.jpg").unwrap();
        let second = enriched.html.find("https://i.redd.it/m2.jpg").unwrap();
        assert!(
            first < second,
            "gallery order follows gallery_data.items, not the metadata map: {}",
            enriched.html
        );
    }

    #[test]
    fn hostile_payloads_pass_through_but_die_at_the_sanitize_gate() {
        // The provider itself is transport: raw hostile HTML may leave it…
        let enriched = parse_post(HOSTILE).unwrap();
        assert!(enriched.html.contains("<script>"));
        // …but the storage path always runs the sanitize gate; prove it
        // kills every vector from this payload.
        let clean = crate::content::sanitize(&enriched.html, None);
        assert!(!clean.contains("script"), "{clean}");
        assert!(!clean.contains("onerror"), "{clean}");
        assert!(!clean.contains("javascript:"), "{clean}");
        assert!(
            crate::content::plain_text(&clean).contains("hi"),
            "benign content survives"
        );
    }

    #[test]
    fn malformed_payloads_fail_soft_never_panic() {
        for garbage in [&b"not json"[..], b"[]", b"{}", b"[1,2]"] {
            assert!(matches!(
                parse_post(garbage),
                Err(EnrichError::Malformed(_))
            ));
        }
    }
}

//! Fetch-free URL rewrites: sites whose feed URL follows from the page URL
//! alone, so a pasted page subscribes without scraping it (reddit and
//! `YouTube` pages in particular are hostile to scraping).
//!
//! Every rewrite is still *validated* by the caller (fetched and parsed)
//! before it is offered — a rule here is a guess, never a promise.

use url::Url;

/// Feed URLs implied by `url`, best first. Empty when no rule applies.
///
/// | Input | Feed |
/// |---|---|
/// | `reddit.com/r/<sub>[/<sort>]`, `reddit.com/user/<name>` | `…/.rss` |
/// | `github.com/<owner>/<repo>[/releases]` | `…/releases.atom` |
/// | `github.com/<owner>/<repo>/tags` | `…/tags.atom` |
/// | `youtube.com/…?list=<id>` (playlist or watch page) | `feeds/videos.xml?playlist_id=<id>` |
/// | `youtube.com/channel/UC…` | `feeds/videos.xml?channel_id=UC…` |
/// | `<name>.substack.com/…` | `https://<name>.substack.com/feed` |
/// | any other host's `/@<user>` (Mastodon; not X/Threads/Medium) | `/@<user>.rss` |
pub(super) fn rewrites(url: &Url) -> Vec<String> {
    let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
        return Vec::new();
    };
    let segments: Vec<&str> = url
        .path_segments()
        .map(|segments| segments.filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    // A host-specific rule owns its host even when it has no rewrite for
    // this path: `youtube.com/@handle` is not a Mastodon account.
    if host == "reddit.com" || host.ends_with(".reddit.com") {
        return reddit(&segments).into_iter().collect();
    }
    if host == "github.com" || host == "www.github.com" {
        return github(&segments).into_iter().collect();
    }
    if is_youtube_host(&host) {
        return youtube(url, &segments).into_iter().collect();
    }
    if host == "substack.com" || host.ends_with(".substack.com") {
        // `substack.com` itself (profiles, the reader app) has no one feed.
        let name = host.strip_suffix(".substack.com").unwrap_or_default();
        let is_publication = !name.is_empty() && name != "www" && !name.contains('.');
        let is_feed = segments.first() == Some(&"feed");
        return (is_publication && !is_feed)
            .then(|| format!("https://{host}/feed"))
            .into_iter()
            .collect();
    }
    if NOT_MASTODON
        .iter()
        .any(|other| host == *other || host.ends_with(&format!(".{other}")))
    {
        return Vec::new();
    }
    mastodon(url, &segments).into_iter().collect()
}

/// Hosts that reuse the `/@handle` shape but are not Mastodon.
const NOT_MASTODON: [&str; 4] = ["twitter.com", "x.com", "threads.net", "medium.com"];

/// Whether `host` is `YouTube` (`youtube.com`, any subdomain, or `youtu.be`).
pub(super) fn is_youtube_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host == "youtube.com" || host.ends_with(".youtube.com") || host == "youtu.be"
}

/// The videos feed of one `YouTube` channel.
pub(super) fn youtube_channel_feed_url(channel_id: &str) -> String {
    format!("https://www.youtube.com/feeds/videos.xml?channel_id={channel_id}")
}

/// Extracts a `UC…` id from a `.../channel/UC…` URL (24 chars, `UC` + 22).
pub(super) fn channel_id_from_url(url: &str) -> Option<String> {
    let start = url.find("/channel/")? + "/channel/".len();
    let id: String = url[start..]
        .chars()
        .take_while(|c| is_id_char(*c))
        .collect();
    (id.len() == 24 && id.starts_with("UC")).then_some(id)
}

fn is_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

fn is_id(value: &str) -> bool {
    !value.is_empty() && value.chars().all(is_id_char)
}

/// `/r/<sub>`, `/r/<sub>/<sort>` and `/user|u/<name>` → their `.rss`.
fn reddit(segments: &[&str]) -> Option<String> {
    const SORTS: [&str; 5] = ["hot", "new", "top", "rising", "controversial"];
    match segments {
        ["r", sub] if is_id(sub) => Some(format!("https://www.reddit.com/r/{sub}/.rss")),
        ["r", sub, sort] if is_id(sub) && SORTS.contains(sort) => {
            Some(format!("https://www.reddit.com/r/{sub}/{sort}/.rss"))
        }
        ["user" | "u", name] if is_id(name) => {
            Some(format!("https://www.reddit.com/user/{name}/.rss"))
        }
        _ => None,
    }
}

/// `/<owner>/<repo>[/releases|/tags]` → the releases (or tags) Atom feed.
fn github(segments: &[&str]) -> Option<String> {
    // First path segments that are GitHub pages, not repository owners.
    const RESERVED: [&str; 16] = [
        "about",
        "apps",
        "collections",
        "enterprise",
        "explore",
        "features",
        "issues",
        "login",
        "marketplace",
        "notifications",
        "orgs",
        "pricing",
        "pulls",
        "settings",
        "sponsors",
        "topics",
    ];
    let (owner, repo, feed) = match segments {
        [owner, repo] | [owner, repo, "releases"] => (*owner, *repo, "releases"),
        [owner, repo, "tags"] => (*owner, *repo, "tags"),
        _ => return None,
    };
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    let valid = |part: &str| is_id(&part.replace('.', "_"));
    (valid(owner) && valid(repo) && !RESERVED.contains(&owner.to_ascii_lowercase().as_str()))
        .then(|| format!("https://github.com/{owner}/{repo}/{feed}.atom"))
}

/// A `?list=` playlist (on `/playlist` or a `/watch` page) or a
/// `/channel/UC…` page → its videos feed.
fn youtube(url: &Url, segments: &[&str]) -> Option<String> {
    if segments.first() == Some(&"feeds") {
        return None; // already a feed URL
    }
    let playlist = url
        .query_pairs()
        .find(|(key, _)| key == "list")
        .map(|(_, value)| value.into_owned())
        .filter(|list| is_id(list));
    if let Some(list) = playlist {
        return Some(format!(
            "https://www.youtube.com/feeds/videos.xml?playlist_id={list}"
        ));
    }
    channel_id_from_url(url.path()).map(|id| youtube_channel_feed_url(&id))
}

/// `/@<user>` on any other host (Mastodon and its forks) → `/@<user>.rss`
/// on the same origin.
fn mastodon(url: &Url, segments: &[&str]) -> Option<String> {
    let [handle] = segments else {
        return None;
    };
    let user = handle.strip_prefix('@')?;
    if user.is_empty() || !user.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    let mut feed = url.clone();
    feed.set_path(&format!("/@{user}.rss"));
    feed.set_query(None);
    feed.set_fragment(None);
    Some(feed.into())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// Input URL → the feed URLs its rewrite proposes.
    const REWRITE_TABLE: &[(&str, &[&str])] = &[
        // reddit
        (
            "https://www.reddit.com/r/rust",
            &["https://www.reddit.com/r/rust/.rss"],
        ),
        (
            "https://old.reddit.com/r/rust/",
            &["https://www.reddit.com/r/rust/.rss"],
        ),
        (
            "https://reddit.com/r/rust/top/?t=week",
            &["https://www.reddit.com/r/rust/top/.rss"],
        ),
        (
            "https://www.reddit.com/user/spez",
            &["https://www.reddit.com/user/spez/.rss"],
        ),
        (
            "https://www.reddit.com/u/spez/",
            &["https://www.reddit.com/user/spez/.rss"],
        ),
        ("https://www.reddit.com/r/rust/.rss", &[]),
        ("https://www.reddit.com/r/rust/comments/abc/title/", &[]),
        ("https://www.reddit.com/", &[]),
        // GitHub
        (
            "https://github.com/rust-lang/rust",
            &["https://github.com/rust-lang/rust/releases.atom"],
        ),
        (
            "https://github.com/tauri-apps/tauri/releases",
            &["https://github.com/tauri-apps/tauri/releases.atom"],
        ),
        (
            "https://github.com/serde-rs/serde.git",
            &["https://github.com/serde-rs/serde/releases.atom"],
        ),
        (
            "https://github.com/rust-lang/rust/tags",
            &["https://github.com/rust-lang/rust/tags.atom"],
        ),
        (
            "https://github.com/vercel/next.js/",
            &["https://github.com/vercel/next.js/releases.atom"],
        ),
        ("https://github.com/rust-lang", &[]),
        ("https://github.com/settings/profile", &[]),
        ("https://github.com/rust-lang/rust/issues", &[]),
        ("https://github.com/rust-lang/rust/releases.atom", &[]),
        // YouTube
        (
            "https://www.youtube.com/playlist?list=PLjZ7Y0k9vYqzKt5qWdNOm0n2uLFAhJwq4",
            &[
                "https://www.youtube.com/feeds/videos.xml?playlist_id=PLjZ7Y0k9vYqzKt5qWdNOm0n2uLFAhJwq4",
            ],
        ),
        (
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PL123_abc-XYZ",
            &["https://www.youtube.com/feeds/videos.xml?playlist_id=PL123_abc-XYZ"],
        ),
        (
            "https://youtu.be/dQw4w9WgXcQ?list=PL123",
            &["https://www.youtube.com/feeds/videos.xml?playlist_id=PL123"],
        ),
        (
            "https://m.youtube.com/channel/UCXuqSBlHAE6Xw-yeJA0Tunw/videos",
            &["https://www.youtube.com/feeds/videos.xml?channel_id=UCXuqSBlHAE6Xw-yeJA0Tunw"],
        ),
        ("https://www.youtube.com/@linustechtips", &[]),
        ("https://www.youtube.com/watch?v=dQw4w9WgXcQ", &[]),
        (
            "https://www.youtube.com/feeds/videos.xml?playlist_id=PL123&list=PL123",
            &[],
        ),
        // Mastodon
        (
            "https://mastodon.social/@Gargron",
            &["https://mastodon.social/@Gargron.rss"],
        ),
        (
            "https://fosstodon.org/@rust_lang/?ref=x#top",
            &["https://fosstodon.org/@rust_lang.rss"],
        ),
        ("https://mastodon.social/@Gargron.rss", &[]),
        ("https://mastodon.social/@Gargron/1123581321", &[]),
        ("https://mastodon.social/@", &[]),
        ("https://x.com/@someone", &[]),
        ("https://www.threads.net/@someone", &[]),
        ("https://medium.com/@someone", &[]),
        // Substack
        (
            "https://example.substack.com/",
            &["https://example.substack.com/feed"],
        ),
        (
            "https://example.substack.com/p/some-post",
            &["https://example.substack.com/feed"],
        ),
        ("https://example.substack.com/feed", &[]),
        ("https://substack.com/@writer", &[]),
        ("https://www.substack.com/", &[]),
        // Everything else
        ("https://blog.example/", &[]),
        ("https://blog.example/posts/2026/hello", &[]),
    ];

    #[test]
    fn the_rewrite_table() {
        for (input, expected) in REWRITE_TABLE {
            let url = Url::parse(input).unwrap();
            assert_eq!(rewrites(&url), *expected, "rewriting {input}");
        }
    }

    #[test]
    fn channel_ids_must_be_well_formed() {
        assert_eq!(
            channel_id_from_url("https://www.youtube.com/channel/UCXuqSBlHAE6Xw-yeJA0Tunw?x=1"),
            Some("UCXuqSBlHAE6Xw-yeJA0Tunw".to_owned())
        );
        assert_eq!(channel_id_from_url("/channel/UCshort"), None);
        assert_eq!(
            channel_id_from_url("/channel/XXXuqSBlHAE6Xw-yeJA0Tunw"),
            None
        );
        assert_eq!(channel_id_from_url("/user/someone"), None);
    }

    #[test]
    fn the_mastodon_rewrite_keeps_the_origin_port() {
        let url = Url::parse("http://127.0.0.1:8080/@alice").unwrap();
        assert_eq!(rewrites(&url), vec!["http://127.0.0.1:8080/@alice.rss"]);
    }
}

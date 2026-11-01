# Privacy

Curio is **local-first**. Your reading data — feeds, articles, read/star
state, tags, the behavioral event log — lives on your machine, in files and
a SQLite database you own, and nowhere else.

## What Curio sends over the network

Every outbound request goes through curio-core's single **policed HTTP
client** (SSRF-guarded, size-capped, honest User-Agent) — a bare client
anywhere in the tree is a build-gated boundary violation. These are *all* the
classes of request the app can make; there are no others:

1. **Feed fetches (refresh).** The subscription URLs you added — and only
   those hosts. Conditional GET (`ETag`/`If-Modified-Since`) minimizes
   transfer; loopback/private-network addresses are refused by default. Every
   request carries a spec-correct `Accept` header (it just says "I want a
   feed").

   **One disclosed exception — reddit.com.** Reddit's CDN blocks curio at two
   layers: the default (rustls) TLS fingerprint *and* the honest curio
   User-Agent. So requests to `reddit.com` (and its subdomains) — and only
   those — use the platform-native TLS stack and send a browser-class
   User-Agent plus an `Accept-Language` header, spaced by a 6.5-second
   politeness delay (about nine requests per minute, under Reddit's
   unauthenticated rate limit). This is the *only* host that sees anything but the honest
   curio UA and the default TLS stack; no cookies, credentials, or identifiers
   are ever sent, and no other host is affected.

   **Optional Reddit API credentials (opt-in).** If you add your own Reddit
   app's client id and secret (Settings → Media & Privacy, or `curio reddit
   login`), Curio exchanges them for short-lived tokens at
   `www.reddit.com/api/v1/access_token` and loads full posts from
   `oauth.reddit.com` at the faster authenticated rate (~85 requests per
   minute). The secret is stored in your operating system's keychain — never
   in Curio's config, database, or logs (debug output redacts it and the
   access token) — and is sent to no host other than Reddit. Each profile
   keeps its own keychain item, so separate profiles can use separate Reddit
   apps. Removing the credentials returns that profile to the
   unauthenticated behavior above; nothing else changes.

2. **Feed discovery and favicons.** *Add feed* (and `curio feed add`)
   fetches the page you typed to discover its feeds, then fetches each
   candidate feed once to check it parses. If the page declares no feed, it
   tries at most six requests for common feed paths, **all on the same
   site** — a guessed path that redirects elsewhere is dropped without
   following it. For reddit,
   GitHub, YouTube, Mastodon and Substack URLs it may request the site's own
   feed URL directly instead of the page. It reads the page's declared icon
   (or the site's own `/favicon.ico`) — **same-origin only**. The **Google favicon service is never contacted
   unless you explicitly opt in** (a checkbox, off by default); turning it on
   sends only a site's *domain* to Google, and only for sites that declare no
   icon of their own.

3. **YouTube links (`youtube-nocookie.com`).** Video items show a click-to-
   load poster. Nothing is requested from YouTube until *you* click play, and
   then only via `youtube-nocookie.com` — the sole cross-origin frame the CSP
   allows.

4. **Article & site images (the image cache).** Remote images are fetched by
   Rust through the policed client, cached by content-hash in your OS cache
   dir, and served to the reader via a scoped `asset:` protocol. The webview
   never makes an unmediated `https:` image request (the CSP `img-src` has no
   `https:`), so images cannot become an exfiltration or fingerprinting path.

## What Curio never does

- **No telemetry.** No usage analytics, no crash reporting to us, no
  metrics, no "anonymous statistics".
- **No phone-home.** No update pings, no license checks, no first-run
  beacons. (When a release updater exists, checking for updates will be
  explicit and off-by-default or clearly user-controlled.)
- **No accounts.** There is no Curio service to sign in to.
- **No third parties.** Your data is never sent to, shared with, or sold to
  anyone, because it never leaves your machine.

## Your data, your files

- Saved articles export as **plain markdown** with YAML frontmatter into
  directories you configure (`curio.frontmatter.v1`).
- Reading behavior is an **append-only local JSONL event log**
  (`curio.events.v1`) under `.curio/events/` — designed for *your* tools to
  consume, gitignored by default so it never accidentally leaves via a repo.
- `curio export --all` (Phase 3) gives you the complete portable archive:
  OPML, events, settings. Deleting Curio's data directory deletes everything.

## Diagnostics

If you hit a crash, any diagnostics bundle is generated locally, redacted,
and shared only if **you** attach it to a report yourself.

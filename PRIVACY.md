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

   **One disclosed exception — reddit.com.** Reddit's CDN blocks the honest
   curio User-Agent outright, so requests to `reddit.com` (and its
   subdomains) send a browser-class User-Agent and an `Accept-Language`
   header, spaced by a 2-second politeness delay. This is the *only* host that
   sees anything but the honest curio UA; no cookies, credentials, or
   identifiers are sent, and no other host is affected. (Reddit additionally
   fingerprints the TLS client, so even this may not always succeed — a
   fuller fix is tracked separately.)

2. **Favicons.** *Add feed* fetches the page you typed to discover its feeds
   and reads its declared icon (or the site's own `/favicon.ico`) —
   **same-origin only**. The **Google favicon service is never contacted
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

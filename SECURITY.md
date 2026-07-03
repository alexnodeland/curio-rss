# Security Policy

## Reporting a vulnerability

Please report vulnerabilities **privately** — do not open a public issue.

- Preferred: GitHub private vulnerability reporting ("Report a vulnerability"
  on the repository's Security tab).
- Alternative: email `alex@ournature.studio` with subject `[curio security]`.

You will get an acknowledgement within 7 days. Please include reproduction
steps and an assessment of impact if you can. Coordinated disclosure: we ask
for a reasonable embargo while a fix ships; credit is given unless you prefer
otherwise.

## Supported versions

Curio is pre-1.0. Only the **latest release** (and `main`) receives security
fixes. After 1.0, the latest minor release line is supported.

## Threat model (what Curio defends against)

Curio's primary attack surface is **hostile feed content** — an RSS/Atom/JSON
feed is untrusted input from the open internet. The load-bearing defenses,
which are design commitments and CI-gated, are documented in
[docs/design/architecture.md](docs/design/architecture.md):

- **SSRF guard**: all outbound fetches go through one policed client that
  denies loopback/private/link-local addresses after DNS resolution and
  re-checks every redirect hop. The only exemption is the explicit per-feed
  `allow_private_network` config flag (`W1` in
  [docs/design/contracts-draft.md](docs/design/contracts-draft.md)) — never
  settable from feed content.
- **Sanitize-at-ingest**: feed HTML is sanitized (ammonia) before it is ever
  stored; raw feed markup never reaches the database or any rendering
  surface. Webview CSP is defense-in-depth, not the primary defense.
- **Headless core boundary**: `curio-core` carries no webview; `deny.toml`
  and the xtask boundary check keep it that way.
- **No bundled external binaries**: yt-dlp (post-v1 enrichment) is never
  bundled — external binary, pinned version, SHA256-verified.

Reports demonstrating a bypass of any of the above are exactly what this
policy is for.

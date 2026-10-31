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

## Verifying a release

Every release after v0.4.0 is built by
[`release.yml`](.github/workflows/release.yml) only after the full CI suite
passes on the tagged commit, and carries:

- **`SHA256SUMS`** — checksums of every asset:
  `shasum -a 256 -c SHA256SUMS --ignore-missing` in the download folder.
- **A build-provenance attestation** (SLSA, signed through Sigstore), stored
  in the repository and attached as `Curio-provenance.sigstore.json`:
  `gh attestation verify Curio-universal.dmg --repo alexnodeland/curio-rss`.
- **CycloneDX SBOMs** — `Curio-rust-<arch>.cdx.json` (the Rust dependency
  graph of the app, per macOS architecture) and `Curio-npm.cdx.json` (the
  frontend's whole npm lockfile — runtime packages and the build toolchain
  that compiled the bundle, dev dependencies included).
- **A minisign signature** on the updater archive (`Curio.app.tar.gz.sig`),
  which the in-app updater checks against the public key compiled into the
  app before installing anything. The private key is used in exactly one CI
  step, which runs no build scripts, in a job where nothing ahead of it runs
  project code, starts a background process or rewrites the step's
  environment; `cargo run -p xtask -- workflows` fails the build if that ever
  changes.

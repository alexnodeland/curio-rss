# Desktop e2e smoke (WebdriverIO + tauri-driver)

A self-contained WebdriverIO harness that drives the **real** Curio desktop
head — real webview, real curio-core engine behind the IPC boundary — through
[`tauri-driver`](https://tauri.app/develop/tests/webdriver/). It runs in the
nightly `smoke` job (`.github/workflows/nightly.yml`) as a **hard gate**, never
in the per-PR gate (`ci.yml`). It has its **own** `package.json` +
`package-lock.json` (installed with `npm ci`) so its heavy WebdriverIO
dependency tree never touches the frontend's install or lint/test surface.

A PR that edits this harness, the nightly workflow, the tauri configs, the
desktop manifests or main-window creation (`src-tauri/src/lib.rs`,
`browser_args.rs`) runs the nightly — and so this smoke — before it merges.
Those runs are not a required check (a path-filtered workflow can't be one), so
waiting for them to go green is on the shipper and the reviewer.

## Platforms

tauri-driver exists on **Linux** (WebKitWebGTK, `webkit2gtk-driver`) and
**Windows** (Edge WebView2 via `msedgedriver`) only. **macOS has no WebDriver
for WKWebView**, so macOS correctness rides the versioned manual checklist in
[`docs/release/macos-checklist.md`](../../../docs/release/macos-checklist.md)
instead — this is a deliberate gap (docs/design/roadmap.md §5, decision D11:
"Playwright-drives-Tauri is unimplementable; no test backdoors").

On Windows, `msedgedriver` hands WebView2 its `--remote-debugging-port` /
`--user-data-dir` flags through the `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`
env var, which the runtime ignores once an app sets browser arguments of its
own — and wry always does. Debug builds of the app (the smoke builds with
`--debug`) therefore forward that env var's flags after wry's defaults when
they create the main window (`src-tauri/src/browser_args.rs`); release builds
never take injected flags. Without it every Windows session fails with
"DevToolsActivePort file doesn't exist".

## What the harness sets up (`wdio.conf.ts`)

No test hooks live in the app. Before the app boots, the launcher:

- starts a feed stub on **127.0.0.1** (`support/stub.ts`) serving
  `/seed.xml` (three items, one of them `fixtures/html/xss-corpus.html`) and
  `/added.xml` (`fixtures/feeds/rss2.xml`), counting every request;
- seeds a throwaway `CURIO_PROFILE` through the **real `curio` CLI**:
  `init` (only `init` creates a profile — every other command refuses a
  directory without one), `feed add <stub>/seed.xml --allow-private-network`
  (the explicit W1 opt-in), `dest add notes <tmp>`, `fetch`.

The app inherits `CURIO_PROFILE` through tauri-driver. It is built with
`e2e/tauri.e2e.conf.json`, which points the startup update check at a dead
loopback endpoint, so nothing in the run reaches the real network.

## The five scenarios (`specs/smoke.e2e.ts`)

1. **boot** — the app launches to the three-pane shell over the seeded feed.
2. **add feed** — the add-feed modal subscribes to `<stub>/added.xml`; the row
   lands in the sidebar, and because nothing typed into the UI can carry the
   W1 exemption, the SSRF guard refuses its first fetch (the row turns
   unhealthy; the stub never sees a request for it).
3. **stored-XSS inert** — the corpus went feed → ingest → DB →
   `SanitizedHtml` → real webview: no script/embedding element, no `on*`
   handler, no `javascript:`/`data:text/html` URL survives; the legitimate
   text does; no dialog fired and the page never navigated.
4. **keyboard nav** — `j`/`k` move the article-list selection.
5. **promote-to-destination** — `p` writes a Markdown note into the sole
   destination whose frontmatter validates against
   `schemas/frontmatter.v1.json`.

All five run; none is skipped. Selectors are `data-testid` hooks on the
frontend (`three-pane`, `feed-row` + `data-feed-url`, `article-row`,
`add-feed-url`, `add-feed-submit`, `reader-content`). A failed scenario saves
a screenshot under `failures/` (or `$CURIO_E2E_SHOTS`); the nightly job
uploads it as an artifact.

## Running locally (Linux/Windows)

```sh
# one-time: the pinned driver (match TAURI_DRIVER_VERSION in nightly.yml),
# plus webkit2gtk-driver + xvfb on Linux
cargo install tauri-driver --locked --version 2.1.0

# from apps/desktop: build the app (frontend embedded) and the CLI under test
npm ci
npm run tauri build -- --debug --no-bundle --config e2e/tauri.e2e.conf.json
cargo build -p curio-cli

cd e2e && npm ci
xvfb-run --auto-servernum npm test   # Linux; plain `npm test` on Windows
```

Binaries resolve from `$CARGO_TARGET_DIR` (else `<repo>/target`);
`CURIO_BIN` / `CURIO_CLI_BIN` override either path and
`CURIO_PROFILE_BUILD=release` selects release builds. On Windows, set
`TAURI_NATIVE_DRIVER` to an `msedgedriver.exe` that matches the installed
WebView2 runtime. `npm run typecheck` type-checks the harness.

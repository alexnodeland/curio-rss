# Desktop e2e smoke (WebdriverIO + tauri-driver)

A self-contained WebdriverIO harness that drives the **real** Curio desktop
head — real webview, real curio-core engine behind the IPC boundary — through
[`tauri-driver`](https://tauri.app/develop/tests/webdriver/). It runs only in
the nightly `smoke` job (`.github/workflows/nightly.yml`), never in the per-PR
gate, and it has its **own** `package.json`/lockfile so its heavy WebdriverIO
dependency tree never touches the frontend's install or lint/test surface.

## Platforms

tauri-driver exists on **Linux** (WebKitWebGTK, `webkit2gtk-driver`) and
**Windows** (Edge WebView2) only. **macOS has no WebDriver for WKWebView**, so
macOS correctness rides the versioned manual checklist in
[`docs/release/macos-checklist.md`](../../../docs/release/macos-checklist.md)
instead — this is a deliberate gap (docs/design/roadmap.md §5, decision D11:
"Playwright-drives-Tauri is unimplementable; no test backdoors").

## The five scenarios (`specs/smoke.e2e.ts`)

1. **boot** — the app launches to the three-pane reader shell.
2. **add feed** — against a local wiremock / 127.0.0.1 stub (never the real
   network), the new feed lands in the sidebar.
3. **stored-XSS inert** — `fixtures/html/xss-corpus.html` flows ingest → DB →
   `SanitizedHtml` → real webview and every script/handler is stripped.
4. **keyboard nav** — `j`/`k` move the article-list selection.
5. **promote-to-destination** — `p` writes a schema-valid Markdown note.

`boot` runs today; the other four are `.skip` pending stable `data-testid`
selectors and the wiremock stub, and are documented inline. This is the
authored skeleton, not a finished suite.

## Running locally (Linux/Windows)

```sh
# from the repo root, build the binary under test first
cargo build -p curio-desktop
cd apps/desktop && npm run build           # produce ../build for the webview
cd e2e && npm install && npm test          # boots tauri-driver, runs the suite
```

`CURIO_BIN` overrides the binary path; `CURIO_PROFILE_BUILD=release` selects
the release binary.

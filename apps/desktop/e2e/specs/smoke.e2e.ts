// The five Phase-4 smoke scenarios (docs/design/roadmap.md §5). This is the
// authored skeleton driven by the nightly `smoke` job through tauri-driver on
// Linux + Windows. Scenarios that need stable frontend selectors or a live
// wiremock stub are `.skip` for now and documented inline; `boot` runs today.
//
// No backdoors: there is no __TAURI_TEST_RESET__ hook (D11). Each scenario
// drives the real webview + the real curio-core engine behind the IPC head,
// against a throwaway CURIO_PROFILE the harness points at a temp dir.

describe('Curio desktop — smoke', () => {
    // 1. Boot: the app launches and the three-pane shell is present.
    it('boots to the reader shell', async () => {
        await browser.waitUntil(async () => (await browser.getTitle()) !== '', {
            timeout: 30_000,
            timeoutMsg: 'window never acquired a title',
        });
        const shell = await browser.$('[data-testid="three-pane"]');
        await shell.waitForExist({ timeout: 30_000 });
    });

    // 2. Add a feed against a local wiremock (127.0.0.1 only — never the real
    //    network). Opens the add-feed modal, submits the stub URL, asserts the
    //    feed lands in the sidebar.
    it.skip('adds a feed served by a local wiremock stub', async () => {
        // Pending: a wiremock/127.0.0.1 stub started in onPrepare serving
        //   fixtures/feeds/rss2.xml, plus data-testid hooks on the add-feed
        //   modal input + submit. Asserts the new feed row renders.
    });

    // 3. Stored-XSS stays inert end-to-end: ingest fixtures/html/xss-corpus.html,
    //    open the article, assert the SanitizedHtml layer stripped every
    //    script/handler (no alert dialog, no <script> nodes in the DOM).
    it.skip('renders the xss-corpus inert through SanitizedHtml', async () => {
        // Pending: seed the corpus via the stub feed, then assert
        //   `browser.execute(() => document.querySelectorAll('script').length)`
        //   is 0 inside the reader pane and no dialog was raised.
    });

    // 4. Keyboard-only navigation: j/k move the selection in the article list.
    it.skip('moves selection with j/k', async () => {
        // Pending: seed >2 articles, focus the list, send 'j' then 'k', assert
        //   the [aria-selected="true"] row index advances and returns.
    });

    // 5. Promote-to-destination writes a schema-valid note: press `p`, choose a
    //    temp destination, assert a Markdown note with valid frontmatter lands.
    it.skip('promotes an article to a destination as a schema-valid note', async () => {
        // Pending: register a temp destination, select an article, send 'p',
        //   then assert the written note validates against
        //   schemas/curio.frontmatter.v1.json.
    });
});

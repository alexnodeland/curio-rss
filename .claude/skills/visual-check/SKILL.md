---
name: visual-check
description: Visually validate a desktop UI change — run the Svelte frontend in a plain browser against the dev mock IPC, capture Playwright screenshots across themes/densities/sizes, inspect them, and iterate until polished. Use for any change a user can see (components, styles, tokens, layout, reader rendering, empty/error/loading states).
---

# Visual check

The frontend runs without Tauri: in a plain browser, `+layout.ts` installs
`src/lib/dev/install-mock.ts`, a mock IPC backed by `src/lib/dev/preview-data.ts`
(feeds, folders, articles of every view mode). That makes the whole UI
screenshot-able headlessly.

## Loop

1. **Serve:** `cd apps/desktop && npm run dev -- --port 1420 --strictPort &`
   (wait for `ready`; reuse a running server).
2. **Capture** with Playwright (preinstalled; `require('playwright')`,
   `chromium.launch()` needs no options). A minimal script:

   ```js
   const { chromium } = require('playwright');
   (async () => {
     const browser = await chromium.launch();
     for (const scheme of ['light', 'dark']) {
       const page = await browser.newPage({ viewport: { width: 1280, height: 800 }, colorScheme: scheme });
       await page.goto('http://localhost:1420/');
       await page.waitForSelector('[role="listbox"]', { timeout: 15000 }).catch(() => {});
       await page.screenshot({ path: `shots/after-${scheme}.png` });
       // drive the interaction under test: page.keyboard.press('j'), page.click(...)
     }
     await browser.close();
   })();
   ```

   Capture **before** (on `origin/main`) and **after**; cover light + dark,
   the narrow layout (~900 px wide), and any state the change touches
   (empty, loading, error, long titles, RTL, a wide table…).
3. **Inspect** every PNG with the Read tool. Check alignment to the spacing
   grid, token use (no stray hex/px), contrast in both themes, focus rings,
   truncation, overflow, and that nothing else moved.
4. **Iterate** until it looks deliberate and native. Then run the gates
   (`just desktop-lint`, `just desktop-npm-test`, `just frontend-bans`).
5. **Report:** list the screenshots and what each verifies in the PR's
   Screenshots section (files can't be attached from a cloud session —
   describe them precisely).

Keep screenshots outside the repo (a scratch dir) — never commit them.
`prefers-reduced-motion` and `forced-colors` can be emulated with
`page.emulateMedia({ reducedMotion: 'reduce', forcedColors: 'active' })`.

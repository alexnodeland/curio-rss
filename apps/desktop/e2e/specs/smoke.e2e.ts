// The five Phase-4 smoke scenarios (docs/design/roadmap.md §5), driven by the
// nightly `smoke` job through tauri-driver on Linux + Windows. Every scenario
// runs — none is skipped — and the job is a hard gate.
//
// No backdoors: there is no __TAURI_TEST_RESET__ hook (D11). Each scenario
// drives the real webview + the real curio-core engine behind the IPC head,
// against the throwaway profile wdio.conf.ts seeded through the `curio` CLI:
// one feed (the 127.0.0.1 stub's /seed.xml, W1-exempted) with three articles,
// and one destination named `notes`. Scenarios share the one app session, so
// they run in order.
import { readFileSync, readdirSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import Ajv2020 from 'ajv/dist/2020.js';
import addFormats from 'ajv-formats';
import { parse as parseYaml } from 'yaml';
import { ADDED_FEED_PATH, SEEDED_TITLES, SEED_FEED_PATH, stubHits } from '../support/stub';

const here = fileURLToPath(new URL('.', import.meta.url));
const repoRoot = resolve(here, '../../../..');

function requiredEnv(name: string): string {
    const value = process.env[name];
    if (value === undefined || value === '') {
        throw new Error(`${name} is unset — run the suite through wdio.conf.ts`);
    }
    return value;
}

const stubBase = requiredEnv('CURIO_E2E_STUB');
const destination = requiredEnv('CURIO_E2E_DEST');

const feedRow = (url: string) => `[data-testid="feed-row"][data-feed-url="${url}"]`;
const articleRow = '[data-testid="article-row"]';
const selectedRow = `${articleRow}[aria-selected="true"]`;

/** The `aria-posinset` of the selected article row, or null if none. */
async function selectedPosition(): Promise<number | null> {
    const row = await $(selectedRow);
    if (!(await row.isExisting())) {
        return null;
    }
    return Number(await row.getAttribute('aria-posinset'));
}

async function waitForSelectedPosition(position: number): Promise<void> {
    await browser.waitUntil(async () => (await selectedPosition()) === position, {
        timeout: 10_000,
        timeoutMsg: `selection never reached row ${position}`,
    });
}

/** The selected row's title (textContent — see {@link selectedRowText}). */
async function selectedTitle(): Promise<string> {
    const title = await $(`${selectedRow} .row-title`);
    return String(await title.getProperty('textContent')).trim();
}

/** The selected row's text content (`getText` is visibility-filtered, and the
 *  title span's truncation hides it from WebKit's visible-text algorithm). */
async function selectedRowText(): Promise<string> {
    const row = await $(selectedRow);
    if (!(await row.isExisting())) {
        return '';
    }
    return String(await row.getProperty('textContent'));
}

/** Clicks the article row carrying this title. */
async function openArticle(title: string): Promise<void> {
    const row = await $(`${articleRow}*=${title}`);
    await row.waitForClickable({ timeout: 20_000 });
    await row.click();
    await browser.waitUntil(
        async () => (await selectedRowText()).includes(title),
        { timeout: 10_000, timeoutMsg: `"${title}" never became the selected row` },
    );
}

/** Every Markdown note under the destination (the `.curio/` manifest dir excluded). */
function notesIn(root: string): string[] {
    return readdirSync(root, { recursive: true, encoding: 'utf8' })
        .filter((entry) => entry.endsWith('.md') && !entry.split(/[\\/]/).includes('.curio'))
        .map((entry) => join(root, entry));
}

/** The YAML frontmatter block of a note, parsed. */
function frontmatter(note: string): unknown {
    const text = readFileSync(note, 'utf8').replace(/\r\n/g, '\n');
    const match = /^---\n([\s\S]*?)\n---\n/.exec(text);
    if (match === null) {
        throw new Error(`${note} has no frontmatter block`);
    }
    return parseYaml(match[1]);
}

describe('Curio desktop — smoke', () => {
    // 1. Boot: the app launches to the three-pane shell over the seeded profile.
    it('boots to the reader shell', async () => {
        await browser.waitUntil(async () => (await browser.getTitle()) !== '', {
            timeout: 30_000,
            timeoutMsg: 'window never acquired a title',
        });
        await $('[data-testid="three-pane"]').waitForExist({ timeout: 30_000 });
        await $(feedRow(`${stubBase}${SEED_FEED_PATH}`)).waitForExist({ timeout: 30_000 });
    });

    // 2. Add a feed through the add-feed modal, pointed at the 127.0.0.1 stub.
    //    The subscription lands in the sidebar — and because a UI-typed URL can
    //    never carry the W1 exemption, the SSRF guard refuses its first fetch:
    //    the row turns unhealthy and the stub never sees a request for it.
    it('adds a feed through the modal, and the SSRF guard keeps loopback unfetched', async () => {
        const url = `${stubBase}${ADDED_FEED_PATH}`;
        await $('[data-testid="three-pane"]').click();
        await browser.keys('a');
        const input = await $('[data-testid="add-feed-url"]');
        await input.waitForDisplayed({ timeout: 10_000 });
        await input.setValue(url);
        await $('[data-testid="add-feed-submit"]').click();

        const row = await $(feedRow(url));
        await row.waitForExist({ timeout: 20_000 });
        await browser.waitUntil(
            async () => ((await row.getAttribute('class')) ?? '').includes('unhealthy'),
            { timeout: 30_000, timeoutMsg: 'the refused first fetch never surfaced in feed health' },
        );
        const hits = await stubHits(stubBase);
        expect(hits[ADDED_FEED_PATH] ?? 0).toBe(0);
        // The seeded feed, by contrast, was fetched (by the CLI's seeding run).
        expect(hits[SEED_FEED_PATH] ?? 0).toBeGreaterThan(0);
    });

    // 3. Stored-XSS stays inert end-to-end: fixtures/html/xss-corpus.html went
    //    feed → ingest → DB → SanitizedHtml → the real webview. Every script,
    //    handler, dangerous URL and embedding element is gone; the legitimate
    //    content around them survives; no dialog fired and nothing navigated.
    it('renders the xss-corpus inert through SanitizedHtml', async () => {
        const appUrl = await browser.getUrl();
        // Sentinel: trap the dialog primitives the corpus reaches for before it
        // renders, so a payload that fires (even one the driver auto-dismisses)
        // leaves a record. A navigation or reload would drop the trap — the
        // `null` read-back below catches that too.
        await browser.execute(() => {
            const tripped: string[] = [];
            Object.defineProperty(window, '__curioXssTripped', { value: tripped });
            for (const name of ['alert', 'confirm', 'prompt']) {
                Object.defineProperty(window, name, {
                    configurable: true,
                    value: (...args: unknown[]) => {
                        tripped.push(`${name}(${args.map(String).join(', ')})`);
                    },
                });
            }
        });
        await $(`${feedRow(`${stubBase}${SEED_FEED_PATH}`)} .feed-select`).click();
        await openArticle(SEEDED_TITLES[0]);

        const content = await $('[data-testid="reader-content"]');
        await content.waitForExist({ timeout: 20_000 });
        await browser.waitUntil(async () => (await content.getText()).includes('survives'), {
            timeout: 20_000,
            timeoutMsg: 'the corpus body never rendered',
        });

        const findings = await browser.execute(() => {
            const root = document.querySelector('[data-testid="reader-content"]');
            if (root === null) {
                return ['reader content vanished'];
            }
            const found: string[] = [];
            const banned = 'script, iframe, object, embed, form, base, meta, style, link';
            for (const element of root.querySelectorAll(banned)) {
                found.push(`<${element.tagName.toLowerCase()}>`);
            }
            for (const element of root.querySelectorAll('*')) {
                for (const attribute of element.attributes) {
                    const value = attribute.value.trim().toLowerCase();
                    if (attribute.name.toLowerCase().startsWith('on')) {
                        found.push(`${element.tagName.toLowerCase()}[${attribute.name}]`);
                    } else if (value.startsWith('javascript:') || value.startsWith('data:text/html')) {
                        found.push(`${element.tagName.toLowerCase()}[${attribute.name}=${value}]`);
                    }
                }
            }
            return found;
        });
        expect(findings).toEqual([]);
        expect(await content.getText()).toContain('Legitimate content around the attacks');

        let dialog: string | null = null;
        try {
            dialog = await browser.getAlertText();
        } catch {
            dialog = null; // "no such alert" — the expected outcome
        }
        expect(dialog).toBeNull();
        const tripped = await browser.execute(
            () =>
                (window as Window & { __curioXssTripped?: string[] }).__curioXssTripped ?? null,
        );
        expect(tripped).toEqual([]);
        expect(await browser.getUrl()).toBe(appUrl);
    });

    // 4. Keyboard-only navigation: j/k move the article-list selection (down,
    //    down, back up) over the seeded feed's three rows.
    it('moves selection with j/k', async () => {
        const first = await $(`${articleRow}[aria-posinset="1"]`);
        await first.waitForClickable({ timeout: 10_000 });
        await first.click();
        await waitForSelectedPosition(1);
        const top = await selectedTitle();
        await browser.keys('j');
        await waitForSelectedPosition(2);
        await browser.keys('j');
        await waitForSelectedPosition(3);
        await browser.keys('k');
        await waitForSelectedPosition(2);
        const titles = [top, await selectedTitle()];
        expect(new Set(titles).size).toBe(2);
        for (const title of titles) {
            expect(SEEDED_TITLES).toContain(title);
        }
    });

    // 5. Promote-to-destination: `p` on the selected article writes a Markdown
    //    note into the sole destination whose frontmatter validates against the
    //    published schemas/frontmatter.v1.json.
    it('promotes an article to a destination as a schema-valid note', async () => {
        expect(notesIn(destination)).toEqual([]);
        const title = await selectedTitle();
        expect(SEEDED_TITLES).toContain(title);
        await browser.keys('p');
        await browser.waitUntil(async () => notesIn(destination).length === 1, {
            timeout: 20_000,
            timeoutMsg: 'no note landed in the destination',
        });

        const schema = JSON.parse(readFileSync(join(repoRoot, 'schemas/frontmatter.v1.json'), 'utf8'));
        const ajv = new Ajv2020({ allErrors: true });
        addFormats(ajv);
        const validate = ajv.compile(schema);
        const data = frontmatter(notesIn(destination)[0]);
        const valid = validate(data);
        expect(validate.errors ?? []).toEqual([]);
        expect(valid).toBe(true);
        expect(data).toMatchObject({
            schema: 'curio.frontmatter.v1',
            title,
            feed: `${stubBase}${SEED_FEED_PATH}`,
        });
    });
});

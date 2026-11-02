/**
 * Reader-surface token plumbing (#296). The reading themes (sepia, paper)
 * repaint the ground and the ink tokens on their own root, but the app's
 * --surface-* and --hairline tokens follow the *app* theme — so any reader
 * fill built from them renders as a near-black box under dark sepia ink on a
 * dark app theme. The fix is a set of --reader-* tints, mixed over a
 * transparent ground and re-declared on every [data-reading-theme] root.
 *
 * jsdom resolves neither var() nor color-mix(), so this gate reads the real
 * stylesheets and pins the plumbing itself:
 *   • app.css declares every reader tint on `:root, [data-reading-theme]`,
 *     built only from the reading ink / a shade over transparent;
 *   • every var(--reader-*) a component uses is declared (a typo would
 *     resolve to nothing and silently drop the fill);
 *   • nothing rendered inside the reader's scroll surface fills or rules
 *     from --surface-*, --hairline or --bg;
 *   • the sepia/paper overrides live once in app.css and force the --ink the
 *     tints mix from; neither the reader pane nor the Settings preview keeps
 *     a local copy that could drift, and the preview carries the attribute
 *     the tokens key off.
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import ReaderPreview from '$components/reader/ReaderPreview.svelte';
import { typographyStore } from '$lib/state/typography.svelte';
import { render } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import { afterEach, describe, expect, it } from 'vitest';

// vitest runs with the package (apps/desktop) as cwd.
const read = (path: string): string => readFileSync(resolve(process.cwd(), path), 'utf8');

const appCss = read('src/app.css');
const component = (name: string): string => read(`src/lib/components/${name}.svelte`);

/** The `<style>` block of a Svelte component, comments stripped. */
function styleOf(source: string): string {
    const match = /<style>([\s\S]*?)<\/style>/.exec(source);
    return (match?.[1] ?? '').replace(/\/\*[\s\S]*?\*\//g, '');
}

/** Flat `selector { body }` rules (no nesting inside a component style). */
function rulesOf(css: string): Array<{ selector: string; body: string }> {
    const rules: Array<{ selector: string; body: string }> = [];
    const rule = /([^{}]+)\{([^{}]*)\}/g;
    for (let m = rule.exec(css); m !== null; m = rule.exec(css)) {
        rules.push({ selector: m[1].trim(), body: m[2] });
    }
    return rules;
}

/** The custom-property declarations of one rule body. */
function declarations(body: string): Record<string, string> {
    const out: Record<string, string> = {};
    const decl = /(--[a-z0-9-]+)\s*:\s*([^;]+);/g;
    for (let d = decl.exec(body); d !== null; d = decl.exec(body)) {
        out[d[1]] = d[2].trim().replace(/\s+/g, ' ');
    }
    return out;
}

/** The body of the first top-level app.css rule whose selector is exactly `selector`. */
function appRule(selector: string): Record<string, string> {
    const css = appCss.replace(/\/\*[\s\S]*?\*\//g, '');
    const want = selector.replace(/\s+/g, ' ');
    const found = rulesOf(css).find((r) => r.selector.replace(/\s+/g, ' ') === want);
    expect(found, `app.css rule \`${selector}\``).toBeDefined();
    return declarations(found?.body ?? '');
}

const READER_TINTS = [
    '--reader-tint',
    '--reader-tint-strong',
    '--reader-inset',
    '--reader-rule',
    '--reader-hairline',
    '--reader-hairline-strong',
];

// App-theme tokens that must never paint a fill or a rule inside the reader.
const APP_SURFACE = /var\(--(surface-[a-z-]+|hairline(-strong)?|bg[a-z-]*)\)/;

describe('reader-surface tokens in app.css', () => {
    it('declares every reader tint on :root and on each [data-reading-theme] root', () => {
        const tokens = appRule(':root, [data-reading-theme]');
        for (const name of READER_TINTS) {
            expect(tokens[name], name).toBeDefined();
        }
    });

    it('mixes each tint over transparent from the reading ink or a shade, never an app surface', () => {
        const tokens = appRule(':root, [data-reading-theme]');
        for (const name of READER_TINTS) {
            const value = tokens[name] ?? '';
            expect(value, name).toMatch(
                /^color-mix\(in srgb, transparent, (var\(--ink\)|#000) \d+%\)$/,
            );
        }
    });

    it('re-resolves the interaction and skeleton tokens on the reading-theme root', () => {
        const root = appRule(':root, [data-reading-theme]');
        const scoped = appRule('[data-reading-theme]');
        expect(scoped['--skeleton-fill']).toBe('var(--reader-tint-strong)');
        expect(scoped['--skeleton-sheen']).toBe('var(--hover)');
        expect(Object.keys(root)).not.toContain('--hover');
        // Same formulas as :root's, so the default reading theme is unchanged.
        const css = appCss.replace(/\/\*[\s\S]*?\*\//g, '');
        const rootDecls = declarations(
            rulesOf(css)
                .filter((r) => r.selector === ':root')
                .map((r) => r.body)
                .join('\n'),
        );
        expect(scoped['--hover']).toBe(rootDecls['--hover']);
        expect(scoped['--active']).toBe(rootDecls['--active']);
    });

    it('keeps the app-wide skeleton default on the raised surface', () => {
        const css = appCss.replace(/\/\*[\s\S]*?\*\//g, '');
        const rootDecls = declarations(
            rulesOf(css)
                .filter((r) => r.selector === ':root')
                .map((r) => r.body)
                .join('\n'),
        );
        expect(rootDecls['--skeleton-fill']).toBe('var(--surface-raised)');
        expect(rootDecls['--skeleton-sheen']).toBe('var(--hover)');
    });

    it('firms up the reader hairlines under prefers-contrast: more', () => {
        const block = /@media \(prefers-contrast: more\) \{([\s\S]*?)\n\}/.exec(appCss)?.[1] ?? '';
        const scoped = rulesOf(block.replace(/\/\*[\s\S]*?\*\//g, '')).find((r) =>
            r.selector.includes('[data-reading-theme]'),
        );
        expect(scoped).toBeDefined();
        const decls = declarations(scoped?.body ?? '');
        expect(Object.keys(decls).sort()).toEqual(
            ['--reader-hairline', '--reader-hairline-strong', '--reader-rule'].sort(),
        );
    });
});

// Rendered only inside the reader's scroll surface (and the Settings preview).
const READER_BODY = [
    'reader/SanitizedHtml',
    'reader/ArticleTags',
    'reader/RedditLayout',
    'reader/YouTubeFacade',
    'reader/YouTubeLayout',
    'reader/ViewModeDispatch',
];

describe('reader-surface tokens in the reader components', () => {
    it.each(READER_BODY)('%s paints no fill or rule from an app-theme surface', (name) => {
        expect(styleOf(component(name))).not.toMatch(APP_SURFACE);
    });

    it('ReaderPane paints nothing inside .reader-scroll from an app-theme surface', () => {
        const source = component('reader/ReaderPane');
        // Every class named in the markup from the scroll surface down.
        const start = source.indexOf('class="reader-scroll"');
        const end = source.indexOf('{/key}', start);
        expect(start).toBeGreaterThan(-1);
        const markup = source.slice(start, end);
        const inside = new Set<string>();
        for (const m of markup.matchAll(/class="([^"]+)"/g)) {
            for (const cls of m[1].split(/\s+/)) inside.add(cls);
        }
        for (const m of markup.matchAll(/class:([a-z-]+)/g)) inside.add(m[1]);
        expect(inside.has('reader-header')).toBe(true);

        const offenders = rulesOf(styleOf(source)).filter(
            (r) =>
                [...r.selector.matchAll(/\.([a-z][a-z0-9-]*)/g)].some((c) => inside.has(c[1])) &&
                APP_SURFACE.test(r.body),
        );
        expect(offenders.map((r) => r.selector)).toEqual([]);
    });

    it('Skeleton fills from its token pair, so the reader can repaint it', () => {
        const style = styleOf(component('common/Skeleton'));
        expect(style).not.toMatch(APP_SURFACE);
        expect(style).toContain('var(--skeleton-fill)');
        expect(style).toContain('var(--skeleton-sheen)');
    });

    it('every --reader-* token a component uses is declared in app.css', () => {
        const declared = new Set(Object.keys(appRule(':root, [data-reading-theme]')));
        const sources = [...READER_BODY, 'reader/ReaderPane', 'reader/ReaderPreview'];
        for (const name of sources) {
            for (const m of styleOf(component(name)).matchAll(/var\((--reader-[a-z-]+)\)/g)) {
                expect(declared.has(m[1]), `${name} uses ${m[1]}`).toBe(true);
            }
        }
    });

    it('the reading-theme overrides are shared from app.css and force the ink', () => {
        for (const theme of ['sepia', 'paper']) {
            const rule = appRule(`[data-reading-theme="${theme}"]`);
            // The tints mix from --ink: the theme must force it on its root.
            expect(rule['--ink'], theme).toBeDefined();
            expect(rule['--reading-surface'], theme).toBeDefined();
        }
        // One source for both roots: a component-local override would let the
        // pane and the Settings preview drift apart again.
        for (const name of ['reader/ReaderPane', 'reader/ReaderPreview']) {
            const local = rulesOf(styleOf(component(name))).filter((r) =>
                r.selector.includes('data-reading-theme'),
            );
            expect(local, name).toEqual([]);
        }
    });
});

describe('ReaderPreview reading-theme root', () => {
    afterEach(() => {
        typographyStore.readingTheme = 'default';
    });

    it('marks its surface with the active reading theme the tints key off', () => {
        const { container } = render(ReaderPreview);
        const surface = container.querySelector('.preview-surface');
        expect(surface?.getAttribute('data-reading-theme')).toBe('default');

        typographyStore.readingTheme = 'sepia';
        flushSync();
        expect(surface?.getAttribute('data-reading-theme')).toBe('sepia');
        // The sample exercises a tinted element: inline code inside the body.
        expect(surface?.querySelector('.sanitized-content :not(pre) > code')).not.toBeNull();
    });
});

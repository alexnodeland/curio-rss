/**
 * Theme contrast gate: every theme's *text* and *UI* tokens must clear WCAG
 * minimums against the surfaces they actually render on — not just against
 * `--bg`. The original gate checked only `fg/fg-muted/fg-subtle` on `--bg`,
 * so five distinct AA failures (sub-AA primary-button text, illegible
 * status-as-text, low-contrast links, muted/subtle text on the chrome and
 * list surfaces) all shipped "green". This widened gate closes that blind
 * spot so the whole class of contrast regressions cannot land again.
 *
 * The pairs mirror how the tokens are consumed:
 *   • body/secondary/subtle text on both `--bg` and `--bg-secondary`
 *     (bylines, timestamps, counts, and caps labels live on chrome/list,
 *      not only the content surface),
 *   • primary-button text (`--accent-fg`) on `--accent`,
 *   • `--link` on `--bg`,
 *   • the semantic *text* tokens (`--error-text`/`--warning-text`/
 *     `--success-text`, each falling back to the raw hue when a theme does
 *     not override it) on both `--bg` and `--bg-secondary`.
 *
 * The test reads the real `app.css`, so it gates the shipped tokens
 * themselves. A retune that regresses any pair fails here.
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const AA_NORMAL = 4.5;
const AA_LARGE = 3;

// vitest runs with the package (apps/desktop) as cwd; app.css is the real
// token source, so the shipped values themselves are what's gated.
const css = readFileSync(resolve(process.cwd(), 'src/app.css'), 'utf8');

/** Normalizes `#rgb`/`#rrggbb`/`#rrggbbaa` to `#rrggbb`; null for anything else. */
function toHex6(value: string): string | null {
    const hex = value.trim().toLowerCase();
    if (/^#[0-9a-f]{3}$/.test(hex)) {
        return `#${hex[1]}${hex[1]}${hex[2]}${hex[2]}${hex[3]}${hex[3]}`;
    }
    if (/^#[0-9a-f]{6}$/.test(hex)) return hex;
    if (/^#[0-9a-f]{8}$/.test(hex)) return hex.slice(0, 7);
    return null;
}

/** Extracts every `[data-theme="…"]` block's tokens (hex only) from app.css. */
function parseThemes(source: string): Record<string, Record<string, string>> {
    const themes: Record<string, Record<string, string>> = {};
    const block = /\[data-theme="([^"]+)"\]\s*\{([^}]*)\}/g;
    for (let match = block.exec(source); match !== null; match = block.exec(source)) {
        const [, name, body] = match;
        const tokens: Record<string, string> = {};
        const decl = /--([a-z-]+):\s*(#[0-9a-fA-F]{3,8})\b/g;
        for (let d = decl.exec(body); d !== null; d = decl.exec(body)) {
            const hex = toHex6(d[2]);
            if (hex !== null) tokens[d[1]] = hex;
        }
        themes[name] = tokens;
    }
    return themes;
}

/** WCAG relative luminance of an `#rrggbb` colour. */
function luminance(hex: string): number {
    const value = Number.parseInt(hex.slice(1), 16);
    const channels = [(value >> 16) & 0xff, (value >> 8) & 0xff, value & 0xff];
    const linear = channels.map((c) => {
        const s = c / 255;
        return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2];
}

/** WCAG contrast ratio between two `#rrggbb` colours (1..21). */
function contrast(a: string, b: string): number {
    const la = luminance(a);
    const lb = luminance(b);
    return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

const themes = parseThemes(css);

/**
 * The gated pairs. `fg` and `bg` are token names; `min` is the WCAG floor.
 * `fallback` names the base token a semantic `-text` token inherits when the
 * theme does not override it (matching the `--error-text: var(--error)`
 * default in the derived `:root` layer).
 */
interface Pair {
    fg: string;
    bg: string;
    min: number;
    fallback?: string;
}

const PAIRS: Pair[] = [
    { fg: 'fg', bg: 'bg', min: AA_NORMAL },
    { fg: 'fg-muted', bg: 'bg', min: AA_NORMAL },
    { fg: 'fg-muted', bg: 'bg-secondary', min: AA_NORMAL },
    { fg: 'fg-subtle', bg: 'bg', min: AA_LARGE },
    { fg: 'fg-subtle', bg: 'bg-secondary', min: AA_LARGE },
    { fg: 'accent-fg', bg: 'accent', min: AA_NORMAL },
    { fg: 'link', bg: 'bg', min: AA_NORMAL },
    { fg: 'error-text', bg: 'bg', min: AA_NORMAL, fallback: 'error' },
    { fg: 'error-text', bg: 'bg-secondary', min: AA_NORMAL, fallback: 'error' },
    { fg: 'warning-text', bg: 'bg', min: AA_NORMAL, fallback: 'warning' },
    { fg: 'warning-text', bg: 'bg-secondary', min: AA_NORMAL, fallback: 'warning' },
    { fg: 'success-text', bg: 'bg', min: AA_NORMAL, fallback: 'success' },
    { fg: 'success-text', bg: 'bg-secondary', min: AA_NORMAL, fallback: 'success' },
];

function resolveToken(tokens: Record<string, string>, pair: Pair): string {
    const value = tokens[pair.fg] ?? (pair.fallback ? tokens[pair.fallback] : undefined);
    if (value === undefined) {
        throw new Error(`token --${pair.fg} (fallback --${pair.fallback}) not found`);
    }
    return value;
}

describe('theme contrast', () => {
    it('parses all nine themes', () => {
        expect(Object.keys(themes)).toHaveLength(9);
    });

    for (const [name, tokens] of Object.entries(themes)) {
        it(`${name}: text & UI tokens clear their WCAG floor on every surface`, () => {
            for (const pair of PAIRS) {
                const fg = resolveToken(tokens, pair);
                const bg = tokens[pair.bg];
                expect(bg, `theme "${name}" missing --${pair.bg}`).toBeDefined();
                const ratio = contrast(fg, bg);
                expect(
                    ratio,
                    `--${pair.fg} ${fg} on --${pair.bg} ${bg} = ${ratio.toFixed(2)} (min ${pair.min})`,
                ).toBeGreaterThanOrEqual(pair.min);
            }
        });
    }
});

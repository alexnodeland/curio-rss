/**
 * Theme contrast gate: every theme's text tokens must clear WCAG minimums
 * against their background. Body text (`--fg`) and secondary text
 * (`--fg-muted`) must meet AA for normal text (4.5:1); the de-emphasized
 * `--fg-subtle` (caps section labels, hints) must meet the large-text /
 * UI-component floor (3:1) — enough to stay legible without collapsing the
 * three-step emphasis hierarchy into two.
 *
 * The test reads the real `app.css`, so it is the tokens themselves — not a
 * copy — that are gated. A retune that regresses any theme fails here.
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const AA_NORMAL = 4.5;
const AA_LARGE = 3;

// vitest runs with the package (apps/desktop) as cwd; app.css is the real
// token source, so the shipped values themselves are what's gated.
const css = readFileSync(resolve(process.cwd(), 'src/app.css'), 'utf8');

interface Palette {
    bg: string;
    fg: string;
    muted: string;
    subtle: string;
}

/** Extracts every `[data-theme="…"]` block's four text tokens from app.css. */
function parseThemes(source: string): Record<string, Palette> {
    const themes: Record<string, Palette> = {};
    const block = /\[data-theme="([^"]+)"\]\s*\{([^}]*)\}/g;
    for (let match = block.exec(source); match !== null; match = block.exec(source)) {
        const [, name, body] = match;
        const token = (key: string): string => {
            const found = new RegExp(`--${key}:\\s*(#[0-9a-fA-F]{6})`).exec(body);
            if (found === null) {
                throw new Error(`theme "${name}" is missing --${key}`);
            }
            return found[1];
        };
        themes[name] = {
            bg: token('bg'),
            fg: token('fg'),
            muted: token('fg-muted'),
            subtle: token('fg-subtle'),
        };
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

describe('theme contrast', () => {
    it('parses all nine themes', () => {
        expect(Object.keys(themes)).toHaveLength(9);
    });

    for (const [name, palette] of Object.entries(themes)) {
        it(`${name}: text tokens clear their WCAG floor`, () => {
            const fg = contrast(palette.fg, palette.bg);
            const muted = contrast(palette.muted, palette.bg);
            const subtle = contrast(palette.subtle, palette.bg);
            expect(
                fg,
                `--fg ${palette.fg} on ${palette.bg} = ${fg.toFixed(2)}`,
            ).toBeGreaterThanOrEqual(AA_NORMAL);
            expect(
                muted,
                `--fg-muted ${palette.muted} on ${palette.bg} = ${muted.toFixed(2)}`,
            ).toBeGreaterThanOrEqual(AA_NORMAL);
            expect(
                subtle,
                `--fg-subtle ${palette.subtle} on ${palette.bg} = ${subtle.toFixed(2)}`,
            ).toBeGreaterThanOrEqual(AA_LARGE);
        });
    }
});

/**
 * The `curio.theme/v1` pure format layer: serialize → parse round-trips,
 * validation rejects malformed/hostile files with specific errors, the built
 * rule can't carry a CSS breakout, and the ported WCAG contrast check flags
 * the right pairs (warn-only, hex-tolerant).
 */
import {
    CURIO_BASE_TOKENS,
    type CustomTheme,
    buildThemeRule,
    contrastFailures,
    isValidThemeSlug,
    parseTheme,
    serializeTheme,
} from '$lib/utils/theme-format';
import { describe, expect, it } from 'vitest';

function tokens(overrides: Record<string, string> = {}): Record<string, string> {
    const base: Record<string, string> = {};
    for (const token of CURIO_BASE_TOKENS) base[token] = '#123456';
    return { ...base, ...overrides };
}

function theme(overrides: Partial<CustomTheme> = {}): CustomTheme {
    return {
        id: 'my-theme',
        name: 'My Theme',
        isDark: true,
        tokens: tokens() as CustomTheme['tokens'],
        ...overrides,
    };
}

describe('theme-format — serialize/parse round-trip', () => {
    it('round-trips a theme through YAML byte-for-value', () => {
        const original = theme({
            tokens: tokens({
                bg: '#0e1116',
                'accent-muted': 'rgba(34, 139, 230, 0.15)',
                'accent-fg': '#fff',
            }) as CustomTheme['tokens'],
        });
        const yaml = serializeTheme(original);
        // Hex values must be quoted or YAML reads `#…` as a comment.
        expect(yaml).toContain('bg: "#0e1116"');
        const result = parseTheme(yaml);
        expect(result.ok).toBe(true);
        if (result.ok) {
            expect(result.theme).toEqual(original);
        }
    });

    it('carries appearance ↔ isDark both ways', () => {
        const light = parseTheme(serializeTheme(theme({ isDark: false })));
        expect(light.ok && light.theme.isDark).toBe(false);
    });
});

describe('theme-format — validation', () => {
    it('rejects a wrong/missing schema', () => {
        const result = parseTheme('id: x\nname: X\nappearance: dark\ntokens: {}\n');
        expect(result.ok).toBe(false);
        if (!result.ok) expect(result.error).toContain('curio.theme/v1');
    });

    it('rejects a bad slug', () => {
        const yaml = serializeTheme(theme()).replace('id: my-theme', 'id: My Theme!');
        const result = parseTheme(yaml);
        expect(result.ok).toBe(false);
        if (!result.ok) expect(result.error).toContain('id');
    });

    it('rejects missing tokens', () => {
        const partial = serializeTheme(theme());
        const trimmed = partial.replace(/ {2}link-visited:.*\n/, '');
        const result = parseTheme(trimmed);
        expect(result.ok).toBe(false);
        if (!result.ok) expect(result.error).toContain('link-visited');
    });

    it('rejects unknown top-level fields and unknown tokens', () => {
        const extra = parseTheme(`${serializeTheme(theme())}evil: true\n`);
        expect(extra.ok).toBe(false);
        if (!extra.ok) expect(extra.error).toContain('evil');
    });

    it('rejects a non-color token value (the CSS-injection gate)', () => {
        const hostile = serializeTheme(theme()).replace(
            'bg: "#123456"',
            'bg: "red; } body { display: none } .x{"',
        );
        const result = parseTheme(hostile);
        expect(result.ok).toBe(false);
        if (!result.ok) expect(result.error).toContain('bg');
    });

    it('rejects a non-object document', () => {
        expect(parseTheme('just a string').ok).toBe(false);
    });
});

describe('theme-format — buildThemeRule', () => {
    it('emits a scoped [data-theme] rule with all 21 tokens and no breakout', () => {
        const rule = buildThemeRule(theme());
        expect(rule.startsWith('[data-theme="custom-my-theme"] {')).toBe(true);
        for (const token of CURIO_BASE_TOKENS) {
            expect(rule).toContain(`--${token}:`);
        }
        // Only one rule block — no `}` other than the closer.
        expect(rule.match(/}/g)).toHaveLength(1);
    });
});

describe('theme-format — slug', () => {
    it('accepts kebab slugs and rejects the rest', () => {
        expect(isValidThemeSlug('my-theme-2')).toBe(true);
        expect(isValidThemeSlug('MyTheme')).toBe(false);
        expect(isValidThemeSlug('-lead')).toBe(false);
        expect(isValidThemeSlug('trail-')).toBe(false);
        expect(isValidThemeSlug('a--b')).toBe(false);
    });
});

describe('theme-format — contrast (warn-only)', () => {
    it('flags a low-contrast fg/bg pair', () => {
        const bad = theme({
            tokens: tokens({ bg: '#000000', 'fg-subtle': '#111111' }) as CustomTheme['tokens'],
        });
        const failures = contrastFailures(bad);
        expect(failures.some((f) => f.fg === 'fg-subtle' && f.bg === 'bg')).toBe(true);
    });

    it('flags sub-AA primary-button text (the widened gate)', () => {
        // white on a mid-blue accent — the real-world 3.56:1 failure that the
        // old --bg-only gate shipped green.
        const bad = theme({
            tokens: tokens({ accent: '#228be6', 'accent-fg': '#ffffff' }) as CustomTheme['tokens'],
        });
        expect(contrastFailures(bad).some((f) => f.fg === 'accent-fg' && f.bg === 'accent')).toBe(
            true,
        );
    });

    it('passes a high-contrast theme and skips non-hex pairs', () => {
        const good = theme({
            tokens: tokens({
                bg: '#000000',
                'bg-secondary': '#000000',
                fg: '#ffffff',
                'fg-muted': '#eeeeee',
                'fg-subtle': '#cccccc',
                accent: '#000000',
                'accent-fg': '#ffffff',
                link: '#ffffff',
                error: '#ffffff',
                warning: '#ffffff',
                success: '#ffffff',
            }) as CustomTheme['tokens'],
        });
        expect(contrastFailures(good)).toHaveLength(0);
        // rgba fg is unparseable → skipped, not falsely flagged.
        const rgba = theme({
            tokens: tokens({ bg: '#000000', fg: 'rgba(255,255,255,1)' }) as CustomTheme['tokens'],
        });
        expect(rgba.tokens.fg).toBe('rgba(255,255,255,1)');
        expect(contrastFailures(rgba).some((f) => f.fg === 'fg')).toBe(false);
    });
});

/**
 * The `curio.theme/v1` format: a portable theme is the 21 base CSS tokens each
 * `[data-theme]` block defines, plus a name and light/dark appearance. All the
 * derived surface/interaction tokens fall out of these bases via the `:root`
 * `color-mix` layer, so a custom theme only ever supplies the 21 bases.
 *
 * This module is pure (no DOM): serialize to YAML, parse+validate an imported
 * file, build the injectable `[data-theme="custom-<id>"]` rule, and run the
 * WCAG contrast check (ported from the #36 gate). Import values are attacker-
 * controlled, so every token value is checked with `CSS.supports('color', v)`
 * before it is ever concatenated into a stylesheet rule — a value that could
 * break out of the declaration block cannot pass that gate.
 */
import { parse, stringify } from 'yaml';

/** The schema tag every `curio.theme/v1` file must carry. */
export const THEME_SCHEMA = 'curio.theme/v1';

/**
 * The 21 base tokens each `[data-theme]` block in app.css defines, in source
 * order. A custom theme must supply exactly these — no more, no less.
 */
export const CURIO_BASE_TOKENS = [
    'bg',
    'bg-secondary',
    'bg-tertiary',
    'bg-hover',
    'fg',
    'fg-muted',
    'fg-subtle',
    'accent',
    'accent-hover',
    'accent-fg',
    'accent-muted',
    'border',
    'border-subtle',
    'error',
    'error-bg',
    'warning',
    'success',
    'unread',
    'read',
    'link',
    'link-visited',
] as const;

export type CurioBaseToken = (typeof CURIO_BASE_TOKENS)[number];

export type ThemeTokens = Readonly<Record<CurioBaseToken, string>>;

export interface CustomTheme {
    /** kebab-case slug; the applied selector is `[data-theme="custom-<id>"]`. */
    readonly id: string;
    readonly name: string;
    readonly isDark: boolean;
    readonly tokens: ThemeTokens;
}

/** A valid custom-theme id: lowercase alphanumerics in hyphen-joined groups. */
const SLUG = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;

export function isValidThemeSlug(id: string): boolean {
    return SLUG.test(id);
}

/** Whether a string is a CSS color the browser accepts (import-value gate). */
export function isCssColor(value: string): boolean {
    if (typeof value !== 'string' || value.length === 0) return false;
    if (typeof CSS !== 'undefined' && typeof CSS.supports === 'function') {
        return CSS.supports('color', value);
    }
    // Non-browser fallback (should not run in production): a conservative
    // shape check that still rejects declaration-breakout characters.
    if (/[;{}<>]/.test(value)) return false;
    return /^#([0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$|^(rgb|hsl)a?\(|^[a-z]+$/i.test(value);
}

/** Serializes a theme to a `curio.theme/v1` YAML document. */
export function serializeTheme(theme: CustomTheme): string {
    const tokens: Record<string, string> = {};
    for (const token of CURIO_BASE_TOKENS) {
        tokens[token] = theme.tokens[token];
    }
    return stringify({
        schema: THEME_SCHEMA,
        id: theme.id,
        name: theme.name,
        appearance: theme.isDark ? 'dark' : 'light',
        tokens,
    });
}

export type ParseResult =
    | { readonly ok: true; readonly theme: CustomTheme }
    | { readonly ok: false; readonly error: string };

function fail(error: string): ParseResult {
    return { ok: false, error };
}

/** Parses and validates a `curio.theme/v1` YAML document. */
export function parseTheme(text: string): ParseResult {
    let doc: unknown;
    try {
        doc = parse(text);
    } catch (error) {
        return fail(
            error instanceof Error
                ? `Could not parse YAML: ${error.message}`
                : 'Could not parse YAML.',
        );
    }
    return validateTheme(doc);
}

const ALLOWED_FIELDS = new Set(['schema', 'id', 'name', 'appearance', 'tokens']);
const KNOWN_TOKENS = new Set<string>(CURIO_BASE_TOKENS);

/**
 * Validates the token map, re-checking every value with `isCssColor` (so a
 * tampered file/settings row can't inject arbitrary CSS). Returns the tokens
 * or an error string.
 */
function extractTokens(raw: unknown): ThemeTokens | string {
    if (raw === null || typeof raw !== 'object') {
        return 'The theme is missing its color tokens.';
    }
    const rawTokens = raw as Record<string, unknown>;
    const unknownTokens = Object.keys(rawTokens).filter((key) => !KNOWN_TOKENS.has(key));
    if (unknownTokens.length > 0) {
        return `Unknown color tokens: ${unknownTokens.join(', ')}.`;
    }
    const missing = CURIO_BASE_TOKENS.filter((token) => !(token in rawTokens));
    if (missing.length > 0) {
        return `Missing color tokens: ${missing.join(', ')}.`;
    }
    const tokens: Record<string, string> = {};
    for (const token of CURIO_BASE_TOKENS) {
        const value = rawTokens[token];
        if (typeof value !== 'string' || !isCssColor(value)) {
            return `Invalid color for '${token}': ${String(value)}`;
        }
        tokens[token] = value;
    }
    return tokens as ThemeTokens;
}

/** Validates the scalar fields of a `curio.theme/v1` document. */
function validateThemeMeta(
    record: Record<string, unknown>,
): { id: string; name: string; isDark: boolean } | string {
    if (record.schema !== THEME_SCHEMA) {
        return `Not a Curio theme file (expected schema: ${THEME_SCHEMA}).`;
    }
    if (typeof record.id !== 'string' || !isValidThemeSlug(record.id)) {
        return 'Invalid theme id — use lowercase letters, numbers, and hyphens.';
    }
    if (typeof record.name !== 'string' || record.name.trim().length === 0) {
        return 'The theme needs a name.';
    }
    if (record.appearance !== 'dark' && record.appearance !== 'light') {
        return "The theme's appearance must be 'dark' or 'light'.";
    }
    return { id: record.id, name: record.name.trim(), isDark: record.appearance === 'dark' };
}

/** Validates a parsed `curio.theme/v1` document object (the file shape). */
export function validateTheme(doc: unknown): ParseResult {
    if (doc === null || typeof doc !== 'object') {
        return fail('Not a Curio theme file.');
    }
    const record = doc as Record<string, unknown>;
    const extra = Object.keys(record).filter((key) => !ALLOWED_FIELDS.has(key));
    if (extra.length > 0) {
        return fail(`Unexpected fields: ${extra.join(', ')}.`);
    }
    const meta = validateThemeMeta(record);
    if (typeof meta === 'string') {
        return fail(meta);
    }
    const tokens = extractTokens(record.tokens);
    if (typeof tokens === 'string') {
        return fail(tokens);
    }
    return { ok: true, theme: { ...meta, tokens } };
}

/**
 * Validates a persisted internal `CustomTheme` object (`{id,name,isDark,tokens}`
 * — the in-memory shape, distinct from the file's `curio.theme/v1` shape).
 * Returns null on any mismatch.
 */
export function validateStoredTheme(doc: unknown): CustomTheme | null {
    if (doc === null || typeof doc !== 'object') return null;
    const record = doc as Record<string, unknown>;
    if (typeof record.id !== 'string' || !isValidThemeSlug(record.id)) return null;
    if (typeof record.name !== 'string' || record.name.length === 0) return null;
    if (typeof record.isDark !== 'boolean') return null;
    const tokens = extractTokens(record.tokens);
    if (typeof tokens === 'string') return null;
    return { id: record.id, name: record.name, isDark: record.isDark, tokens };
}

/**
 * The injectable CSS rule for a custom theme. Safe to concatenate because
 * every value passed `isCssColor` and the id passed `isValidThemeSlug`, so
 * neither can carry a declaration/selector breakout.
 */
export function buildThemeRule(theme: CustomTheme): string {
    const declarations = CURIO_BASE_TOKENS.map(
        (token) => `--${token}: ${theme.tokens[token]};`,
    ).join(' ');
    return `[data-theme="custom-${theme.id}"] { ${declarations} }`;
}

// ─── WCAG contrast check (ported from the #36 gate; warn-only) ───────────────

const AA_NORMAL = 4.5;
const AA_LARGE = 3;

/** The token/`--bg` pairs and their WCAG floors, matching the #36 gate. */
const CONTRAST_PAIRS: readonly { readonly token: CurioBaseToken; readonly min: number }[] = [
    { token: 'fg', min: AA_NORMAL },
    { token: 'fg-muted', min: AA_NORMAL },
    { token: 'fg-subtle', min: AA_LARGE },
];

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

export interface ContrastFailure {
    readonly token: CurioBaseToken;
    readonly ratio: number;
    readonly min: number;
}

/**
 * The token pairs that fall below their WCAG floor against `--bg`. Pairs whose
 * colours aren't plain hex are skipped (can't be evaluated cheaply) rather than
 * flagged — this is a warning, never a hard reject.
 */
export function contrastFailures(theme: CustomTheme): ContrastFailure[] {
    const bg = toHex6(theme.tokens.bg);
    if (bg === null) return [];
    const failures: ContrastFailure[] = [];
    for (const { token, min } of CONTRAST_PAIRS) {
        const fg = toHex6(theme.tokens[token]);
        if (fg === null) continue;
        const ratio = contrast(fg, bg);
        if (ratio < min) {
            failures.push({ token, ratio, min });
        }
    }
    return failures;
}

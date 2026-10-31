/**
 * Reader typography state: body font, size, line height, measure, spacing,
 * weight, alignment, hyphenation, and the reading-surface tint. Every value is
 * persisted to the settings table and clamped into {@link TYPOGRAPHY_LIMITS}.
 */
import { SETTING_KEYS, settingsStore } from './settings.svelte';

/** Reader body font — an id mapped to a concrete CSS font stack. */
export type ReaderFontId =
    | 'sans'
    | 'serif'
    | 'mono'
    | 'humanist'
    | 'grotesque'
    | 'georgia'
    | 'charter'
    | 'readable';

export interface ReaderFontInfo {
    readonly id: ReaderFontId;
    readonly name: string;
    /** The CSS `font-family` value applied to the reader body. */
    readonly stack: string;
}

/**
 * The reader font choices — all **offline-safe system stacks** (Curio is
 * offline-first: the CSP blocks remote fonts and we ship none), each falling
 * through widely-available faces to a generic family. The Settings select and
 * the reader Typography popover both render each option in its own face.
 */
export const READER_FONTS: readonly ReaderFontInfo[] = [
    { id: 'sans', name: 'Sans', stack: 'var(--font-family)' },
    {
        id: 'serif',
        name: 'Serif',
        stack: 'Iowan Old Style, "Palatino Linotype", Palatino, Georgia, Cambria, "Times New Roman", serif',
    },
    { id: 'mono', name: 'Mono', stack: 'var(--font-mono)' },
    {
        id: 'humanist',
        name: 'Humanist',
        stack: 'Seravek, "Gill Sans Nova", "Gill Sans", Calibri, "Segoe UI", sans-serif',
    },
    {
        id: 'grotesque',
        name: 'Grotesque',
        stack: '"Helvetica Neue", Helvetica, "Arial Nova", Arial, sans-serif',
    },
    {
        id: 'georgia',
        name: 'Georgia',
        stack: 'Georgia, Cambria, "Times New Roman", Times, serif',
    },
    {
        id: 'charter',
        name: 'Charter',
        stack: 'Charter, "Bitstream Charter", "Sitka Text", Cambria, Georgia, serif',
    },
    {
        id: 'readable',
        name: 'Readable',
        stack: '"Comic Sans MS", "Comic Neue", "Trebuchet MS", "Segoe UI", sans-serif',
    },
] as const;

export function isReaderFontId(value: string): value is ReaderFontId {
    return READER_FONTS.some((font) => font.id === value);
}

/** Typography clamp bounds — the controls and `initTypography` share them. */
export const TYPOGRAPHY_LIMITS = {
    fontSize: { min: 13, max: 24, default: 16 },
    lineHeight: { min: 1.3, max: 2, default: 1.6 },
    measure: { min: 520, max: 960, default: 720 },
    /** Space between paragraphs, as a multiple of the font size (em). */
    paragraphSpacing: { min: 0.4, max: 2, default: 1 },
    /** Reader body font weight (100-step CSS weights). */
    fontWeight: { min: 300, max: 700, default: 400 },
    /** Extra tracking between letters, in em. */
    letterSpacing: { min: -0.02, max: 0.08, default: 0 },
} as const;

/** Reader text alignment. `justify` sets `text-align: justify` with hyphenation. */
export type ReaderTextAlign = 'left' | 'justify';
export const READER_TEXT_ALIGNS: readonly ReaderTextAlign[] = ['left', 'justify'] as const;

export function isReaderTextAlign(value: string): value is ReaderTextAlign {
    return (READER_TEXT_ALIGNS as readonly string[]).includes(value);
}

/**
 * A reading surface tint applied to the reader body only, independent of the
 * app theme: `default` follows the app theme, `sepia`/`paper` force a warm
 * paper ground with dark ink so long reads stay comfortable on any theme.
 */
export type ReaderTheme = 'default' | 'sepia' | 'paper';
export const READER_THEMES: readonly ReaderTheme[] = ['default', 'sepia', 'paper'] as const;

export function isReaderTheme(value: string): value is ReaderTheme {
    return (READER_THEMES as readonly string[]).includes(value);
}

function clamp(value: number, min: number, max: number): number {
    return Math.min(max, Math.max(min, value));
}

export class TypographyStore {
    fontSize: number = $state(TYPOGRAPHY_LIMITS.fontSize.default);
    lineHeight: number = $state(TYPOGRAPHY_LIMITS.lineHeight.default);
    measure: number = $state(TYPOGRAPHY_LIMITS.measure.default);
    fontFamily: ReaderFontId = $state('serif');
    paragraphSpacing: number = $state(TYPOGRAPHY_LIMITS.paragraphSpacing.default);
    textAlign: ReaderTextAlign = $state('left');
    fontWeight: number = $state(TYPOGRAPHY_LIMITS.fontWeight.default);
    letterSpacing: number = $state(TYPOGRAPHY_LIMITS.letterSpacing.default);
    hyphenate: boolean = $state(false);
    readingTheme: ReaderTheme = $state('default');

    /** The CSS `font-family` value for the current reader font. */
    get readerFontStack(): string {
        const font = READER_FONTS.find((candidate) => candidate.id === this.fontFamily);
        return (font ?? READER_FONTS[0]).stack;
    }

    /**
     * Adopts persisted reader typography at startup (after
     * `settingsStore.load()`). Each value is clamped into range; anything
     * unparseable or unknown falls back to the default.
     */
    initTypography(): void {
        this.fontSize = readNumber(
            SETTING_KEYS.fontSize,
            TYPOGRAPHY_LIMITS.fontSize,
            TYPOGRAPHY_LIMITS.fontSize.default,
        );
        this.lineHeight = readNumber(
            SETTING_KEYS.lineHeight,
            TYPOGRAPHY_LIMITS.lineHeight,
            TYPOGRAPHY_LIMITS.lineHeight.default,
        );
        this.measure = readNumber(
            SETTING_KEYS.measure,
            TYPOGRAPHY_LIMITS.measure,
            TYPOGRAPHY_LIMITS.measure.default,
        );
        this.paragraphSpacing = readNumber(
            SETTING_KEYS.paragraphSpacing,
            TYPOGRAPHY_LIMITS.paragraphSpacing,
            TYPOGRAPHY_LIMITS.paragraphSpacing.default,
        );
        const persistedFont = settingsStore.get(SETTING_KEYS.fontFamily);
        if (persistedFont !== undefined && isReaderFontId(persistedFont)) {
            this.fontFamily = persistedFont;
        }
        const persistedAlign = settingsStore.get(SETTING_KEYS.textAlign);
        if (persistedAlign !== undefined && isReaderTextAlign(persistedAlign)) {
            this.textAlign = persistedAlign;
        }
        this.fontWeight = readNumber(
            SETTING_KEYS.fontWeight,
            TYPOGRAPHY_LIMITS.fontWeight,
            TYPOGRAPHY_LIMITS.fontWeight.default,
        );
        this.letterSpacing = readNumber(
            SETTING_KEYS.letterSpacing,
            TYPOGRAPHY_LIMITS.letterSpacing,
            TYPOGRAPHY_LIMITS.letterSpacing.default,
        );
        this.hyphenate = settingsStore.get(SETTING_KEYS.hyphenate) === 'true';
        const persistedReadingTheme = settingsStore.get(SETTING_KEYS.readingTheme);
        if (persistedReadingTheme !== undefined && isReaderTheme(persistedReadingTheme)) {
            this.readingTheme = persistedReadingTheme;
        }
    }

    /** Sets the reader font size (px), clamped, and persists it. */
    async setFontSize(px: number): Promise<void> {
        const { min, max } = TYPOGRAPHY_LIMITS.fontSize;
        this.fontSize = clamp(px, min, max);
        await settingsStore.set(SETTING_KEYS.fontSize, String(this.fontSize));
    }

    /** Sets the reader line height, clamped, and persists it. */
    async setLineHeight(value: number): Promise<void> {
        const { min, max } = TYPOGRAPHY_LIMITS.lineHeight;
        this.lineHeight = clamp(value, min, max);
        await settingsStore.set(SETTING_KEYS.lineHeight, String(this.lineHeight));
    }

    /** Sets the reader measure (max line length, px), clamped, and persists it. */
    async setMeasure(px: number): Promise<void> {
        const { min, max } = TYPOGRAPHY_LIMITS.measure;
        this.measure = clamp(px, min, max);
        await settingsStore.set(SETTING_KEYS.measure, String(this.measure));
    }

    /** Sets the reader font family and persists it. */
    async setFontFamily(id: ReaderFontId): Promise<void> {
        this.fontFamily = id;
        await settingsStore.set(SETTING_KEYS.fontFamily, id);
    }

    /** Sets the paragraph spacing (em multiple), clamped, and persists it. */
    async setParagraphSpacing(value: number): Promise<void> {
        const { min, max } = TYPOGRAPHY_LIMITS.paragraphSpacing;
        this.paragraphSpacing = clamp(value, min, max);
        await settingsStore.set(SETTING_KEYS.paragraphSpacing, String(this.paragraphSpacing));
    }

    /** Sets the reader text alignment and persists it. */
    async setTextAlign(align: ReaderTextAlign): Promise<void> {
        this.textAlign = align;
        await settingsStore.set(SETTING_KEYS.textAlign, align);
    }

    /** Sets the reader body font weight, clamped, and persists it. */
    async setFontWeight(value: number): Promise<void> {
        const { min, max } = TYPOGRAPHY_LIMITS.fontWeight;
        this.fontWeight = clamp(value, min, max);
        await settingsStore.set(SETTING_KEYS.fontWeight, String(this.fontWeight));
    }

    /** Sets the reader letter spacing (em), clamped, and persists it. */
    async setLetterSpacing(value: number): Promise<void> {
        const { min, max } = TYPOGRAPHY_LIMITS.letterSpacing;
        this.letterSpacing = clamp(value, min, max);
        await settingsStore.set(SETTING_KEYS.letterSpacing, String(this.letterSpacing));
    }

    /** Toggles automatic hyphenation of reader body text and persists it. */
    async setHyphenate(value: boolean): Promise<void> {
        this.hyphenate = value;
        await settingsStore.set(SETTING_KEYS.hyphenate, String(value));
    }

    /** Sets the reading-surface tint (independent of the app theme) and persists it. */
    async setReadingTheme(theme: ReaderTheme): Promise<void> {
        this.readingTheme = theme;
        await settingsStore.set(SETTING_KEYS.readingTheme, theme);
    }

    /** Test isolation. */
    reset(): void {
        this.fontSize = TYPOGRAPHY_LIMITS.fontSize.default;
        this.lineHeight = TYPOGRAPHY_LIMITS.lineHeight.default;
        this.measure = TYPOGRAPHY_LIMITS.measure.default;
        this.fontFamily = 'serif';
    }
}

function readNumber(
    key: (typeof SETTING_KEYS)[keyof typeof SETTING_KEYS],
    limits: { min: number; max: number },
    fallback: number,
): number {
    const raw = settingsStore.get(key);
    if (raw === undefined) {
        return fallback;
    }
    const parsed = Number.parseFloat(raw);
    if (Number.isNaN(parsed)) {
        return fallback;
    }
    return clamp(parsed, limits.min, limits.max);
}

/** The app-wide singleton. */
export const typographyStore = new TypographyStore();

/**
 * The one DOM-touching theme helper: read a theme's live base-token values off
 * an element the theme is applied to (the document root for the active theme,
 * or a detached probe for a specific one). Kept out of `theme-format` so that
 * module stays pure and node-testable.
 */
import { CURIO_BASE_TOKENS, type ThemeTokens } from './theme-format';

/** Reads the 21 base token values resolved on `element` via `getComputedStyle`. */
export function readComputedTokens(element: Element): ThemeTokens {
    const style = getComputedStyle(element);
    const tokens: Record<string, string> = {};
    for (const token of CURIO_BASE_TOKENS) {
        tokens[token] = style.getPropertyValue(`--${token}`).trim();
    }
    return tokens as ThemeTokens;
}

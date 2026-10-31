/**
 * Test isolation for the UI chrome stores (theme, typography, layout, toasts,
 * prefs): calls every singleton's `reset()` in one go, for component suites
 * that touch several of them.
 *
 * Each `reset()` restores only the fields it lists (carried over unchanged
 * from the old `UiStore.reset()`): typography leaves paragraph spacing,
 * alignment, weight, letter spacing, hyphenation and reading theme alone, and
 * layout leaves the pane widths and home-section toggles alone. A suite that
 * mutates one of those must restore it itself.
 */
import { layoutStore } from '$lib/state/layout.svelte';
import { prefsStore } from '$lib/state/prefs.svelte';
import { themeStore } from '$lib/state/theme.svelte';
import { toastStore } from '$lib/state/toasts.svelte';
import { typographyStore } from '$lib/state/typography.svelte';

export function resetUiStores(): void {
    toastStore.reset();
    layoutStore.reset();
    prefsStore.reset();
    themeStore.reset();
    typographyStore.reset();
}

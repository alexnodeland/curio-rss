/**
 * Test isolation for the UI chrome stores (theme, typography, layout, toasts,
 * prefs): resets every singleton in one call, for component suites that touch
 * several of them.
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

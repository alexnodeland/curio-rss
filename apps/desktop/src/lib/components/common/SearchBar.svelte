<script lang="ts">
/**
 * The FTS search box. Typing drives `searchStore.setQuery` (debounced →
 * `search_articles`); the middle pane swaps to results while the query is
 * non-empty. `/` focuses it (via the store's focus nonce), Escape clears and
 * blurs. Being an input, it is a typing context — global shortcuts stay
 * inert while it holds focus (see `shouldIgnoreKeyEvent`).
 */
import Icon from '$components/common/Icon.svelte';
import { t } from '$lib/i18n';
import { searchStore } from '$lib/state/search.svelte';

let input: HTMLInputElement | undefined = $state();
let lastFocusNonce = 0;

// Focus on the `/` shortcut: the action layer bumps the nonce, we react.
$effect(() => {
    const nonce = searchStore.focusNonce;
    if (nonce !== lastFocusNonce) {
        lastFocusNonce = nonce;
        input?.focus();
        input?.select();
    }
});

function onInput(event: Event & { currentTarget: HTMLInputElement }): void {
    searchStore.setQuery(event.currentTarget.value);
}

function onKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
        event.preventDefault();
        searchStore.clear();
        input?.blur();
    }
}
</script>

<div class="search">
    <span class="search-icon" aria-hidden="true"><Icon name="search" size={16} /></span>
    <input
        bind:this={input}
        class="search-input"
        type="search"
        value={searchStore.query}
        placeholder={t('search.placeholder')}
        aria-label={t('search.label')}
        oninput={onInput}
        onkeydown={onKeydown}
    />
    {#if searchStore.active}
        <button
            class="search-clear"
            type="button"
            aria-label={t('search.clear')}
            onclick={() => searchStore.clear()}><Icon name="close" size={14} /></button
        >
    {/if}
</div>

<style>
    .search {
        display: flex;
        align-items: center;
        gap: var(--space-2);
        height: 34px;
        padding: 0 var(--space-3);
        border-radius: var(--radius-md);
        background: var(--surface-inset);
        border: 1px solid var(--hairline);
        transition:
            border-color var(--dur-fast) var(--ease),
            box-shadow var(--dur-fast) var(--ease);
    }

    .search:focus-within {
        border-color: color-mix(in srgb, var(--accent), transparent 40%);
        box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent), transparent 82%);
    }

    .search-icon {
        flex: 0 0 auto;
        display: flex;
        color: var(--fg-subtle);
    }

    .search-input {
        flex: 1 1 auto;
        min-width: 0;
        padding: var(--space-1) 0;
        background: transparent;
        color: var(--fg);
        border: none;
        font-size: var(--text-md);
    }

    .search-input:focus-visible {
        outline: none;
    }

    /* Hide the native search decorations — we ship our own clear button. */
    .search-input::-webkit-search-cancel-button {
        appearance: none;
    }

    .search-clear {
        flex: 0 0 auto;
        display: flex;
        align-items: center;
        justify-content: center;
        width: 1.125rem;
        height: 1.125rem;
        border-radius: 50%;
        background: transparent;
        color: var(--fg-subtle);
        font-size: 1rem;
        line-height: 1;
    }

    .search-clear:hover {
        color: var(--fg);
        background: var(--bg-hover);
    }
</style>

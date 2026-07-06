<script lang="ts">
/**
 * A reusable modal shell: backdrop, a focus-trapped dialog with a labelled
 * header and close button, and a scrollable body snippet. Dismissal by
 * Escape is owned by the shell keydown handler (it closes `activeModal`);
 * the close button covers the pointer path. Only one modal is ever open, so
 * a fixed title id is safe.
 */
import { t } from '$lib/i18n';
import type { Snippet } from 'svelte';

let {
    title,
    onclose,
    size = 'medium',
    children,
}: {
    title: string;
    onclose: () => void;
    size?: 'medium' | 'large';
    children: Snippet;
} = $props();

let dialog: HTMLElement | undefined = $state();

$effect(() => {
    dialog?.focus();
});
</script>

<div class="overlay-backdrop">
    <div
        class="overlay"
        class:large={size === 'large'}
        role="dialog"
        aria-modal="true"
        aria-labelledby="cx-modal-title"
        tabindex="-1"
        bind:this={dialog}
    >
        <header class="overlay-header">
            <h2 id="cx-modal-title">{title}</h2>
            <button class="overlay-close" type="button" onclick={onclose} aria-label={t('modal.close')}
                >×</button
            >
        </header>
        <div class="overlay-body">
            {@render children()}
        </div>
    </div>
</div>

<style>
    .overlay-backdrop {
        position: fixed;
        inset: 0;
        display: flex;
        align-items: center;
        justify-content: center;
        background: rgb(0 0 0 / 45%);
        z-index: 100;
    }

    .overlay {
        width: min(560px, calc(100vw - var(--space-8)));
        max-height: min(80vh, 720px);
        display: flex;
        flex-direction: column;
        background: var(--bg-secondary);
        border: 1px solid var(--border);
        border-radius: var(--radius-lg);
        box-shadow: var(--shadow-lg);
    }

    .overlay.large {
        width: min(720px, calc(100vw - var(--space-8)));
    }

    .overlay:focus-visible {
        outline: 2px solid var(--accent);
        outline-offset: -2px;
    }

    .overlay-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: var(--space-4) var(--space-5);
        border-bottom: 1px solid var(--border-subtle);
    }

    .overlay-header h2 {
        font-size: 1rem;
    }

    .overlay-close {
        padding: 0 var(--space-2);
        font-size: 1.25rem;
        line-height: 1;
        color: var(--fg-muted);
        background: transparent;
        border-radius: var(--radius-sm);
    }

    .overlay-close:hover {
        color: var(--fg);
        background: var(--bg-hover);
    }

    .overlay-body {
        overflow-y: auto;
        padding: var(--space-4) var(--space-5) var(--space-5);
        display: flex;
        flex-direction: column;
        gap: var(--space-5);
    }
</style>

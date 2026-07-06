<script lang="ts">
/**
 * A reusable modal shell: backdrop, a focus-trapped dialog with a labelled
 * header and close button, and a scrollable body snippet. Dismissal by
 * Escape is owned by the shell keydown handler (it closes `activeModal`);
 * the close button covers the pointer path. Only one modal is ever open, so
 * a fixed title id is safe.
 */
import Icon from '$components/common/Icon.svelte';
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
                ><Icon name="close" size={16} /></button
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
        padding: var(--space-6);
        background: color-mix(in srgb, #000, transparent 45%);
        backdrop-filter: blur(3px);
        z-index: 100;
        animation: backdrop-in var(--dur-base) var(--ease);
    }

    .overlay {
        width: min(560px, calc(100vw - var(--space-8)));
        max-height: min(82vh, 760px);
        display: flex;
        flex-direction: column;
        background: var(--surface-overlay);
        border: 1px solid var(--hairline);
        border-radius: var(--radius-xl);
        box-shadow: var(--shadow-lg);
        animation: overlay-in var(--dur-base) var(--ease);
    }

    .overlay.large {
        width: min(760px, calc(100vw - var(--space-8)));
    }

    .overlay:focus-visible {
        outline: none;
    }

    @keyframes backdrop-in {
        from {
            opacity: 0;
        }
    }

    @keyframes overlay-in {
        from {
            opacity: 0;
            transform: translateY(8px) scale(0.99);
        }
    }

    .overlay-header {
        flex: 0 0 auto;
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: var(--space-4) var(--space-4) var(--space-4) var(--space-6);
        border-bottom: 1px solid var(--hairline);
    }

    .overlay-header h2 {
        font-size: var(--text-lg);
        font-weight: 640;
        letter-spacing: var(--tracking-snug);
    }

    .overlay-close {
        display: inline-grid;
        place-items: center;
        width: 32px;
        height: 32px;
        color: var(--fg-muted);
        background: transparent;
        border-radius: var(--radius-md);
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .overlay-close:hover {
        color: var(--fg);
        background: var(--hover);
    }

    .overlay-body {
        overflow-y: auto;
        padding: var(--space-6);
        display: flex;
        flex-direction: column;
        gap: var(--space-6);
    }
</style>

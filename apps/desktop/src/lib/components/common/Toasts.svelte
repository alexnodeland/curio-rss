<script lang="ts">
/**
 * The toast outlet — renders `uiStore.toasts` (queued by the action layer
 * on command failures, and by anything else that needs a transient notice).
 * Announced politely to screen readers; every toast is dismissible.
 */
import { t } from '$lib/i18n';
import { uiStore } from '$lib/state/ui.svelte';
</script>

<div class="toasts" aria-live="polite">
    {#each uiStore.toasts as toast (toast.id)}
        <div class="toast toast-{toast.tone}" role="status">
            <span class="toast-message">{toast.message}</span>
            <button
                class="toast-dismiss"
                onclick={() => uiStore.dismissToast(toast.id)}
                aria-label={t('toast.dismiss')}
            >
                ×
            </button>
        </div>
    {/each}
</div>

<style>
    .toasts {
        position: fixed;
        right: var(--space-4);
        bottom: var(--space-4);
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
        z-index: 200;
        max-width: min(360px, calc(100vw - var(--space-8)));
    }

    .toast {
        display: flex;
        align-items: center;
        gap: var(--space-3);
        padding: var(--space-3) var(--space-4);
        border-radius: var(--radius-md);
        border: 1px solid var(--border);
        background: var(--bg-secondary);
        color: var(--fg);
        box-shadow: var(--shadow-md);
        font-size: 0.875rem;
    }

    .toast-error {
        border-color: var(--error);
        background: var(--error-bg);
    }

    .toast-success {
        border-color: var(--success);
    }

    .toast-warning {
        border-color: var(--warning);
    }

    .toast-message {
        flex: 1 1 auto;
        min-width: 0;
        overflow-wrap: break-word;
    }

    .toast-dismiss {
        flex: 0 0 auto;
        padding: 0 var(--space-1);
        font-size: 1rem;
        line-height: 1;
        color: var(--fg-muted);
        background: transparent;
        border-radius: var(--radius-sm);
    }

    .toast-dismiss:hover {
        color: var(--fg);
        background: var(--bg-hover);
    }
</style>

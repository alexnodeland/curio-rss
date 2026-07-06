<script lang="ts">
/**
 * The toast outlet — renders `uiStore.toasts` (queued by the action layer
 * on command failures, and by anything else that needs a transient notice).
 * Announced politely to screen readers; every toast is dismissible.
 */
import Icon from '$components/common/Icon.svelte';
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
                <Icon name="close" size={16} />
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
        position: relative;
        display: flex;
        align-items: center;
        gap: var(--space-3);
        padding: var(--space-3) var(--space-4);
        padding-left: var(--space-5);
        border-radius: var(--radius-lg);
        border: 1px solid var(--hairline);
        background: var(--surface-overlay);
        color: var(--fg);
        box-shadow: var(--shadow-lg);
        font-size: var(--text-md);
        overflow: hidden;
        animation: toast-in var(--dur-base) var(--ease);
    }

    /* Tone accent as a left bar, not a full border tint. */
    .toast::before {
        content: '';
        position: absolute;
        left: 0;
        top: 0;
        bottom: 0;
        width: 3px;
        background: var(--accent);
    }

    .toast-error::before {
        background: var(--error);
    }
    .toast-success::before {
        background: var(--success);
    }
    .toast-warning::before {
        background: var(--warning);
    }

    @keyframes toast-in {
        from {
            opacity: 0;
            transform: translateY(6px);
        }
    }

    .toast-message {
        flex: 1 1 auto;
        min-width: 0;
        overflow-wrap: break-word;
    }

    .toast-dismiss {
        flex: 0 0 auto;
        display: grid;
        place-items: center;
        width: 1.35rem;
        height: 1.35rem;
        font-size: 1rem;
        line-height: 1;
        color: var(--fg-muted);
        background: transparent;
        border-radius: var(--radius-sm);
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .toast-dismiss:hover {
        color: var(--fg);
        background: var(--hover);
    }
</style>

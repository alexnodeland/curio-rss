<script lang="ts">
/**
 * The centered failure state for a pane whose first load failed (no rows or
 * article to keep showing): the localized failure and a Retry that re-runs
 * the load. Disabled while that retry is in flight, so a double click never
 * queues a second round-trip.
 */
import Icon from '$components/common/Icon.svelte';
import { t } from '$lib/i18n';

let {
    message,
    onretry,
    busy = false,
}: {
    message: string;
    onretry: () => void;
    /** The retried load is in flight. */
    busy?: boolean;
} = $props();
</script>

<div class="error-state">
    <div class="error-mark" aria-hidden="true">
        <Icon name="alert" size={22} strokeWidth={1.75} />
    </div>
    <p class="error-message" role="alert">{message}</p>
    <button class="state-action" type="button" disabled={busy} onclick={onretry}>
        <Icon name="refresh" size={14} />
        {t('toast.retry')}
    </button>
</div>

<style>
    .error-state {
        flex: 1 1 auto;
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        gap: var(--space-3);
        padding: var(--space-8) var(--space-6);
        text-align: center;
    }

    .error-mark {
        display: grid;
        place-items: center;
        width: 48px;
        height: 48px;
        border-radius: var(--radius-xl);
        color: var(--error-text);
        background: var(--error-bg);
    }

    .error-message {
        max-width: 32em;
        font-size: var(--text-md);
        line-height: 1.5;
        color: var(--fg);
    }
</style>

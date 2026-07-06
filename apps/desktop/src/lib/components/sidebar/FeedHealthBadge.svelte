<script lang="ts">
/**
 * A tiny lifecycle badge on a feed row: paused feeds and 410-dead feeds
 * (auto-paused by core) get a visible pill; active feeds show nothing.
 */
import type { FeedStatusDto } from '$lib/bindings';
import { type MessageKey, t } from '$lib/i18n';

let { status }: { status: FeedStatusDto } = $props();

const LABEL: Record<'paused' | 'dead', MessageKey> = {
    paused: 'feedHealth.badge.paused',
    dead: 'feedHealth.badge.dead',
};
</script>

{#if status === 'paused' || status === 'dead'}
    <span class="health-badge health-{status}">{t(LABEL[status])}</span>
{/if}

<style>
    .health-badge {
        flex: 0 0 auto;
        padding: 0 var(--space-1);
        border-radius: var(--radius-xl);
        font-size: 0.625rem;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }

    .health-paused {
        background: var(--bg-tertiary);
        color: var(--fg-muted);
    }

    .health-dead {
        background: var(--error);
        color: var(--accent-fg);
    }
</style>

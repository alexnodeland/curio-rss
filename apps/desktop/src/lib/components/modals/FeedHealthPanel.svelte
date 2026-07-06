<script lang="ts">
/**
 * The feed-health panel: a feed's lifecycle status with pause / resume /
 * reactivate controls (`set_feed_status`) and its recent fetch log
 * (`recent_fetches` — the feed-health surface). Status changes emit
 * `FeedsChanged` Rust-side, so the sidebar refreshes without client
 * bookkeeping.
 */
import Modal from '$components/common/Modal.svelte';
import {
    type FeedStatusDto,
    type FetchRecordDto,
    type FetchStatusDto,
    commands,
} from '$lib/bindings';
import { type MessageKey, formatIntlDateTime, t } from '$lib/i18n';
import { toastCommandError } from '$lib/state/actions';
import { feedsStore } from '$lib/state/feeds.svelte';

let { feedId, onclose }: { feedId: number; onclose: () => void } = $props();

const FETCH_LABEL: Record<FetchStatusDto, MessageKey> = {
    ok: 'feedHealth.fetch.ok',
    not_modified: 'feedHealth.fetch.not_modified',
    error: 'feedHealth.fetch.error',
};

const STATUS_LABEL: Record<FeedStatusDto, MessageKey> = {
    active: 'feedHealth.badge.active',
    paused: 'feedHealth.badge.paused',
    dead: 'feedHealth.badge.dead',
};

let fetches: FetchRecordDto[] = $state([]);
let loadingFetches = $state(true);

feedsStore.prime();

const feed = $derived(feedsStore.feeds.find((candidate) => candidate.id === feedId) ?? null);

$effect(() => {
    const id = feedId;
    loadingFetches = true;
    let cancelled = false;
    void commands.recentFetches(id, 20).then((result) => {
        if (cancelled) {
            return;
        }
        if (result.status === 'ok') {
            fetches = result.data;
        } else {
            toastCommandError(result.error);
        }
        loadingFetches = false;
    });
    return () => {
        cancelled = true;
    };
});

async function setStatus(status: FeedStatusDto): Promise<void> {
    const result = await feedsStore.setFeedStatus(feedId, status);
    if (result.status === 'error') {
        toastCommandError(result.error);
    }
}

function whenDate(iso: string): string {
    const date = new Date(iso);
    return Number.isNaN(date.getTime()) ? iso : formatIntlDateTime(date);
}
</script>

<Modal title={t('feedHealth.title')} {onclose}>
    {#if feed === null}
        <p class="status">{t('reader.missing')}</p>
    {:else}
        <div class="health-head">
            <span class="feed-name truncate">{feed.title ?? feed.url}</span>
            <span class="status-pill status-{feed.status}">{t(STATUS_LABEL[feed.status])}</span>
        </div>

        <div class="controls" role="group" aria-label={t('feedHealth.status')}>
            {#if feed.status === 'active'}
                <button type="button" onclick={() => void setStatus('paused')}>{t('feedHealth.pause')}</button>
            {:else if feed.status === 'paused'}
                <button type="button" onclick={() => void setStatus('active')}>{t('feedHealth.resume')}</button>
            {:else}
                <button type="button" onclick={() => void setStatus('active')}>{t('feedHealth.revive')}</button>
            {/if}
        </div>

        <section class="recent" aria-label={t('feedHealth.recent')}>
            <h3>{t('feedHealth.recent')}</h3>
            {#if loadingFetches}
                <p class="status">{t('app.loading')}</p>
            {:else if fetches.length === 0}
                <p class="status">{t('feedHealth.none')}</p>
            {:else}
                <ul class="fetch-list">
                    {#each fetches as record, index (index)}
                        <li class="fetch fetch-{record.status}">
                            <span class="fetch-when">{whenDate(record.fetched_at)}</span>
                            <span class="fetch-status">{t(FETCH_LABEL[record.status])}</span>
                            {#if record.http_status !== null}
                                <span class="fetch-http">HTTP {record.http_status}</span>
                            {/if}
                            {#if record.articles_new > 0}
                                <span class="fetch-new">{t('feedHealth.new', { count: record.articles_new })}</span>
                            {/if}
                            {#if record.error !== null}
                                <span class="fetch-error truncate" title={record.error}>{record.error}</span>
                            {/if}
                        </li>
                    {/each}
                </ul>
            {/if}
        </section>
    {/if}
</Modal>

<style>
    .health-head {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: var(--space-3);
    }

    .feed-name {
        font-size: 0.9375rem;
        font-weight: 600;
        min-width: 0;
    }

    .status-pill {
        flex: 0 0 auto;
        padding: 2px var(--space-2);
        border-radius: var(--radius-xl);
        font-size: 0.6875rem;
        font-weight: 600;
        background: var(--bg-tertiary);
        color: var(--fg-muted);
    }

    .status-paused {
        background: var(--warning, #b8860b);
        color: var(--accent-fg);
    }

    .status-dead {
        background: var(--error);
        color: var(--accent-fg);
    }

    .controls {
        display: flex;
        gap: var(--space-2);
    }

    .controls button {
        padding: var(--space-2) var(--space-3);
        border-radius: var(--radius-md);
        background: var(--bg-tertiary);
        color: var(--fg);
        border: 1px solid var(--border);
        font-size: 0.8125rem;
    }

    .controls button:hover {
        background: var(--bg-hover);
    }

    .recent h3 {
        font-size: 0.6875rem;
        font-weight: 600;
        letter-spacing: 0.08em;
        text-transform: uppercase;
        color: var(--fg-subtle);
        margin-bottom: var(--space-2);
    }

    .status {
        font-size: 0.8125rem;
        color: var(--fg-muted);
    }

    .fetch-list {
        list-style: none;
        display: flex;
        flex-direction: column;
        gap: var(--space-1);
    }

    .fetch {
        display: flex;
        align-items: baseline;
        gap: var(--space-2);
        padding: var(--space-1) var(--space-2);
        border-radius: var(--radius-sm);
        font-size: 0.75rem;
    }

    .fetch-error {
        color: var(--error);
        min-width: 0;
    }

    .fetch-when {
        color: var(--fg-muted);
        font-variant-numeric: tabular-nums;
    }

    .fetch-new {
        color: var(--accent);
    }
</style>

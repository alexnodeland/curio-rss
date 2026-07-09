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
import { markAllRead, selectView, toastCommandError } from '$lib/state/actions';
import { feedsStore } from '$lib/state/feeds.svelte';
import { selectionStore } from '$lib/state/selection.svelte';

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
let confirmingRemove = $state(false);

feedsStore.prime();

const feed = $derived(feedsStore.feeds.find((candidate) => candidate.id === feedId) ?? null);

// Turn a silent 403/429 in the latest fetch into a plain-language hint, so a
// blocked or rate-limited feed reads as an explanation, not a bare log row.
const fetchHint = $derived.by((): string | null => {
    const latest = fetches[0];
    if (latest === undefined || latest.http_status === null) {
        return null;
    }
    if (latest.http_status === 403) {
        return t('feedHealth.hint.forbidden');
    }
    if (latest.http_status === 429) {
        return t('feedHealth.hint.rateLimited');
    }
    return null;
});

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

/**
 * Unsubscribes and closes the panel. Stored articles survive (core sets
 * their `feed_id` to null); the sidebar + list refresh off the Rust-emitted
 * `FeedsChanged`/`ArticlesChanged`. If we were scoped to this feed, fall back
 * to All so the list is never stuck on a subscription that no longer exists.
 */
async function unsubscribe(): Promise<void> {
    const result = await feedsStore.removeFeed(feedId);
    if (result.status === 'error') {
        toastCommandError(result.error);
        return;
    }
    if (selectionStore.selectedFeedId === feedId) {
        selectView('all');
    }
    onclose();
}

// The feed's tags as a comma-separated string. `tagsDraft` holds the user's
// in-progress edit (null = untouched, so the field tracks the live tags);
// saving clears it back to null to re-sync to the freshly-stored tags.
let tagsDraft: string | null = $state(null);
const currentTags = $derived(feed?.tags.join(', ') ?? '');
const tagsValue = $derived(tagsDraft ?? currentTags);

function parseTags(value: string): string[] {
    return value
        .split(',')
        .map((tag) => tag.trim())
        .filter((tag) => tag.length > 0);
}

async function saveTags(): Promise<void> {
    const result = await feedsStore.setFeedTags(feedId, parseTags(tagsValue));
    if (result.status === 'error') {
        toastCommandError(result.error);
        return;
    }
    tagsDraft = null;
}

// The feed's display name (same draft/live pattern as tags). An empty name
// clears the title so the feed shows its URL again.
let titleDraft: string | null = $state(null);
const currentTitle = $derived(feed?.title ?? '');
const titleValue = $derived(titleDraft ?? currentTitle);

async function saveTitle(): Promise<void> {
    const trimmed = titleValue.trim();
    const result = await feedsStore.setFeedTitle(feedId, trimmed === '' ? null : trimmed);
    if (result.status === 'error') {
        toastCommandError(result.error);
        return;
    }
    titleDraft = null;
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
            <button type="button" onclick={() => void markAllRead(feedId)}
                >{t('feedMarkRead.action')}</button
            >
        </div>

        <section class="tags-edit">
            <label class="tags-label" for="feed-name-input">{t('feedName.label')}</label>
            <div class="tags-row">
                <input
                    id="feed-name-input"
                    class="tags-input"
                    type="text"
                    value={titleValue}
                    placeholder={feed.url}
                    oninput={(event) => (titleDraft = event.currentTarget.value)}
                />
                <button
                    type="button"
                    class="tags-save"
                    disabled={titleValue === currentTitle}
                    onclick={() => void saveTitle()}>{t('feedName.save')}</button
                >
            </div>
        </section>

        <section class="tags-edit">
            <label class="tags-label" for="feed-tags-input">{t('feedTags.label')}</label>
            <div class="tags-row">
                <input
                    id="feed-tags-input"
                    class="tags-input"
                    type="text"
                    value={tagsValue}
                    placeholder={t('feedTags.placeholder')}
                    oninput={(event) => (tagsDraft = event.currentTarget.value)}
                />
                <button
                    type="button"
                    class="tags-save"
                    disabled={tagsValue === currentTags}
                    onclick={() => void saveTags()}>{t('feedTags.save')}</button
                >
            </div>
            <p class="tags-hint">{t('feedTags.hint')}</p>
        </section>

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
            {#if fetchHint !== null}
                <p class="fetch-hint" role="note">{fetchHint}</p>
            {/if}
        </section>

        <section class="danger-zone" aria-label={t('feedHealth.unsubscribe')}>
            {#if confirmingRemove}
                <p class="danger-prompt">{t('feedHealth.unsubscribe.confirm')}</p>
                <div class="danger-actions">
                    <button type="button" class="danger" onclick={() => void unsubscribe()}
                        >{t('feedHealth.unsubscribe.remove')}</button
                    >
                    <button type="button" onclick={() => (confirmingRemove = false)}
                        >{t('feedHealth.unsubscribe.cancel')}</button
                    >
                </div>
            {:else}
                <button type="button" class="danger-trigger" onclick={() => (confirmingRemove = true)}
                    >{t('feedHealth.unsubscribe')}</button
                >
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
        font-size: var(--text-base);
        font-weight: 600;
        letter-spacing: var(--tracking-snug);
        min-width: 0;
        color: var(--fg);
    }

    .status-pill {
        flex: 0 0 auto;
        display: inline-flex;
        align-items: center;
        gap: var(--space-2);
        padding: 3px var(--space-3);
        border-radius: var(--radius-pill);
        font-size: var(--text-xs);
        font-weight: 600;
        background: var(--surface-raised);
        border: 1px solid var(--hairline);
        color: var(--fg-muted);
    }

    .status-pill::before {
        content: '';
        width: 7px;
        height: 7px;
        border-radius: 50%;
        background: var(--fg-subtle);
    }

    .status-active::before {
        background: var(--success);
    }

    .status-paused::before {
        background: var(--warning);
    }

    .status-dead::before {
        background: var(--error);
    }

    .controls {
        display: flex;
        gap: var(--space-2);
    }

    .controls button {
        padding: var(--space-2) var(--space-4);
        border-radius: var(--radius-md);
        background: transparent;
        color: var(--fg-muted);
        border: 1px solid var(--hairline-strong);
        font-size: var(--text-md);
        font-weight: 500;
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .controls button:hover {
        background: var(--hover);
        color: var(--fg);
    }

    .recent h3 {
        font-size: 0.6875rem;
        font-weight: 650;
        letter-spacing: var(--tracking-caps);
        text-transform: uppercase;
        color: var(--fg-subtle);
        margin-bottom: var(--space-2);
    }

    .status {
        font-size: var(--text-md);
        color: var(--fg-muted);
    }

    .fetch-list {
        list-style: none;
        display: flex;
        flex-direction: column;
        gap: 1px;
    }

    .fetch {
        display: flex;
        align-items: baseline;
        gap: var(--space-3);
        padding: var(--space-2) var(--space-3);
        border-radius: var(--radius-md);
        font-size: var(--text-xs);
        transition: background var(--dur-fast) var(--ease);
    }

    .fetch:hover {
        background: var(--hover);
    }

    .fetch-error {
        color: var(--error);
        min-width: 0;
    }

    .fetch-when {
        color: var(--fg-muted);
        font-family: var(--font-mono);
        font-variant-numeric: tabular-nums;
    }

    .fetch-http {
        color: var(--fg-subtle);
        font-family: var(--font-mono);
    }

    .fetch-new {
        color: var(--accent);
        font-weight: 500;
    }

    .fetch-hint {
        margin-top: var(--space-2);
        padding: var(--space-2) var(--space-3);
        border-radius: var(--radius-md);
        background: var(--surface-inset);
        border: 1px solid var(--hairline);
        color: var(--fg-muted);
        font-size: var(--text-sm);
        line-height: 1.4;
    }

    .danger-zone {
        margin-top: var(--space-4);
        padding-top: var(--space-3);
        border-top: 1px solid var(--hairline);
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
    }

    .danger-trigger,
    .danger-actions button {
        align-self: flex-start;
        padding: var(--space-2) var(--space-4);
        border-radius: var(--radius-md);
        background: transparent;
        border: 1px solid var(--hairline-strong);
        font-size: var(--text-md);
        font-weight: 500;
        color: var(--fg-muted);
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease),
            border-color var(--dur-fast) var(--ease);
    }

    .danger-prompt {
        font-size: var(--text-md);
        color: var(--fg-muted);
    }

    .danger-actions {
        display: flex;
        gap: var(--space-2);
    }

    .danger {
        color: var(--error);
        border-color: color-mix(in srgb, var(--error), transparent 40%);
    }

    .danger-trigger:hover,
    .danger:hover {
        background: color-mix(in srgb, var(--error), transparent 88%);
        border-color: var(--error);
        color: var(--error);
    }

    .tags-edit {
        display: flex;
        flex-direction: column;
        gap: var(--space-1);
    }

    .tags-label {
        font-size: 0.6875rem;
        font-weight: 650;
        letter-spacing: var(--tracking-caps);
        text-transform: uppercase;
        color: var(--fg-subtle);
    }

    .tags-row {
        display: flex;
        gap: var(--space-2);
    }

    .tags-input {
        flex: 1 1 auto;
        min-width: 0;
        padding: var(--space-2) var(--space-3);
        border-radius: var(--radius-md);
        background: var(--surface-raised);
        border: 1px solid var(--hairline-strong);
        color: var(--fg);
        font-size: var(--text-md);
    }

    .tags-save {
        flex: 0 0 auto;
        padding: var(--space-2) var(--space-4);
        border-radius: var(--radius-md);
        background: transparent;
        border: 1px solid var(--hairline-strong);
        color: var(--fg-muted);
        font-size: var(--text-md);
        font-weight: 500;
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .tags-save:hover:not(:disabled) {
        background: var(--hover);
        color: var(--fg);
    }

    .tags-save:disabled {
        opacity: 0.5;
    }

    .tags-hint {
        font-size: var(--text-xs);
        color: var(--fg-subtle);
    }
</style>

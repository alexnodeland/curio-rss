<script lang="ts">
/**
 * The edit-feed modal: one place to see and change everything about a
 * subscription. Details — the (read-only, copyable) feed URL, display name
 * (`set_feed_title`), site URL + description (`set_feed_metadata`), and
 * folders & tags (`set_feed_tags`, chip editor). Health — lifecycle status
 * with pause / resume / reactivate (`set_feed_status`), the recent fetch log
 * (`recent_fetches`) with a plain-language hint for a refused/rate-limited
 * feed, and mark-all-read. Danger — the two-step unsubscribe. Replaces the
 * old standalone feed-health panel (its health surface is now a section
 * here). Status/edit changes emit `FeedsChanged` head-side, so the sidebar
 * refreshes without client bookkeeping.
 */
import Modal from '$components/common/Modal.svelte';
import TagEditor from '$components/common/TagEditor.svelte';
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
import type { EditFeedSection } from '$lib/state/ui.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import { copyText } from '$lib/utils/clipboard';
import { openExternal } from '$lib/utils/external';

let {
    feedId,
    section = 'details',
    onclose,
}: { feedId: number; section?: EditFeedSection; onclose: () => void } = $props();

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
let healthEl = $state<HTMLElement>();

feedsStore.prime();

const feed = $derived(feedsStore.feeds.find((candidate) => candidate.id === feedId) ?? null);

// Open scrolled to Health when that's what the caller asked for (the sidebar
// health dot), once the section has rendered.
$effect(() => {
    if (section === 'health' && healthEl !== undefined) {
        healthEl.scrollIntoView({ block: 'start' });
    }
});

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

async function copyFeedUrl(): Promise<void> {
    if (feed === null) {
        return;
    }
    const ok = await copyText(feed.url);
    uiStore.showToast(
        t(ok ? 'editFeed.url.copied' : 'app.error.internal'),
        ok ? 'success' : 'error',
    );
}

/**
 * Unsubscribes and closes the modal. Stored articles survive (core sets
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

// Each editable field holds an in-progress draft (null = untouched, so the
// field tracks the live value); saving clears the draft back to null to
// re-sync to the freshly-stored value.
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

let siteDraft: string | null = $state(null);
let descDraft: string | null = $state(null);
const currentSite = $derived(feed?.site_url ?? '');
const currentDesc = $derived(feed?.description ?? '');
const siteValue = $derived(siteDraft ?? currentSite);
const descValue = $derived(descDraft ?? currentDesc);
const detailsDirty = $derived(siteValue !== currentSite || descValue !== currentDesc);

async function saveDetails(): Promise<void> {
    const site = siteValue.trim();
    const desc = descValue.trim();
    const result = await feedsStore.setFeedMetadata(
        feedId,
        site === '' ? null : site,
        desc === '' ? null : desc,
    );
    if (result.status === 'error') {
        toastCommandError(result.error);
        return;
    }
    siteDraft = null;
    descDraft = null;
}

async function saveTags(next: string[]): Promise<void> {
    const result = await feedsStore.setFeedTags(feedId, next);
    if (result.status === 'error') {
        toastCommandError(result.error);
    }
}

function whenDate(iso: string): string {
    const date = new Date(iso);
    return Number.isNaN(date.getTime()) ? iso : formatIntlDateTime(date);
}
</script>

<Modal title={t('editFeed.title')} {onclose} size="large">
    {#if feed === null}
        <p class="status">{t('reader.missing')}</p>
    {:else}
        <div class="head">
            <span class="feed-name truncate">{feed.title ?? feed.url}</span>
            <span class="status-pill status-{feed.status}">{t(STATUS_LABEL[feed.status])}</span>
        </div>

        <section class="field" aria-label={t('editFeed.section.details')}>
            <div class="field-block">
                <span class="field-label" id="edit-url-label">{t('editFeed.url.label')}</span>
                <div class="field-row">
                    <output class="url-value truncate" aria-labelledby="edit-url-label"
                        >{feed.url}</output
                    >
                    <button type="button" class="ghost" onclick={() => void copyFeedUrl()}
                        >{t('editFeed.url.copy')}</button
                    >
                    <button
                        type="button"
                        class="ghost"
                        onclick={() => void openExternal(feed.url)}>{t('editFeed.url.open')} ↗</button
                    >
                </div>
            </div>

            <div class="field-block">
                <label class="field-label" for="edit-name">{t('feedName.label')}</label>
                <div class="field-row">
                    <input
                        id="edit-name"
                        class="text-input"
                        type="text"
                        value={titleValue}
                        placeholder={feed.url}
                        oninput={(event) => (titleDraft = event.currentTarget.value)}
                    />
                    <button
                        type="button"
                        class="ghost"
                        disabled={titleValue === currentTitle}
                        onclick={() => void saveTitle()}>{t('feedName.save')}</button
                    >
                </div>
            </div>

            <div class="field-block">
                <label class="field-label" for="edit-site">{t('editFeed.site.label')}</label>
                <input
                    id="edit-site"
                    class="text-input"
                    type="url"
                    value={siteValue}
                    placeholder={t('editFeed.site.placeholder')}
                    oninput={(event) => (siteDraft = event.currentTarget.value)}
                />
            </div>

            <div class="field-block">
                <label class="field-label" for="edit-desc">{t('editFeed.description.label')}</label>
                <textarea
                    id="edit-desc"
                    class="text-input"
                    rows="2"
                    value={descValue}
                    oninput={(event) => (descDraft = event.currentTarget.value)}
                ></textarea>
                <div class="field-row field-row-end">
                    <button
                        type="button"
                        class="ghost"
                        disabled={!detailsDirty}
                        onclick={() => void saveDetails()}>{t('editFeed.details.save')}</button
                    >
                </div>
            </div>

            <div class="field-block">
                <span class="field-label" id="edit-tags-label">{t('feedTags.label')}</span>
                <TagEditor
                    tags={feed.tags}
                    label={t('feedTags.label')}
                    describedById="edit-tags-hint"
                    onchange={(next) => void saveTags(next)}
                />
                <p class="field-hint" id="edit-tags-hint">{t('feedTags.hint')}</p>
            </div>
        </section>

        <section class="health" aria-label={t('editFeed.section.health')} bind:this={healthEl}>
            <h3>{t('editFeed.section.health')}</h3>
            <div class="controls" role="group" aria-label={t('feedHealth.status')}>
                {#if feed.status === 'active'}
                    <button type="button" onclick={() => void setStatus('paused')}
                        >{t('feedHealth.pause')}</button
                    >
                {:else if feed.status === 'paused'}
                    <button type="button" onclick={() => void setStatus('active')}
                        >{t('feedHealth.resume')}</button
                    >
                {:else}
                    <button type="button" onclick={() => void setStatus('active')}
                        >{t('feedHealth.revive')}</button
                    >
                {/if}
                <button type="button" onclick={() => void markAllRead(feedId)}
                    >{t('feedMarkRead.action')}</button
                >
            </div>

            <div class="recent" aria-label={t('feedHealth.recent')}>
                <h4>{t('feedHealth.recent')}</h4>
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
                                    <span class="fetch-new"
                                        >{t('feedHealth.new', { count: record.articles_new })}</span
                                    >
                                {/if}
                                {#if record.error !== null}
                                    <span class="fetch-error truncate" title={record.error}
                                        >{record.error}</span
                                    >
                                {/if}
                            </li>
                        {/each}
                    </ul>
                {/if}
                {#if fetchHint !== null}
                    <p class="fetch-hint" role="note">{fetchHint}</p>
                {/if}
            </div>
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
                <button
                    type="button"
                    class="danger-trigger"
                    onclick={() => (confirmingRemove = true)}>{t('feedHealth.unsubscribe')}</button
                >
            {/if}
        </section>
    {/if}
</Modal>

<style>
    .head {
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

    .field,
    .health {
        display: flex;
        flex-direction: column;
        gap: var(--space-3);
    }

    .field-block {
        display: flex;
        flex-direction: column;
        gap: var(--space-1);
    }

    .field-label {
        font-size: 0.6875rem;
        font-weight: 650;
        letter-spacing: var(--tracking-caps);
        text-transform: uppercase;
        color: var(--fg-subtle);
    }

    .field-row {
        display: flex;
        gap: var(--space-2);
        align-items: center;
    }

    .field-row-end {
        justify-content: flex-end;
    }

    .url-value {
        flex: 1 1 auto;
        min-width: 0;
        padding: var(--space-2) var(--space-3);
        border-radius: var(--radius-md);
        background: var(--surface-inset);
        border: 1px solid var(--hairline);
        color: var(--fg-muted);
        font-family: var(--font-mono);
        font-size: var(--text-sm);
    }

    .text-input {
        flex: 1 1 auto;
        min-width: 0;
        width: 100%;
        padding: var(--space-2) var(--space-3);
        border-radius: var(--radius-md);
        background: var(--surface-raised);
        border: 1px solid var(--hairline-strong);
        color: var(--fg);
        font-size: var(--text-md);
        font-family: inherit;
        resize: vertical;
    }

    .ghost {
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

    .ghost:hover:not(:disabled) {
        background: var(--hover);
        color: var(--fg);
    }

    .ghost:disabled {
        opacity: 0.5;
    }

    .field-hint {
        font-size: var(--text-xs);
        color: var(--fg-subtle);
    }

    .health {
        margin-top: var(--space-4);
        padding-top: var(--space-3);
        border-top: 1px solid var(--hairline);
    }

    .health h3 {
        font-size: 0.6875rem;
        font-weight: 650;
        letter-spacing: var(--tracking-caps);
        text-transform: uppercase;
        color: var(--fg-subtle);
    }

    .recent h4 {
        font-size: 0.6875rem;
        font-weight: 650;
        letter-spacing: var(--tracking-caps);
        text-transform: uppercase;
        color: var(--fg-subtle);
        margin-bottom: var(--space-2);
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
</style>

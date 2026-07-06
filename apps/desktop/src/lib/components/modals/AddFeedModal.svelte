<script lang="ts">
/**
 * Add-feed modal (ported from the sketch's error-unwrap reference to runes).
 * Autodiscovery: the typed URL is fetched once Rust-side (`discover_feeds`,
 * policed) and scanned for `<link rel="alternate">` feeds + a favicon.
 * Favicon precedence is same-origin only; the Google favicon service is an
 * opt-in fallback (`uiStore.allowRemoteFavicon`) applied here — nothing
 * reaches Google unless the user turned it on. Adding dedupes against
 * `get_feed_by_url` first, then subscribes through the feeds store.
 */
import CachedImage from '$components/common/CachedImage.svelte';
import Modal from '$components/common/Modal.svelte';
import { type DiscoveredFeedDto, commands } from '$lib/bindings';
import { t } from '$lib/i18n';
import { toastCommandError } from '$lib/state/actions';
import { feedsStore } from '$lib/state/feeds.svelte';
import { uiStore } from '$lib/state/ui.svelte';

let { onclose }: { onclose: () => void } = $props();

let url = $state('');
let tagsInput = $state('');
let candidates: DiscoveredFeedDto[] = $state([]);
let selectedUrl: string | null = $state(null);
let faviconSrc: string | null = $state(null);
let triedRemoteFavicon = $state(false);
let finding = $state(false);
let adding = $state(false);
let searched = $state(false);

/** The URL a subscribe will use: a chosen candidate, else the typed URL. */
function targetUrl(): string {
    return (selectedUrl ?? url).trim();
}

function parsedTags(): string[] {
    return tagsInput
        .split(',')
        .map((tag) => tag.trim())
        .filter((tag) => tag.length > 0);
}

async function find(): Promise<void> {
    const query = url.trim();
    if (query.length === 0) {
        uiStore.showToast(t('addFeed.needUrl'), 'warning');
        return;
    }
    finding = true;
    triedRemoteFavicon = false;
    try {
        const result = await commands.discoverFeeds(query);
        if (result.status === 'error') {
            toastCommandError(result.error);
            return;
        }
        candidates = result.data.feeds;
        faviconSrc = result.data.favicon;
        selectedUrl = candidates.length > 0 ? candidates[0].url : null;
        searched = true;
    } catch {
        uiStore.showToast(t('app.error.internal'), 'error');
    } finally {
        finding = false;
    }
}

function onFaviconFail(): void {
    // The same-origin favicon 404'd; fall back to Google's service only if
    // the user opted in (a privacy choice — see PRIVACY.md).
    if (triedRemoteFavicon || !uiStore.allowRemoteFavicon) {
        faviconSrc = null;
        return;
    }
    triedRemoteFavicon = true;
    const host = hostOf(url);
    faviconSrc = host === null ? null : `https://www.google.com/s2/favicons?domain=${host}&sz=64`;
}

function hostOf(value: string): string | null {
    try {
        return new URL(value.trim()).hostname || null;
    } catch {
        return null;
    }
}

async function add(): Promise<void> {
    const target = targetUrl();
    if (target.length === 0) {
        uiStore.showToast(t('addFeed.needUrl'), 'warning');
        return;
    }
    adding = true;
    try {
        const existing = await commands.getFeedByUrl(target);
        if (existing.status === 'error') {
            toastCommandError(existing.error);
            return;
        }
        if (existing.data !== null) {
            uiStore.showToast(t('addFeed.already'), 'warning');
            return;
        }
        const result = await feedsStore.addFeed({ url: target, title: null, tags: parsedTags() });
        if (result.status === 'error') {
            toastCommandError(result.error);
            return;
        }
        uiStore.showToast(
            t('addFeed.added', { title: result.data.title ?? result.data.url }),
            'success',
        );
        onclose();
    } catch {
        uiStore.showToast(t('app.error.internal'), 'error');
    } finally {
        adding = false;
    }
}
</script>

<Modal title={t('addFeed.title')} {onclose}>
    <form
        class="add-feed"
        onsubmit={(event) => {
            event.preventDefault();
            void add();
        }}
    >
        <label class="field">
            <span class="field-label">{t('addFeed.urlLabel')}</span>
            <div class="url-row">
                {#if faviconSrc !== null}
                    <CachedImage src={faviconSrc} alt={t('addFeed.favicon.alt')} size={18} onfail={onFaviconFail} />
                {/if}
                <input
                    class="url-input"
                    type="url"
                    bind:value={url}
                    placeholder={t('addFeed.urlPlaceholder')}
                    autocomplete="off"
                    spellcheck="false"
                />
                <button class="find-button" type="button" onclick={() => void find()} disabled={finding}>
                    {finding ? t('addFeed.finding') : t('addFeed.find')}
                </button>
            </div>
        </label>

        {#if searched}
            {#if candidates.length > 0}
                <fieldset class="candidates">
                    <legend>{t('addFeed.candidates')}</legend>
                    {#each candidates as candidate (candidate.url)}
                        <label class="candidate">
                            <input type="radio" name="feed" value={candidate.url} bind:group={selectedUrl} />
                            <span class="candidate-main">
                                <span class="candidate-title truncate"
                                    >{candidate.title ?? candidate.url}</span
                                >
                                <span class="candidate-url truncate">{candidate.url}</span>
                            </span>
                        </label>
                    {/each}
                </fieldset>
            {:else}
                <p class="status">{t('addFeed.noneFound')}</p>
            {/if}
        {/if}

        <label class="field">
            <span class="field-label">{t('addFeed.tagsLabel')}</span>
            <input
                class="tags-input"
                type="text"
                bind:value={tagsInput}
                placeholder={t('addFeed.tagsPlaceholder')}
            />
        </label>

        <label class="favicon-optin">
            <input
                type="checkbox"
                checked={uiStore.allowRemoteFavicon}
                onchange={(event) => void uiStore.setAllowRemoteFavicon(event.currentTarget.checked)}
            />
            <span>
                <span class="optin-label">{t('addFeed.favicon.allowRemote')}</span>
                <span class="optin-hint">{t('addFeed.favicon.allowRemoteHint')}</span>
            </span>
        </label>

        <div class="actions">
            <button class="add-button" type="submit" disabled={adding}>
                {adding ? t('addFeed.adding') : t('addFeed.add')}
            </button>
        </div>
    </form>
</Modal>

<style>
    .add-feed {
        display: flex;
        flex-direction: column;
        gap: var(--space-4);
    }

    .field {
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
    }

    .field-label {
        font-size: 0.75rem;
        font-weight: 600;
        color: var(--fg-subtle);
    }

    .url-row {
        display: flex;
        align-items: center;
        gap: var(--space-2);
    }

    .url-input,
    .tags-input {
        flex: 1 1 auto;
        min-width: 0;
        padding: var(--space-2) var(--space-3);
        border-radius: var(--radius-md);
        background: var(--bg);
        color: var(--fg);
        border: 1px solid var(--border-subtle);
        font-size: 0.875rem;
    }

    .url-input:focus-visible,
    .tags-input:focus-visible {
        outline: none;
        border-color: var(--accent);
    }

    .find-button,
    .add-button {
        flex: 0 0 auto;
        padding: var(--space-2) var(--space-3);
        border-radius: var(--radius-md);
        font-size: 0.8125rem;
    }

    .find-button {
        background: transparent;
        color: var(--fg);
        border: 1px solid var(--border);
    }

    .find-button:hover:not(:disabled) {
        background: var(--bg-hover);
    }

    .add-button {
        background: var(--accent);
        color: var(--accent-fg);
        font-weight: 600;
    }

    .candidates {
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
        padding: var(--space-3);
        border: 1px solid var(--border-subtle);
        border-radius: var(--radius-md);
    }

    .candidates legend {
        padding: 0 var(--space-1);
        font-size: 0.75rem;
        font-weight: 600;
        color: var(--fg-subtle);
    }

    .candidate {
        display: flex;
        align-items: center;
        gap: var(--space-2);
    }

    .candidate-main {
        display: flex;
        flex-direction: column;
        min-width: 0;
    }

    .candidate-title {
        font-size: 0.8125rem;
        color: var(--fg);
    }

    .candidate-url {
        font-size: 0.6875rem;
        color: var(--fg-muted);
        font-family: var(--font-mono);
    }

    .status {
        font-size: 0.8125rem;
        color: var(--fg-muted);
    }

    .favicon-optin {
        display: flex;
        gap: var(--space-2);
        align-items: flex-start;
        font-size: 0.8125rem;
    }

    .optin-label {
        display: block;
        color: var(--fg);
    }

    .optin-hint {
        display: block;
        color: var(--fg-muted);
        font-size: 0.75rem;
    }

    .actions {
        display: flex;
        justify-content: flex-end;
    }
</style>

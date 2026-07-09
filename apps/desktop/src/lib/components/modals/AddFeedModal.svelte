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
import { HN_PRESETS, detectSource } from '$lib/utils/source-presets';

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
let urlInput = $state<HTMLInputElement>();
let showHnPresets = $state(false);

// What the typed string resolves to, if it's a recognized source (a
// subreddit, Mastodon handle, YouTube channel, HN URL). A YouTube @handle
// isn't recognized here — it needs discovery to find its channel id.
const detected = $derived(detectSource(url));

/**
 * The URL a subscribe will use: a recognized source's constructed feed URL
 * wins, then a chosen discovery candidate, else the typed URL.
 */
function targetUrl(): string {
    return (detected?.feedUrl ?? selectedUrl ?? url).trim();
}

/** Fills the URL input with a starter pattern and focuses it (presets row). */
function useTemplate(template: string): void {
    url = template;
    showHnPresets = false;
    urlInput?.focus();
}

/** Picks a Hacker News feed: sets the URL and suggests the HN folder. */
function pickHnPreset(feedUrl: string): void {
    url = feedUrl;
    showHnPresets = false;
    if (tagsInput.trim() === '') {
        tagsInput = 'Community/Hacker News';
    }
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
        // A recognized source suggests its folder when the user left tags empty.
        const tags = parsedTags();
        if (tags.length === 0 && detected !== null) {
            tags.push(detected.suggestedTag);
        }
        const result = await feedsStore.addFeed({ url: target, title: null, tags });
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
                    type="text"
                    bind:this={urlInput}
                    bind:value={url}
                    placeholder={t('addFeed.urlPlaceholder')}
                    autocomplete="off"
                    spellcheck="false"
                />
                <button
                    class="find-button"
                    type="button"
                    onclick={() => void find()}
                    disabled={finding || detected !== null}
                >
                    {finding ? t('addFeed.finding') : t('addFeed.find')}
                </button>
            </div>
            {#if detected !== null}
                <p class="detected" role="note">
                    <span class="detected-label">{t('presets.detected')}</span>
                    <span class="detected-url truncate">{detected.feedUrl}</span>
                </p>
            {/if}
        </label>

        <div class="presets">
            <span class="field-label">{t('presets.title')}</span>
            <div class="preset-row">
                <button type="button" class="preset" onclick={() => useTemplate('r/')}
                    >{t('presets.reddit')}</button
                >
                <button
                    type="button"
                    class="preset"
                    onclick={() => useTemplate('https://www.youtube.com/@')}
                    >{t('presets.youtube')}</button
                >
                <button type="button" class="preset" onclick={() => useTemplate('@')}
                    >{t('presets.mastodon')}</button
                >
                <button
                    type="button"
                    class="preset"
                    aria-expanded={showHnPresets}
                    onclick={() => (showHnPresets = !showHnPresets)}>{t('presets.hn')}</button
                >
            </div>
            {#if showHnPresets}
                <div class="preset-row hn-row">
                    {#each HN_PRESETS as preset (preset.id)}
                        <button
                            type="button"
                            class="preset preset-sm"
                            onclick={() => pickHnPreset(preset.feedUrl)}>{t(preset.labelKey)}</button
                        >
                    {/each}
                </div>
            {/if}
        </div>

        {#if searched && detected === null}
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
        font-size: var(--text-xs);
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
        height: 34px;
        padding: var(--space-2) var(--space-3);
        border-radius: var(--radius-md);
        background: var(--surface-inset);
        color: var(--fg);
        border: 1px solid var(--hairline);
        font-size: var(--text-md);
        transition:
            border-color var(--dur-fast) var(--ease),
            box-shadow var(--dur-fast) var(--ease);
    }

    .url-input:focus-visible,
    .tags-input:focus-visible {
        outline: none;
        border-color: color-mix(in srgb, var(--accent), transparent 40%);
        box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent), transparent 82%);
    }

    .find-button,
    .add-button {
        flex: 0 0 auto;
        border-radius: var(--radius-md);
        font-size: var(--text-sm);
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .find-button {
        padding: var(--space-2) var(--space-3);
        background: transparent;
        color: var(--fg-muted);
        border: 1px solid var(--hairline-strong);
    }

    .find-button:hover:not(:disabled) {
        background: var(--hover);
        color: var(--fg);
    }

    .add-button {
        padding: var(--space-2) var(--space-4);
        background: var(--accent);
        color: var(--accent-fg);
        font-weight: 560;
    }

    .add-button:hover:not(:disabled) {
        background: var(--accent-hover);
    }

    .candidates {
        display: flex;
        flex-direction: column;
        gap: var(--space-1);
        padding: var(--space-2);
        background: var(--surface-inset);
        border: 1px solid var(--hairline);
        border-radius: var(--radius-lg);
    }

    .candidates legend {
        padding: 0 var(--space-1);
        font-size: var(--text-xs);
        font-weight: 600;
        color: var(--fg-subtle);
    }

    .candidate {
        display: flex;
        align-items: center;
        gap: var(--space-2);
        padding: var(--space-2) var(--space-3);
        border-radius: var(--radius-md);
        cursor: pointer;
        transition: background var(--dur-fast) var(--ease);
    }

    .candidate:hover {
        background: var(--hover);
    }

    .candidate-main {
        display: flex;
        flex-direction: column;
        min-width: 0;
    }

    .candidate-title {
        font-size: var(--text-sm);
        color: var(--fg);
    }

    .candidate-url {
        font-size: var(--text-xs);
        color: var(--fg-subtle);
        font-family: var(--font-mono);
    }

    .status {
        font-size: var(--text-sm);
        color: var(--fg-muted);
    }

    .favicon-optin {
        display: flex;
        gap: var(--space-2);
        align-items: flex-start;
        font-size: var(--text-sm);
    }

    .optin-label {
        display: block;
        color: var(--fg);
    }

    .optin-hint {
        display: block;
        color: var(--fg-subtle);
        font-size: var(--text-xs);
    }

    .actions {
        display: flex;
        justify-content: flex-end;
    }

    .detected {
        display: flex;
        align-items: baseline;
        gap: var(--space-2);
        margin-top: var(--space-1);
        min-width: 0;
    }

    .detected-label {
        flex: 0 0 auto;
        font-size: var(--text-xs);
        color: var(--fg-subtle);
    }

    .detected-url {
        min-width: 0;
        font-family: var(--font-mono);
        font-size: var(--text-xs);
        color: var(--accent);
    }

    .presets {
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
    }

    .preset-row {
        display: flex;
        flex-wrap: wrap;
        gap: var(--space-2);
    }

    .hn-row {
        padding-left: var(--space-2);
    }

    .preset {
        padding: var(--space-1) var(--space-3);
        border-radius: var(--radius-pill);
        background: var(--surface-inset);
        color: var(--fg-muted);
        border: 1px solid var(--hairline);
        font-size: var(--text-sm);
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .preset:hover {
        background: var(--hover);
        color: var(--fg);
    }

    .preset-sm {
        font-size: var(--text-xs);
    }
</style>

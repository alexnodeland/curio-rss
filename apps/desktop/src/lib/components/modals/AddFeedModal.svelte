<script lang="ts">
/**
 * Add-feed modal (ported from the sketch's error-unwrap reference to runes).
 * Autodiscovery: `discover_feeds` hands the typed text to the core's
 * discovery (`curio_core::feeds::discover`, policed) — it normalizes a bare
 * domain, applies the fetch-free site rewrites, scans the page's declared
 * feeds, probes same-origin common paths, and offers only candidates it
 * fetched and parsed, plus a favicon. Favicon precedence is same-origin only; the Google favicon service is an
 * opt-in fallback (`prefsStore.allowRemoteFavicon`) applied here — nothing
 * reaches Google unless the user turned it on. Adding dedupes against
 * `get_feed_by_url` first, then subscribes through the feeds store — and
 * `add_feed` validates: what was typed is resolved through the same core
 * discovery, so a homepage or bare domain subscribes to its feed and a page
 * with none (`NO_FEED`) or an unreachable one is refused, with an explicit
 * "use this URL directly" (`validate: false`) offered instead.
 */
import CachedImage from '$components/common/CachedImage.svelte';
import Modal from '$components/common/Modal.svelte';
import { type CommandError, type DiscoveredFeedDto, commands } from '$lib/bindings';
import { t } from '$lib/i18n';
import { toastCommandError } from '$lib/state/actions';
import { feedsStore } from '$lib/state/feeds.svelte';
import { prefsStore } from '$lib/state/prefs.svelte';
import { toastStore } from '$lib/state/toasts.svelte';
import { openExternal } from '$lib/utils/external';
import {
    HN_PRESETS,
    KILL_THE_NEWSLETTER_URL,
    detectSource,
    hostOf,
} from '$lib/utils/source-presets';

let { onclose }: { onclose: () => void } = $props();

let url = $state('');
let tagsInput = $state('');
let candidates: DiscoveredFeedDto[] = $state([]);
let selectedUrl: string | null = $state(null);
let faviconSrc: string | null = $state(null);
let triedRemoteFavicon = $state(false);
let finding = $state(false);
let adding = $state(false);
let savingLink = $state(false);
let searched = $state(false);
let urlInput = $state<HTMLInputElement>();
let showHnPresets = $state(false);
let showNewsletterHelp = $state(false);
// Why the last subscribe (or find) could not validate the URL — the cue to
// offer subscribing to it unvalidated. `null`: nothing failed.
let unverified = $state<'noFeed' | 'unreachable' | null>(null);
// Bumped by every edit of the URL (`resetDiscovery`). A find or subscribe
// captures it before awaiting the core — which can take the whole discovery
// budget — and drops its result if the URL changed meanwhile, so a late
// reply about an earlier URL never unlocks the escape hatch for a new one.
let generation = 0;

// What the typed string resolves to, if it's a recognized source (a
// subreddit, Mastodon handle, YouTube channel, HN URL). A YouTube @handle
// isn't recognized here — it needs discovery to find its channel id.
const detected = $derived(detectSource(url));

// The typed text is a fetchable page URL ⇒ offer the read-later save
// (the GoodLinks path: one article, no subscription).
const canSaveLink = $derived(/^https?:\/\//i.test(url.trim()));

// Discovery found nothing, or a subscribe could not validate: say so and
// offer the explicit unvalidated subscribe.
const offerUnvalidated = $derived(
    unverified !== null || (searched && detected === null && candidates.length === 0),
);

/**
 * The URL a subscribe will use: a recognized source's constructed feed URL
 * wins, then a chosen discovery candidate, else the typed URL.
 */
function targetUrl(): string {
    return (detected?.feedUrl ?? selectedUrl ?? url).trim();
}

/** Clears any prior discovery result — the candidates belong to an earlier URL
 *  and must not survive an edit, or `add()` would subscribe to a stale one. */
function resetDiscovery(): void {
    generation += 1;
    candidates = [];
    selectedUrl = null;
    searched = false;
    faviconSrc = null;
    triedRemoteFavicon = false;
    unverified = null;
}

/** Fills the URL input with a starter pattern and focuses it (presets row). */
function useTemplate(template: string): void {
    url = template;
    resetDiscovery();
    showHnPresets = false;
    showNewsletterHelp = false;
    urlInput?.focus();
}

/** Toggles the newsletters-as-feeds explainer (presets row). */
function toggleNewsletterHelp(): void {
    showNewsletterHelp = !showNewsletterHelp;
    showHnPresets = false;
}

/** Picks a Hacker News feed: sets the URL and suggests the HN folder. */
function pickHnPreset(feedUrl: string): void {
    url = feedUrl;
    resetDiscovery();
    showHnPresets = false;
    showNewsletterHelp = false;
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
    if (finding) {
        return; // reentrancy guard — an Enter + button double-fire is one find
    }
    const query = url.trim();
    if (query.length === 0) {
        toastStore.showToast(t('addFeed.needUrl'), 'warning');
        return;
    }
    finding = true;
    triedRemoteFavicon = false;
    const asked = generation;
    try {
        const result = await commands.discoverFeeds(query);
        if (asked !== generation) {
            return; // the URL was edited while discovery ran: this answer is stale
        }
        if (result.status === 'error') {
            toastCommandError(result.error);
            if (result.error.code === 'NETWORK') {
                unverified = 'unreachable';
            }
            return;
        }
        candidates = result.data.feeds;
        faviconSrc = result.data.favicon;
        selectedUrl = candidates.length > 0 ? candidates[0].url : null;
        searched = true;
    } catch {
        toastStore.showToast(t('app.error.internal'), 'error');
    } finally {
        finding = false;
    }
}

function onFaviconFail(): void {
    // The same-origin favicon 404'd; fall back to Google's service only if
    // the user opted in (a privacy choice — see PRIVACY.md).
    if (triedRemoteFavicon || !prefsStore.allowRemoteFavicon) {
        faviconSrc = null;
        return;
    }
    triedRemoteFavicon = true;
    const host = hostOf(url);
    faviconSrc = host === null ? null : `https://www.google.com/s2/favicons?domain=${host}&sz=64`;
}

/**
 * Subscribes to {@link targetUrl}. `validate` (the default) has the core
 * resolve and check it first; `false` is the explicit "use this URL
 * directly" escape hatch offered once validation failed.
 */
async function add(validate = true): Promise<void> {
    if (adding) {
        return; // reentrancy guard against a double-submit
    }
    const target = targetUrl();
    if (target.length === 0) {
        toastStore.showToast(t('addFeed.needUrl'), 'warning');
        return;
    }
    adding = true;
    const asked = generation;
    try {
        const existing = await commands.getFeedByUrl(target);
        if (existing.status === 'error') {
            toastCommandError(existing.error);
            return;
        }
        if (existing.data !== null) {
            toastStore.showToast(t('addFeed.already'), 'warning');
            return;
        }
        // A recognized source suggests its folder when the user left tags empty.
        const tags = parsedTags();
        if (tags.length === 0 && detected !== null) {
            tags.push(detected.suggestedTag);
        }
        const result = await feedsStore.addFeed({ url: target, title: null, tags, validate });
        if (result.status === 'error') {
            subscribeFailed(result.error, asked !== generation);
            return;
        }
        toastStore.showToast(
            t('addFeed.added', { title: result.data.title ?? result.data.url }),
            'success',
        );
        onclose();
    } catch {
        toastStore.showToast(t('app.error.internal'), 'error');
    } finally {
        adding = false;
    }
}

/** Presents a refused subscribe: a duplicate warns, an unvalidated URL
 *  offers the escape hatch, anything else toasts. `stale`: the URL was
 *  edited while the subscribe ran, so the refusal is about a URL no longer
 *  in the field — it must not offer the escape hatch for the new one. */
function subscribeFailed(error: CommandError, stale: boolean): void {
    switch (error.code) {
        case 'ALREADY_SUBSCRIBED':
            toastStore.showToast(t('addFeed.already'), 'warning');
            return;
        case 'NO_FEED':
            // Said inline, beside the escape hatch — no toast on top, and
            // nothing at all once the field shows a different URL.
            if (!stale) {
                unverified = 'noFeed';
            }
            return;
        case 'NETWORK':
            if (!stale) {
                unverified = 'unreachable';
            }
            toastCommandError(error);
            return;
        default:
            toastCommandError(error);
    }
}

/** Saves the typed URL itself as a read-later article (no subscription). */
async function saveLink(): Promise<void> {
    if (savingLink) {
        return; // reentrancy guard against a double-click
    }
    savingLink = true;
    try {
        const result = await commands.saveUrl(url.trim(), parsedTags());
        if (result.status === 'error') {
            toastCommandError(result.error);
            return;
        }
        if (result.data.created && !result.data.hydrated) {
            toastStore.showToast(t('addFeed.linkSavedBare'), 'warning');
        } else {
            toastStore.showToast(
                t('addFeed.linkSaved', { title: result.data.article.title }),
                'success',
            );
        }
        onclose();
    } catch {
        toastStore.showToast(t('app.error.internal'), 'error');
    } finally {
        savingLink = false;
    }
}
</script>

<Modal title={t('addFeed.title')} {onclose}>
    <form
        id="add-feed-form"
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
                    data-testid="add-feed-url"
                    type="text"
                    bind:this={urlInput}
                    bind:value={url}
                    oninput={resetDiscovery}
                    onkeydown={(event) => {
                        // Enter with nothing discovered yet finds feeds first
                        // (rather than submitting a raw, likely-not-a-feed URL);
                        // once a source is recognized or a candidate is chosen,
                        // Enter falls through to the form's subscribe.
                        if (
                            event.key === 'Enter' &&
                            detected === null &&
                            !(searched && selectedUrl !== null)
                        ) {
                            event.preventDefault();
                            void find();
                        }
                    }}
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
                    onclick={() => useTemplate('https://github.com/')}
                    >{t('presets.github')}</button
                >
                <button
                    type="button"
                    class="preset"
                    aria-expanded={showNewsletterHelp}
                    onclick={toggleNewsletterHelp}>{t('presets.newsletter')}</button
                >
                <button
                    type="button"
                    class="preset"
                    aria-expanded={showHnPresets}
                    onclick={() => {
                        showHnPresets = !showHnPresets;
                        showNewsletterHelp = false;
                    }}>{t('presets.hn')}</button
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
            {#if showNewsletterHelp}
                <div class="newsletter-help" role="note">
                    <p>{t('presets.newsletter.hint')}</p>
                    <button
                        type="button"
                        class="link-button"
                        onclick={() => void openExternal(KILL_THE_NEWSLETTER_URL)}
                        >{t('presets.newsletter.open')}</button
                    >
                </div>
            {/if}
        </div>

        {#if searched && detected === null && candidates.length > 0}
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
        {/if}
        {#if offerUnvalidated}
            <div class="unvalidated" role="status">
                <p class="status">
                    {unverified === 'unreachable' ? t('addFeed.unreachable') : t('addFeed.noneFound')}
                </p>
                <button
                    type="button"
                    class="link-button"
                    data-testid="add-feed-unvalidated"
                    onclick={() => void add(false)}
                    disabled={adding}>{t('addFeed.useUrl')}</button
                >
            </div>
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
                checked={prefsStore.allowRemoteFavicon}
                onchange={(event) => void prefsStore.setAllowRemoteFavicon(event.currentTarget.checked)}
            />
            <span>
                <span class="optin-label">{t('addFeed.favicon.allowRemote')}</span>
                <span class="optin-hint">{t('addFeed.favicon.allowRemoteHint')}</span>
            </span>
        </label>

    </form>
    {#snippet footer()}
        <!-- Pinned so Subscribe stays reachable however long the discovery
             list grows; `form=` keeps it submitting the form above. -->
        <button
            class="save-link-button"
            type="button"
            onclick={() => void saveLink()}
            disabled={savingLink || !canSaveLink}
        >
            {savingLink ? t('addFeed.savingLink') : t('addFeed.saveLink')}
        </button>
        <button class="add-button" data-testid="add-feed-submit" type="submit" form="add-feed-form" disabled={adding}>
            {adding ? t('addFeed.adding') : t('addFeed.add')}
        </button>
    {/snippet}
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
    .save-link-button,
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

    .save-link-button {
        padding: var(--space-2) var(--space-3);
        background: transparent;
        color: var(--fg-muted);
        border: 1px solid var(--hairline-strong);
    }

    .save-link-button:hover:not(:disabled) {
        background: var(--hover);
        color: var(--fg);
    }

    .save-link-button:disabled {
        opacity: 0.5;
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

    .unvalidated,
    .newsletter-help {
        display: flex;
        flex-direction: column;
        align-items: flex-start;
        gap: var(--space-1);
    }

    .newsletter-help {
        padding: var(--space-2) var(--space-3);
        background: var(--surface-inset);
        border: 1px solid var(--hairline);
        border-radius: var(--radius-lg);
        font-size: var(--text-sm);
        color: var(--fg-muted);
    }

    .link-button {
        padding: 0;
        background: transparent;
        color: var(--accent);
        font-size: var(--text-sm);
    }

    .link-button:hover:not(:disabled) {
        text-decoration: underline;
    }

    .link-button:disabled {
        opacity: 0.5;
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

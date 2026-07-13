<script lang="ts">
/**
 * The optional Reddit API credentials (BYO OAuth, D15). The secret is
 * write-only: it goes down through `set_reddit_api` into the OS keychain
 * and never comes back — status reports only the client id. Without
 * credentials Reddit keeps working unauthenticated at the slow pacing;
 * this panel is purely the opt-in fast lane.
 */
import { type RedditApiStatusDto, commands } from '$lib/bindings';
import { t } from '$lib/i18n';
import { toastCommandError } from '$lib/state/actions';
import { uiStore } from '$lib/state/ui.svelte';
import { openExternal } from '$lib/utils/external';

let status = $state<RedditApiStatusDto | null>(null);
let clientId = $state('');
let clientSecret = $state('');
let busy = $state(false);

async function refresh(): Promise<void> {
    const result = await commands.getRedditApiStatus();
    if (result.status === 'ok') {
        status = result.data;
    }
}
void refresh();

const canSave = $derived(clientId.trim() !== '' && clientSecret.trim() !== '');

async function save(): Promise<void> {
    if (busy || !canSave) {
        return;
    }
    busy = true;
    try {
        const result = await commands.setRedditApi(clientId.trim(), clientSecret.trim());
        if (result.status === 'error') {
            toastCommandError(result.error);
            return;
        }
        status = result.data;
        clientId = '';
        clientSecret = '';
        uiStore.showToast(t('settings.redditApi.saved'), 'success');
    } catch {
        uiStore.showToast(t('app.error.internal'), 'error');
    } finally {
        busy = false;
    }
}

async function remove(): Promise<void> {
    if (busy) {
        return;
    }
    busy = true;
    try {
        const result = await commands.clearRedditApi();
        if (result.status === 'error') {
            toastCommandError(result.error);
            return;
        }
        status = result.data;
        uiStore.showToast(t('settings.redditApi.removed'), 'success');
    } catch {
        uiStore.showToast(t('app.error.internal'), 'error');
    } finally {
        busy = false;
    }
}
</script>

<section class="reddit-api" aria-label={t('settings.redditApi.title')}>
    <h4>{t('settings.redditApi.title')}</h4>
    <p class="hint">{t('settings.redditApi.hint')}</p>

    {#if status?.configured}
        <p class="status connected">
            {t('settings.redditApi.status.on', { clientId: status.client_id ?? '' })}
        </p>
        <button class="ghost" type="button" disabled={busy} onclick={() => void remove()}>
            {t('settings.redditApi.remove')}
        </button>
    {:else}
        <p class="status">{t('settings.redditApi.status.off')}</p>
        <form
            class="credentials"
            onsubmit={(event) => {
                event.preventDefault();
                void save();
            }}
        >
            <input
                class="cred-input"
                type="text"
                bind:value={clientId}
                placeholder={t('settings.redditApi.clientId')}
                aria-label={t('settings.redditApi.clientId')}
                autocomplete="off"
                spellcheck="false"
            />
            <input
                class="cred-input"
                type="password"
                bind:value={clientSecret}
                placeholder={t('settings.redditApi.clientSecret')}
                aria-label={t('settings.redditApi.clientSecret')}
                autocomplete="off"
            />
            <button class="save" type="submit" disabled={busy || !canSave}>
                {t('settings.redditApi.save')}
            </button>
        </form>
        <button
            class="link-button"
            type="button"
            onclick={() => void openExternal('https://www.reddit.com/prefs/apps')}
        >
            {t('settings.redditApi.create')} ↗
        </button>
    {/if}
</section>

<style>
    .reddit-api {
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
        padding-top: var(--space-4);
        margin-top: var(--space-2);
        border-top: 1px solid var(--hairline);
    }

    h4 {
        font-size: 0.6875rem;
        font-weight: 650;
        letter-spacing: var(--tracking-caps);
        text-transform: uppercase;
        color: var(--fg-subtle);
    }

    .hint {
        font-size: var(--text-sm);
        color: var(--fg-subtle);
        line-height: 1.45;
    }

    .status {
        font-size: var(--text-sm);
        color: var(--fg-muted);
    }

    .status.connected {
        color: var(--success);
    }

    .credentials {
        display: flex;
        flex-wrap: wrap;
        gap: var(--space-2);
        align-items: center;
    }

    .cred-input {
        flex: 1 1 10rem;
        min-width: 0;
        height: 32px;
        padding: 0 var(--space-3);
        border-radius: var(--radius-md);
        background: var(--surface-inset);
        color: var(--fg);
        border: 1px solid var(--hairline);
        font-size: var(--text-sm);
    }

    .cred-input:focus-visible {
        outline: none;
        border-color: color-mix(in srgb, var(--accent), transparent 40%);
        box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent), transparent 82%);
    }

    .save,
    .ghost {
        flex: 0 0 auto;
        align-self: flex-start;
        height: 32px;
        padding: 0 var(--space-4);
        border-radius: var(--radius-md);
        font-size: var(--text-sm);
        font-weight: 560;
    }

    .save {
        background: var(--accent);
        color: var(--accent-fg);
    }

    .save:hover:not(:disabled) {
        background: var(--accent-hover);
    }

    .save:disabled {
        opacity: 0.5;
    }

    .ghost {
        background: transparent;
        color: var(--fg-muted);
        border: 1px solid var(--hairline-strong);
    }

    .ghost:hover:not(:disabled) {
        background: var(--hover);
        color: var(--fg);
    }

    .link-button {
        align-self: flex-start;
        background: transparent;
        color: var(--accent);
        font-size: var(--text-xs);
        font-weight: 500;
    }

    .link-button:hover {
        text-decoration: underline;
    }
</style>

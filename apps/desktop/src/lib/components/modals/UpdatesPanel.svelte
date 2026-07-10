<script lang="ts">
/**
 * App-updates controls for Settings → General: the running version, a manual
 * "Check for updates" with inline status, an "Install and restart" flow with
 * download progress, and the auto-check / auto-install toggles (persisted on
 * the uiStore). The updater runs only in the packaged app — in the dev browser
 * the plugin calls throw and the flow lands on the error state, which is fine.
 */
import { commands } from '$lib/bindings';
import { t } from '$lib/i18n';
import { uiStore } from '$lib/state/ui.svelte';
import { type Update, checkForUpdate, installAndRelaunch } from '$lib/utils/updates';

type Status =
    | { kind: 'idle' }
    | { kind: 'checking' }
    | { kind: 'upToDate' }
    | { kind: 'available'; update: Update }
    | { kind: 'installing'; percent: number }
    | { kind: 'error' };

let version = $state('');
let status = $state<Status>({ kind: 'idle' });

$effect(() => {
    void commands.getAppInfo().then((result) => {
        if (result.status === 'ok') {
            version = result.data.version;
        }
    });
});

const busy = $derived(status.kind === 'checking' || status.kind === 'installing');

async function check(): Promise<void> {
    status = { kind: 'checking' };
    try {
        const update = await checkForUpdate();
        status = update === null ? { kind: 'upToDate' } : { kind: 'available', update };
    } catch {
        status = { kind: 'error' };
    }
}

async function install(update: Update): Promise<void> {
    status = { kind: 'installing', percent: 0 };
    try {
        await installAndRelaunch(update, (fraction) => {
            status = { kind: 'installing', percent: Math.round(fraction * 100) };
        });
        // A successful relaunch replaces this process; reaching here is rare.
    } catch {
        status = { kind: 'error' };
    }
}
</script>

<div class="field-block updates">
    <div class="updates-row">
        <span class="field-label">{t('settings.updates.version', { version })}</span>
        <button class="field-button" type="button" disabled={busy} onclick={check}>
            {status.kind === 'checking' ? t('settings.updates.checking') : t('settings.updates.check')}
        </button>
    </div>

    {#if status.kind === 'upToDate'}
        <span class="updates-status">{t('settings.updates.upToDate')}</span>
    {:else if status.kind === 'error'}
        <span class="updates-status error">{t('settings.updates.error')}</span>
    {:else if status.kind === 'available'}
        {@const update = status.update}
        <div class="updates-row">
            <span class="updates-status">
                {t('settings.updates.available', { version: update.version })}
            </span>
            <button class="field-button accent" type="button" onclick={() => install(update)}>
                {t('settings.updates.install')}
            </button>
        </div>
    {:else if status.kind === 'installing'}
        <span class="updates-status">
            {t('settings.updates.installing', { percent: status.percent })}
        </span>
    {/if}

    <label class="toggle">
        <input
            type="checkbox"
            checked={uiStore.updatesAutoCheck}
            onchange={(event) => void uiStore.setUpdatesAutoCheck(event.currentTarget.checked)}
        />
        <span class="toggle-text">
            <span class="toggle-label">{t('settings.updates.autoCheck')}</span>
        </span>
    </label>
    <label class="toggle">
        <input
            type="checkbox"
            checked={uiStore.updatesAutoInstall}
            disabled={!uiStore.updatesAutoCheck}
            onchange={(event) => void uiStore.setUpdatesAutoInstall(event.currentTarget.checked)}
        />
        <span class="toggle-text">
            <span class="toggle-label">{t('settings.updates.autoInstall')}</span>
            <span class="toggle-hint">{t('settings.updates.autoInstall.hint')}</span>
        </span>
    </label>
</div>

<style>
    .updates-row {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: var(--space-3);
    }

    .updates-status {
        font-size: var(--text-sm);
        color: var(--fg-muted);
    }

    .updates-status.error {
        color: var(--error-text);
    }

    .field-button {
        flex: 0 0 auto;
        padding: var(--space-1) var(--space-3);
        border-radius: var(--radius-md);
        background: transparent;
        color: var(--fg-muted);
        border: 1px solid var(--hairline-strong);
        font-size: var(--text-sm);
        font-weight: 500;
        cursor: pointer;
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .field-button:hover:not(:disabled) {
        background: var(--hover);
        color: var(--fg);
    }

    .field-button:disabled {
        opacity: 0.6;
        cursor: default;
    }

    .field-button.accent {
        background: var(--accent);
        color: var(--accent-fg);
        border-color: transparent;
    }

    .field-button.accent:hover {
        background: var(--accent-hover);
        color: var(--accent-fg);
    }

    /* The toggle/field styles are scoped per component, so this panel carries
       its own copy of the shared Settings styles rather than SettingsModal's. */
    .updates.field-block {
        display: flex;
        flex-direction: column;
        gap: var(--space-3);
    }

    .field-label {
        font-size: var(--text-sm);
        color: var(--fg-muted);
    }

    .toggle {
        display: flex;
        align-items: flex-start;
        gap: var(--space-3);
        cursor: pointer;
    }

    .toggle input {
        margin-top: 2px;
        width: 16px;
        height: 16px;
        flex: 0 0 auto;
        accent-color: var(--accent);
    }

    .toggle-text {
        display: flex;
        flex-direction: column;
        gap: 2px;
    }

    .toggle-label {
        font-size: var(--text-md);
        color: var(--fg);
    }

    .toggle-hint {
        font-size: var(--text-xs);
        color: var(--fg-subtle);
    }
</style>

<script lang="ts">
/**
 * The embedded destinations manager used inside the settings modal: list,
 * add (dialog-pick TOKEN → `add_destination`, never a raw path — D6),
 * remove, and choose the default promote target. All logic lives in
 * `destinationsStore`; this is only its settings-surface presentation
 * (the standalone `DestinationsPanel` is the promote-flow entry point).
 */
import { t } from '$lib/i18n';
import { toastCommandError } from '$lib/state/actions';
import { destinationsStore } from '$lib/state/destinations.svelte';
import { uiStore } from '$lib/state/ui.svelte';

let draftName = $state('');
let pickedToken: string | null = $state(null);
let pickedPath: string | null = $state(null);

destinationsStore.prime();

async function chooseFolder(): Promise<void> {
    try {
        const result = await destinationsStore.pickRoot();
        if (result.status === 'error') {
            toastCommandError(result.error);
            return;
        }
        if (result.data !== null) {
            pickedToken = result.data.token;
            pickedPath = result.data.path;
        }
    } catch {
        uiStore.showToast(t('app.error.internal'), 'error');
    }
}

async function add(): Promise<void> {
    const name = draftName.trim();
    if (name.length === 0 || pickedToken === null) {
        uiStore.showToast(t('destinations.needNameAndFolder'), 'warning');
        return;
    }
    try {
        const result = await destinationsStore.add(name, pickedToken);
        if (result.status === 'error') {
            toastCommandError(result.error);
            return;
        }
        draftName = '';
        pickedToken = null;
        pickedPath = null;
    } catch {
        uiStore.showToast(t('app.error.internal'), 'error');
    }
}

async function remove(name: string): Promise<void> {
    const result = await destinationsStore.remove(name);
    if (result.status === 'error') {
        toastCommandError(result.error);
    }
}

async function makeDefault(name: string): Promise<void> {
    const result = await destinationsStore.setSelected(name);
    if (result.status === 'error') {
        toastCommandError(result.error);
    }
}
</script>

<div class="manager">
    {#if destinationsStore.error !== null}
        <p class="status error" role="alert">{destinationsStore.error.message}</p>
    {:else if !destinationsStore.loaded}
        <p class="status">{t('destinations.loading')}</p>
    {:else if destinationsStore.destinations.length === 0}
        <p class="status">{t('destinations.empty')}</p>
    {:else}
        <ul class="destination-list">
            {#each destinationsStore.destinations as destination (destination.name)}
                <li class="destination">
                    <div class="destination-main">
                        <span class="destination-name">{destination.name}</span>
                        <span class="destination-root truncate">{destination.root}</span>
                    </div>
                    {#if destinationsStore.selectedName === destination.name}
                        <span class="default-badge">{t('destinations.default')}</span>
                    {:else}
                        <button
                            class="link-button"
                            type="button"
                            onclick={() => void makeDefault(destination.name)}
                        >
                            {t('destinations.makeDefault')}
                        </button>
                    {/if}
                    <button
                        class="remove-button"
                        type="button"
                        aria-label={t('destinations.remove', { name: destination.name })}
                        onclick={() => void remove(destination.name)}>×</button
                    >
                </li>
            {/each}
        </ul>
    {/if}

    <form
        class="add-form"
        onsubmit={(event) => {
            event.preventDefault();
            void add();
        }}
    >
        <input
            class="name-input"
            type="text"
            bind:value={draftName}
            placeholder={t('destinations.namePlaceholder')}
            aria-label={t('destinations.name')}
        />
        <button class="choose-button" type="button" onclick={() => void chooseFolder()}>
            {t('destinations.chooseFolder')}
        </button>
        {#if pickedPath !== null}
            <span class="picked truncate" title={pickedPath}>{pickedPath}</span>
        {/if}
        <button class="add-button" type="submit">{t('destinations.add')}</button>
    </form>
</div>

<style>
    .manager {
        display: flex;
        flex-direction: column;
        gap: var(--space-3);
    }

    .status {
        font-size: 0.8125rem;
        color: var(--fg-muted);
    }

    .status.error {
        color: var(--error);
    }

    .destination-list {
        list-style: none;
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
    }

    .destination {
        display: flex;
        align-items: center;
        gap: var(--space-3);
        padding: var(--space-2) var(--space-3);
        border-radius: var(--radius-md);
        background: var(--bg-tertiary);
    }

    .destination-main {
        flex: 1 1 auto;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 2px;
    }

    .destination-name {
        font-size: 0.875rem;
        font-weight: 600;
        color: var(--fg);
    }

    .destination-root {
        font-size: 0.75rem;
        color: var(--fg-muted);
        font-family: var(--font-mono);
    }

    .default-badge {
        flex: 0 0 auto;
        padding: 2px var(--space-2);
        border-radius: var(--radius-xl);
        background: var(--accent);
        color: var(--accent-fg);
        font-size: 0.6875rem;
        font-weight: 600;
    }

    .link-button {
        flex: 0 0 auto;
        background: transparent;
        color: var(--accent);
        font-size: 0.75rem;
    }

    .link-button:hover {
        text-decoration: underline;
    }

    .remove-button {
        flex: 0 0 auto;
        display: flex;
        align-items: center;
        justify-content: center;
        width: 1.25rem;
        height: 1.25rem;
        border-radius: 50%;
        background: transparent;
        color: var(--fg-subtle);
        font-size: 1rem;
        line-height: 1;
    }

    .remove-button:hover {
        color: var(--error);
        background: var(--bg-hover);
    }

    .add-form {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--space-2);
    }

    .name-input {
        flex: 1 1 8rem;
        min-width: 0;
        padding: var(--space-1) var(--space-2);
        border-radius: var(--radius-md);
        background: var(--bg);
        color: var(--fg);
        border: 1px solid var(--border-subtle);
        font-size: 0.8125rem;
    }

    .name-input:focus-visible {
        outline: none;
        border-color: var(--accent);
    }

    .choose-button,
    .add-button {
        flex: 0 0 auto;
        padding: var(--space-1) var(--space-3);
        border-radius: var(--radius-md);
        font-size: 0.8125rem;
    }

    .choose-button {
        background: transparent;
        color: var(--fg);
        border: 1px solid var(--border);
    }

    .choose-button:hover {
        background: var(--bg-hover);
    }

    .add-button {
        background: var(--accent);
        color: var(--accent-fg);
    }

    .picked {
        flex: 1 1 100%;
        font-size: 0.75rem;
        color: var(--fg-muted);
        font-family: var(--font-mono);
    }
</style>

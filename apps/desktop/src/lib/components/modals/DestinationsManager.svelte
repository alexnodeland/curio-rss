<script lang="ts">
/**
 * The embedded destinations manager used inside the settings modal: list,
 * add (dialog-pick TOKEN → `add_destination`, never a raw path — D6),
 * remove, and choose the default promote target. All logic lives in
 * `destinationsStore`; this is only its settings-surface presentation
 * (the standalone `DestinationsPanel` is the promote-flow entry point).
 */
import Icon from '$components/common/Icon.svelte';
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
    try {
        const result = await destinationsStore.remove(name);
        if (result.status === 'error') {
            toastCommandError(result.error);
        }
    } catch {
        uiStore.showToast(t('app.error.internal'), 'error');
    }
}

async function makeDefault(name: string): Promise<void> {
    try {
        const result = await destinationsStore.setSelected(name);
        if (result.status === 'error') {
            toastCommandError(result.error);
        }
    } catch {
        uiStore.showToast(t('app.error.internal'), 'error');
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
                        onclick={() => void remove(destination.name)}
                        ><Icon name="close" size={14} /></button
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
        gap: var(--space-4);
    }

    .status {
        font-size: var(--text-md);
        color: var(--fg-muted);
    }

    .status.error {
        color: var(--error-text);
    }

    .destination-list {
        list-style: none;
        display: flex;
        flex-direction: column;
        gap: 2px;
    }

    .destination {
        display: flex;
        align-items: center;
        gap: var(--space-3);
        padding: var(--space-3) var(--space-4);
        border-radius: var(--radius-lg);
        transition: background var(--dur-fast) var(--ease);
    }

    .destination:hover {
        background: var(--hover);
    }

    .destination-main {
        flex: 1 1 auto;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 2px;
    }

    .destination-name {
        font-size: var(--text-md);
        font-weight: 560;
        color: var(--fg);
    }

    .destination-root {
        font-size: var(--text-xs);
        color: var(--fg-subtle);
        font-family: var(--font-mono);
    }

    .default-badge {
        flex: 0 0 auto;
        padding: 2px var(--space-2);
        border-radius: var(--radius-pill);
        background: var(--selected);
        color: var(--accent);
        border: 1px solid color-mix(in srgb, var(--accent), transparent 70%);
        font-size: var(--text-xs);
        font-weight: 600;
    }

    .link-button {
        flex: 0 0 auto;
        background: transparent;
        color: var(--accent);
        font-size: var(--text-xs);
        font-weight: 500;
        transition: color var(--dur-fast) var(--ease);
    }

    .link-button:hover {
        text-decoration: underline;
    }

    .remove-button {
        flex: 0 0 auto;
        display: inline-grid;
        place-items: center;
        width: 26px;
        height: 26px;
        border-radius: var(--radius-md);
        background: transparent;
        color: var(--fg-subtle);
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .remove-button:hover {
        color: var(--error-text);
        background: var(--hover);
    }

    .add-form {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--space-2);
        padding-top: var(--space-4);
        border-top: 1px solid var(--hairline);
    }

    .name-input {
        flex: 1 1 8rem;
        min-width: 0;
        height: 34px;
        padding: 0 var(--space-3);
        border-radius: var(--radius-md);
        background: var(--surface-inset);
        color: var(--fg);
        border: 1px solid var(--hairline);
        font-size: var(--text-md);
        transition:
            border-color var(--dur-fast) var(--ease),
            box-shadow var(--dur-fast) var(--ease);
    }

    .name-input:focus-visible {
        outline: none;
        border-color: color-mix(in srgb, var(--accent), transparent 40%);
        box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent), transparent 82%);
    }

    .choose-button {
        flex: 0 0 auto;
        height: 34px;
        padding: 0 var(--space-3);
        border-radius: var(--radius-md);
        font-size: var(--text-md);
        background: transparent;
        color: var(--fg-muted);
        border: 1px solid var(--hairline-strong);
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .choose-button:hover {
        background: var(--hover);
        color: var(--fg);
    }

    .add-button {
        flex: 0 0 auto;
        height: 34px;
        padding: 0 var(--space-4);
        border-radius: var(--radius-md);
        font-size: var(--text-md);
        font-weight: 560;
        background: var(--accent);
        color: var(--accent-fg);
        transition: background var(--dur-fast) var(--ease);
    }

    .add-button:hover {
        background: var(--accent-hover);
    }

    .picked {
        flex: 1 1 100%;
        font-size: var(--text-xs);
        color: var(--fg-subtle);
        font-family: var(--font-mono);
    }
</style>

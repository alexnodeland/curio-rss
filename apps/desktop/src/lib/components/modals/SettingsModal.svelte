<script lang="ts">
/**
 * The settings modal: a WAI-ARIA tablist (a left rail on wide viewports, a
 * top row on narrow) over six panels — General, Appearance, Reading,
 * Media & Privacy, Data, and Advanced. Each panel is a landmark region for
 * screen readers; keyboard dismissal (Escape) stays the shell's. The active
 * tab is remembered for the session (a module-scope value, not persisted).
 * All six panels render at once and inactive ones carry `hidden`, so the
 * panels that prime commands on mount (destinations, doctor) keep firing on
 * open exactly as before the tabs.
 */
import Modal from '$components/common/Modal.svelte';
import ThemePicker from '$components/common/ThemePicker.svelte';
import DestinationsManager from '$components/modals/DestinationsManager.svelte';
import DoctorPanel from '$components/modals/DoctorPanel.svelte';
import OpmlPanel from '$components/modals/OpmlPanel.svelte';
import TypographyControls from '$components/reader/TypographyControls.svelte';
import { LOCALES, type LocaleId, type MessageKey, localeStore, t } from '$lib/i18n';
import { REFRESH_INTERVAL_OPTIONS, type ReadingDensity, uiStore } from '$lib/state/ui.svelte';

let { onclose }: { onclose: () => void } = $props();

/** Human labels for each background-refresh cadence (minutes; 0 = off). */
const INTERVAL_LABELS: Record<number, MessageKey> = {
    0: 'settings.refreshInterval.off',
    15: 'settings.refreshInterval.m15',
    30: 'settings.refreshInterval.m30',
    60: 'settings.refreshInterval.h1',
    180: 'settings.refreshInterval.h3',
};

/** The list-density options and their labels. */
const DENSITY_OPTIONS: readonly ReadingDensity[] = ['comfortable', 'compact'];
const DENSITY_LABELS: Record<ReadingDensity, MessageKey> = {
    comfortable: 'settings.density.comfortable',
    compact: 'settings.density.compact',
};

const TABS = [
    { id: 'general', panelId: 'settings-panel-general', label: 'settings.section.general' },
    {
        id: 'appearance',
        panelId: 'settings-panel-appearance',
        label: 'settings.section.appearance',
    },
    { id: 'reading', panelId: 'settings-panel-reading', label: 'settings.section.reading' },
    { id: 'media', panelId: 'settings-panel-media', label: 'settings.section.mediaPrivacy' },
    { id: 'data', panelId: 'settings-panel-data', label: 'settings.section.data' },
    { id: 'advanced', panelId: 'settings-panel-advanced', label: 'settings.section.advanced' },
] as const;

// Remembered across opens within a session (not persisted): a plain module var.
let lastOpenTab = 0;

let activeIndex = $state(lastOpenTab);
const tabEls: (HTMLButtonElement | undefined)[] = $state([]);

// Focus follows the active tab — but only once focus is already inside the
// tablist, so opening the modal doesn't yank focus off the dialog's trap.
$effect(() => {
    const active = tabEls[activeIndex];
    if (active === undefined) return;
    const focused = document.activeElement;
    if (focused !== null && tabEls.includes(focused as HTMLButtonElement)) {
        active.focus();
    }
});

function select(index: number): void {
    activeIndex = index;
    lastOpenTab = index;
}

function move(delta: 1 | -1): void {
    const n = TABS.length;
    select((activeIndex + delta + n) % n);
}

function onKeydown(event: KeyboardEvent): void {
    switch (event.key) {
        case 'ArrowRight':
        case 'ArrowDown':
            event.preventDefault();
            move(1);
            break;
        case 'ArrowLeft':
        case 'ArrowUp':
            event.preventDefault();
            move(-1);
            break;
        case 'Home':
            event.preventDefault();
            select(0);
            break;
        case 'End':
            event.preventDefault();
            select(TABS.length - 1);
            break;
        default:
            break;
    }
}
</script>

<Modal title={t('settings.title')} {onclose} size="large">
    <div class="settings-layout">
        <div
            class="tablist"
            role="tablist"
            aria-label={t('settings.tablist.label')}
            aria-orientation="vertical"
            tabindex="-1"
            onkeydown={onKeydown}
        >
            {#each TABS as tab, index (tab.id)}
                <button
                    bind:this={tabEls[index]}
                    type="button"
                    role="tab"
                    id={`settings-tab-${tab.id}`}
                    aria-selected={index === activeIndex}
                    aria-controls={tab.panelId}
                    tabindex={index === activeIndex ? 0 : -1}
                    onclick={() => select(index)}
                >
                    {t(tab.label)}
                </button>
            {/each}
        </div>

        <div class="panels">
            <div
                role="tabpanel"
                id="settings-panel-general"
                aria-labelledby="settings-tab-general"
                tabindex="0"
                hidden={activeIndex !== 0}
            >
                <label class="field">
                    <span class="field-label">{t('settings.language')}</span>
                    <select
                        class="field-select"
                        value={localeStore.active}
                        onchange={(event) =>
                            void localeStore.set(event.currentTarget.value as LocaleId)}
                    >
                        {#each LOCALES as option (option.id)}
                            <option value={option.id}>{option.name}</option>
                        {/each}
                    </select>
                </label>

                <div class="field-block">
                    <label class="field">
                        <span class="field-label">{t('settings.refreshInterval')}</span>
                        <select
                            class="field-select"
                            value={String(uiStore.refreshIntervalMinutes)}
                            onchange={(event) =>
                                void uiStore.setRefreshIntervalMinutes(
                                    Number(event.currentTarget.value),
                                )}
                        >
                            {#each REFRESH_INTERVAL_OPTIONS as minutes (minutes)}
                                <option value={String(minutes)}>{t(INTERVAL_LABELS[minutes])}</option>
                            {/each}
                        </select>
                    </label>
                    <span class="toggle-hint">{t('settings.refreshInterval.hint')}</span>
                </div>

                <label class="toggle">
                    <input
                        type="checkbox"
                        checked={uiStore.refreshOnLaunch}
                        onchange={(event) =>
                            void uiStore.setRefreshOnLaunch(event.currentTarget.checked)}
                    />
                    <span class="toggle-text">
                        <span class="toggle-label">{t('settings.refreshOnLaunch')}</span>
                        <span class="toggle-hint">{t('settings.refreshOnLaunch.hint')}</span>
                    </span>
                </label>
            </div>

            <div
                role="tabpanel"
                id="settings-panel-appearance"
                aria-labelledby="settings-tab-appearance"
                tabindex="0"
                hidden={activeIndex !== 1}
            >
                <ThemePicker />
            </div>

            <div
                role="tabpanel"
                id="settings-panel-reading"
                aria-labelledby="settings-tab-reading"
                tabindex="0"
                hidden={activeIndex !== 2}
            >
                <TypographyControls />
                <div class="field-block">
                    <label class="field">
                        <span class="field-label">{t('settings.density')}</span>
                        <select
                            class="field-select"
                            value={uiStore.readingDensity}
                            onchange={(event) =>
                                void uiStore.setReadingDensity(
                                    event.currentTarget.value as ReadingDensity,
                                )}
                        >
                            {#each DENSITY_OPTIONS as density (density)}
                                <option value={density}>{t(DENSITY_LABELS[density])}</option>
                            {/each}
                        </select>
                    </label>
                    <span class="toggle-hint">{t('settings.density.hint')}</span>
                </div>
                <label class="toggle">
                    <input
                        type="checkbox"
                        checked={uiStore.markOnScroll}
                        onchange={(event) => void uiStore.setMarkOnScroll(event.currentTarget.checked)}
                    />
                    <span class="toggle-text">
                        <span class="toggle-label">{t('settings.markOnScroll')}</span>
                        <span class="toggle-hint">{t('settings.markOnScroll.hint')}</span>
                    </span>
                </label>
                <label class="toggle">
                    <input
                        type="checkbox"
                        checked={uiStore.isHomeLayout('youtube')}
                        onchange={(event) =>
                            void uiStore.setHomeLayout('youtube', event.currentTarget.checked)}
                    />
                    <span class="toggle-text">
                        <span class="toggle-label">{t('settings.homeYoutube')}</span>
                        <span class="toggle-hint">{t('settings.homeYoutube.hint')}</span>
                    </span>
                </label>
                <label class="toggle">
                    <input
                        type="checkbox"
                        checked={uiStore.isHomeLayout('reddit')}
                        onchange={(event) =>
                            void uiStore.setHomeLayout('reddit', event.currentTarget.checked)}
                    />
                    <span class="toggle-text">
                        <span class="toggle-label">{t('settings.homeReddit')}</span>
                        <span class="toggle-hint">{t('settings.homeReddit.hint')}</span>
                    </span>
                </label>
            </div>

            <div
                role="tabpanel"
                id="settings-panel-media"
                aria-labelledby="settings-tab-media"
                tabindex="0"
                hidden={activeIndex !== 3}
            >
                <label class="toggle">
                    <input
                        type="checkbox"
                        checked={uiStore.mediaPrefetch}
                        onchange={(event) => void uiStore.setMediaPrefetch(event.currentTarget.checked)}
                    />
                    <span class="toggle-text">
                        <span class="toggle-label">{t('settings.media.prefetch')}</span>
                        <span class="toggle-hint">{t('settings.media.prefetch.hint')}</span>
                    </span>
                </label>
                <label class="toggle">
                    <input
                        type="checkbox"
                        checked={uiStore.allowRemoteFavicon}
                        onchange={(event) =>
                            void uiStore.setAllowRemoteFavicon(event.currentTarget.checked)}
                    />
                    <span class="toggle-text">
                        <span class="toggle-label">{t('settings.allowRemoteFavicon')}</span>
                        <span class="toggle-hint">{t('settings.allowRemoteFavicon.hint')}</span>
                    </span>
                </label>
            </div>

            <div
                role="tabpanel"
                id="settings-panel-data"
                aria-labelledby="settings-tab-data"
                tabindex="0"
                hidden={activeIndex !== 4}
            >
                <DestinationsManager />
                <OpmlPanel />
            </div>

            <div
                role="tabpanel"
                id="settings-panel-advanced"
                aria-labelledby="settings-tab-advanced"
                tabindex="0"
                hidden={activeIndex !== 5}
            >
                <DoctorPanel />
            </div>
        </div>
    </div>
</Modal>

<style>
    .settings-layout {
        display: flex;
        gap: var(--space-5);
    }

    .tablist {
        display: flex;
        flex-direction: column;
        gap: var(--space-1);
        flex: 0 0 auto;
        min-width: 9rem;
    }

    .tablist button[role='tab'] {
        text-align: left;
        padding: var(--space-2) var(--space-3);
        border-radius: var(--radius-md);
        font-size: var(--text-sm);
        color: var(--fg-muted);
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .tablist button[role='tab']:hover {
        background: var(--hover);
        color: var(--fg);
    }

    .tablist button[role='tab'][aria-selected='true'] {
        background: var(--selected);
        color: var(--accent);
        font-weight: 560;
    }

    .tablist button[role='tab']:focus-visible {
        outline: none;
        box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent), transparent 40%);
    }

    .panels {
        flex: 1 1 auto;
        min-width: 0;
    }

    [role='tabpanel'] {
        display: flex;
        flex-direction: column;
        gap: var(--space-4);
    }

    /* The author `display: flex` above outranks the UA `[hidden]` rule, so the
       hidden panels need an explicit, higher-specificity hide. */
    [role='tabpanel'][hidden] {
        display: none;
    }

    [role='tabpanel']:focus-visible {
        outline: none;
    }

    @media (max-width: 600px) {
        .settings-layout {
            flex-direction: column;
        }

        .tablist {
            flex-direction: row;
            flex-wrap: wrap;
        }
    }

    .field-block {
        display: flex;
        flex-direction: column;
        gap: var(--space-1);
    }

    .field {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: var(--space-3);
    }

    .field-label {
        font-size: var(--text-sm);
        color: var(--fg-muted);
    }

    .field-select {
        padding: var(--space-1) var(--space-2);
        border-radius: var(--radius-md);
        background: var(--bg-inset, transparent);
        color: var(--fg);
        border: 1px solid var(--hairline-strong);
        font-size: var(--text-sm);
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

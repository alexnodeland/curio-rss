<script lang="ts">
/**
 * The settings modal: appearance (the 9-theme picker at last wired in),
 * reading typography, destinations management, OPML import/export, and the
 * database doctor — each a self-contained section over the generated
 * commands. Keyboard dismissal is the shell's; sections are landmark
 * regions for screen readers.
 */
import Modal from '$components/common/Modal.svelte';
import ThemePicker from '$components/common/ThemePicker.svelte';
import DestinationsManager from '$components/modals/DestinationsManager.svelte';
import DoctorPanel from '$components/modals/DoctorPanel.svelte';
import OpmlPanel from '$components/modals/OpmlPanel.svelte';
import TypographyControls from '$components/reader/TypographyControls.svelte';
import { t } from '$lib/i18n';
import { uiStore } from '$lib/state/ui.svelte';

let { onclose }: { onclose: () => void } = $props();
</script>

<Modal title={t('settings.title')} {onclose} size="large">
    <section class="section" aria-labelledby="settings-appearance">
        <h3 id="settings-appearance">{t('settings.section.appearance')}</h3>
        <ThemePicker />
    </section>

    <section class="section" aria-labelledby="settings-reading">
        <h3 id="settings-reading">{t('settings.section.reading')}</h3>
        <TypographyControls />
    </section>

    <section class="section" aria-labelledby="settings-media">
        <h3 id="settings-media">{t('settings.section.media')}</h3>
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
    </section>

    <section class="section" aria-labelledby="settings-destinations">
        <h3 id="settings-destinations">{t('settings.section.destinations')}</h3>
        <DestinationsManager />
    </section>

    <section class="section" aria-labelledby="settings-data">
        <h3 id="settings-data">{t('settings.section.data')}</h3>
        <OpmlPanel />
    </section>

    <section class="section" aria-labelledby="settings-diagnostics">
        <h3 id="settings-diagnostics">{t('settings.section.diagnostics')}</h3>
        <DoctorPanel />
    </section>
</Modal>

<style>
    .section {
        display: flex;
        flex-direction: column;
        gap: var(--space-3);
    }

    .section h3 {
        font-size: 0.6875rem;
        font-weight: 650;
        letter-spacing: var(--tracking-caps);
        text-transform: uppercase;
        color: var(--fg-subtle);
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

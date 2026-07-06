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
        font-weight: 600;
        letter-spacing: 0.08em;
        text-transform: uppercase;
        color: var(--fg-subtle);
    }
</style>

<script lang="ts">
/**
 * The standalone destinations modal (the promote-flow entry point). It is a
 * thin wrapper around the shared `DestinationsManager` — the single
 * destinations implementation, also mounted inside Settings → Data. The
 * `Modal` shell owns the backdrop, focus trap, Escape dismissal, and close
 * button, so this file carries no duplicated overlay markup or CSS.
 */
import Modal from '$components/common/Modal.svelte';
import DestinationsManager from '$components/modals/DestinationsManager.svelte';
import { t } from '$lib/i18n';

let { onclose }: { onclose: () => void } = $props();
</script>

<Modal title={t('destinations.title')} {onclose}>
    <DestinationsManager />
</Modal>

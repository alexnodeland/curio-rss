<script lang="ts">
/**
 * The reader shell: the three-pane frame, the global keyboard wiring
 * (registry matcher → action layer), the help overlay, and the toast
 * outlet. Shortcuts are inert in typing contexts and while a modal owns
 * the keyboard (Escape / `?` dismiss the help overlay).
 */
import RefreshStatus from '$components/common/RefreshStatus.svelte';
import Toasts from '$components/common/Toasts.svelte';
import ListPane from '$components/layout/ListPane.svelte';
import ThreePane from '$components/layout/ThreePane.svelte';
import AddFeedModal from '$components/modals/AddFeedModal.svelte';
import DestinationsPanel from '$components/modals/DestinationsPanel.svelte';
import FeedHealthPanel from '$components/modals/FeedHealthPanel.svelte';
import HelpOverlay from '$components/modals/HelpOverlay.svelte';
import SettingsModal from '$components/modals/SettingsModal.svelte';
import ReaderPane from '$components/reader/ReaderPane.svelte';
import Sidebar from '$components/sidebar/Sidebar.svelte';
import { createMatcher, shouldIgnoreKeyEvent } from '$lib/keyboard/registry';
import { handleShortcut } from '$lib/state/actions';
import { uiStore } from '$lib/state/ui.svelte';

const matcher = createMatcher();

function onKeydown(event: KeyboardEvent): void {
    if (shouldIgnoreKeyEvent(event)) {
        return;
    }
    if (uiStore.activeModal !== null) {
        // A modal owns the keyboard; Escape (and `?` for help) dismisses.
        const dismissesHelp = uiStore.activeModal === 'help' && event.key === '?';
        if (event.key === 'Escape' || dismissesHelp) {
            event.preventDefault();
            uiStore.closeModal();
        }
        matcher.reset();
        return;
    }
    const result = matcher.handle(event.key);
    if (result.kind === 'none') {
        return;
    }
    event.preventDefault();
    if (result.kind === 'match') {
        handleShortcut(result.id);
    }
}
</script>

<svelte:window onkeydown={onKeydown} />

<ThreePane>
    {#snippet sidebar()}
        <Sidebar />
    {/snippet}
    {#snippet list()}
        <ListPane />
    {/snippet}
    {#snippet reader()}
        <ReaderPane />
    {/snippet}
</ThreePane>

{#if uiStore.activeModal === 'help'}
    <HelpOverlay onclose={() => uiStore.closeModal()} />
{/if}

{#if uiStore.activeModal === 'destinations'}
    <DestinationsPanel onclose={() => uiStore.closeModal()} />
{/if}

{#if uiStore.activeModal === 'add-feed'}
    <AddFeedModal onclose={() => uiStore.closeModal()} />
{/if}

{#if uiStore.activeModal === 'settings'}
    <SettingsModal onclose={() => uiStore.closeModal()} />
{/if}

{#if uiStore.activeModal === 'feed-health' && uiStore.healthFeedId !== null}
    <FeedHealthPanel feedId={uiStore.healthFeedId} onclose={() => uiStore.closeModal()} />
{/if}

<RefreshStatus />

<Toasts />

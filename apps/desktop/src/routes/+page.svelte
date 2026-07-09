<script lang="ts">
/**
 * The reader shell: the three-pane frame, the global keyboard wiring
 * (registry matcher → action layer), the help overlay, and the toast
 * outlet. Shortcuts are inert in typing contexts and while a modal owns
 * the keyboard (Escape / `?` dismiss the help overlay).
 */
import MenuHost from '$components/common/MenuHost.svelte';
import RefreshStatus from '$components/common/RefreshStatus.svelte';
import Toasts from '$components/common/Toasts.svelte';
import TooltipHost from '$components/common/TooltipHost.svelte';
import ListPane from '$components/layout/ListPane.svelte';
import ThreePane from '$components/layout/ThreePane.svelte';
import AddFeedModal from '$components/modals/AddFeedModal.svelte';
import DestinationsPanel from '$components/modals/DestinationsPanel.svelte';
import EditFeedModal from '$components/modals/EditFeedModal.svelte';
import HelpOverlay from '$components/modals/HelpOverlay.svelte';
import SettingsModal from '$components/modals/SettingsModal.svelte';
import ReaderPane from '$components/reader/ReaderPane.svelte';
import Sidebar from '$components/sidebar/Sidebar.svelte';
import { t } from '$lib/i18n';
import { createMatcher, shouldIgnoreKeyEvent } from '$lib/keyboard/registry';
import { handleShortcut } from '$lib/state/actions';
import { menuStore } from '$lib/state/menu.svelte';
import { searchStore } from '$lib/state/search.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { uiStore } from '$lib/state/ui.svelte';

const matcher = createMatcher();

// While a modal owns the screen, the whole app frame behind it is `inert`
// — removed from the tab order and hidden from assistive tech — so the
// modal's focus trap has nothing to leak to.
const backgroundInert = $derived(uiStore.activeModal !== null);

function onKeydown(event: KeyboardEvent): void {
    // An open menu owns the keyboard entirely (its own level handles Escape,
    // arrows, activation); never let app shortcuts fire underneath it.
    if (menuStore.isOpen) {
        return;
    }
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
    if (selectionStore.focus === 'sidebar') {
        // The sidebar tree owns the keyboard: it handles its own navigation
        // keys (which stopPropagation), so anything reaching here is a stray
        // that must not fire an article-pane shortcut underneath the tree.
        matcher.reset();
        return;
    }
    if (event.key === 'Escape') {
        // Menu + modal Escape are handled above; here Escape steps back through
        // the shell so it is never a no-op — clear an active search, else
        // deselect the open article.
        if (searchStore.active) {
            event.preventDefault();
            searchStore.clear();
        } else if (selectionStore.selectedArticleId !== null) {
            event.preventDefault();
            selectionStore.selectedArticleId = null;
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

<a class="skip-link" href="#main-content">{t('a11y.skipToContent')}</a>

<main id="main-content" class="app-main" tabindex="-1" inert={backgroundInert}>
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
</main>

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

{#if uiStore.activeModal === 'edit-feed' && uiStore.editFeedId !== null}
    <EditFeedModal
        feedId={uiStore.editFeedId}
        section={uiStore.editFeedSection}
        onclose={() => uiStore.closeModal()}
    />
{/if}

<RefreshStatus />

<Toasts />

<MenuHost />

<TooltipHost />

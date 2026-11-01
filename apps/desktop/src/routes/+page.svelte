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
import {
    createMatcher,
    eventKeyName,
    paneShortcut,
    shouldIgnoreKeyEvent,
} from '$lib/keyboard/registry';
import { handleShortcut } from '$lib/state/actions';
import { menuStore } from '$lib/state/menu.svelte';
import { searchStore } from '$lib/state/search.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { layoutStore } from '$lib/state/layout.svelte';

const matcher = createMatcher();

// While a modal owns the screen, the whole app frame behind it is `inert`
// — removed from the tab order and hidden from assistive tech — so the
// modal's focus trap has nothing to leak to.
const backgroundInert = $derived(layoutStore.activeModal !== null);

// A modal owns the keyboard; Escape (and `?` over help) dismisses it. Returns
// true when the event belonged to the modal layer (handled or swallowed).
function handleModalKey(event: KeyboardEvent): boolean {
    if (layoutStore.activeModal === null) {
        return false;
    }
    const dismissesHelp = layoutStore.activeModal === 'help' && event.key === '?';
    if (event.key === 'Escape' || dismissesHelp) {
        event.preventDefault();
        layoutStore.closeModal();
    }
    matcher.reset();
    return true;
}

// Menu + modal Escape are handled earlier; here Escape steps back through the
// shell so it is never a no-op — clear an active search, else deselect the open
// article. Returns true when the event was an Escape (handled).
function handleEscapeStepback(event: KeyboardEvent): boolean {
    if (event.key !== 'Escape') {
        return false;
    }
    if (searchStore.active) {
        event.preventDefault();
        searchStore.clear();
    } else if (selectionStore.selectedArticleId !== null) {
        event.preventDefault();
        selectionStore.selectedArticleId = null;
    }
    matcher.reset();
    return true;
}

/**
 * Whether nothing in particular holds focus — `<body>` (app launch, or the
 * focused element was unmounted) or the skip-link's `<main>` target. The
 * article list is the default pane, so its keys (↑/↓, Enter/→) work from
 * here too: the reader's empty-state hint is true from the first keystroke.
 */
function isUnfocused(target: EventTarget | null): boolean {
    return (
        target === document.body ||
        target === document.documentElement ||
        (target instanceof HTMLElement && target.id === 'main-content')
    );
}

function onKeydown(event: KeyboardEvent): void {
    // An open menu owns the keyboard entirely (its own level handles Escape,
    // arrows, activation); never let app shortcuts fire underneath it.
    if (menuStore.isOpen) {
        return;
    }
    // A modal owns the keyboard *before* the typing guard, so Escape closes it
    // even while one of its fields is focused (the guard used to swallow the
    // Escape first). Non-Escape keys are swallowed too, but not preventDefault'd,
    // so typing into the modal's inputs still works.
    if (handleModalKey(event)) {
        return;
    }
    if (shouldIgnoreKeyEvent(event)) {
        return;
    }
    if (selectionStore.focus === 'sidebar') {
        // The sidebar tree owns the keyboard: it handles its own navigation
        // keys (which stopPropagation), so anything reaching here is a stray
        // that must not fire an article-pane shortcut underneath the tree.
        matcher.reset();
        return;
    }
    if (handleEscapeStepback(event)) {
        return;
    }
    const key = eventKeyName(event);
    const paneId = isUnfocused(event.target) ? paneShortcut('list', key) : null;
    if (paneId !== null) {
        event.preventDefault();
        matcher.reset();
        handleShortcut(paneId);
        return;
    }
    const result = matcher.handle(key);
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

{#if layoutStore.activeModal === 'help'}
    <HelpOverlay onclose={() => layoutStore.closeModal()} />
{/if}

{#if layoutStore.activeModal === 'destinations'}
    <DestinationsPanel onclose={() => layoutStore.closeModal()} />
{/if}

{#if layoutStore.activeModal === 'add-feed'}
    <AddFeedModal onclose={() => layoutStore.closeModal()} />
{/if}

{#if layoutStore.activeModal === 'settings'}
    <SettingsModal onclose={() => layoutStore.closeModal()} />
{/if}

{#if layoutStore.activeModal === 'edit-feed' && layoutStore.editFeedId !== null}
    <EditFeedModal
        feedId={layoutStore.editFeedId}
        section={layoutStore.editFeedSection}
        onclose={() => layoutStore.closeModal()}
    />
{/if}

<RefreshStatus />

<Toasts />

<MenuHost />

<TooltipHost />

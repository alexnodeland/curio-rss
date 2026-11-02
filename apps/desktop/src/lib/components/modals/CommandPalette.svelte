<script lang="ts">
/**
 * The ⌘K command palette: one field that fuzzy-finds anything the app can do
 * — commands (with the keys that run them), views, feeds, folders, settings
 * tabs and themes — and runs it on Enter or a click. The catalogue and the
 * ranking live in `$lib/state/palette`; this is the WAI-ARIA combobox over
 * them: the field keeps DOM focus while ↑/↓ move `aria-activedescendant`
 * through the listbox, so typing never stops.
 *
 * It is a shell modal (`activeModal === 'palette'`): the app frame behind it
 * is inert, Escape is the shell's, and `trapFocus` hands focus back to
 * whatever held it on close. A chosen item runs only after the palette has
 * unmounted and that restore has happened, so an item that moves focus
 * itself (search, the feed tree) keeps it.
 */
import Icon, { type IconName } from '$components/common/Icon.svelte';
import { trapFocus } from '$lib/actions/trap-focus';
import { t } from '$lib/i18n';
import { keyLabel } from '$lib/keyboard/labels';
import {
    PALETTE_GROUP_LABELS,
    type PaletteGroup,
    highlightRuns,
    paletteItems,
    rankPaletteItems,
} from '$lib/state/palette';
import { tick } from 'svelte';

let { onclose }: { onclose: () => void } = $props();

const GROUP_ICONS: Readonly<Record<PaletteGroup, IconName>> = {
    command: 'sparkle',
    view: 'inbox',
    feed: 'rss',
    folder: 'folder',
    settings: 'settings',
    theme: 'circle-dot',
};

/** Rows a PageUp / PageDown moves the active row by. */
const PAGE_ROWS = 8;

let query = $state('');
let activeIndex = $state(0);
let input: HTMLInputElement | undefined = $state();
let results: HTMLUListElement | undefined = $state();

const items = $derived(paletteItems());
const ranked = $derived(rankPaletteItems(items, query));

/** The DOM id of the option at `index` (the field's active descendant). */
function optionId(index: number): string {
    return `palette-option-${index}`;
}

const activeId = $derived(ranked.length > 0 ? optionId(activeIndex) : undefined);

// The active row stays in view as the keys move it (jsdom has no layout, and
// no scrollIntoView, hence the guard).
$effect(() => {
    const index = activeIndex;
    const option = results?.querySelector<HTMLElement>(`#${optionId(index)}`);
    option?.scrollIntoView?.({ block: 'nearest' });
});

// `trapFocus` seats focus on the dialog as it mounts; the field takes it
// right after, so typing starts at once.
$effect(() => {
    void tick().then(() => input?.focus());
});

/** Moves the active row by `delta`, wrapping at either end for single steps. */
function move(delta: number): void {
    const count = ranked.length;
    if (count === 0) {
        return;
    }
    if (Math.abs(delta) === 1) {
        activeIndex = (activeIndex + delta + count) % count;
    } else {
        activeIndex = Math.min(Math.max(activeIndex + delta, 0), count - 1);
    }
}

/** Closes the palette, lets focus return home, then runs the row's item. */
async function runAt(index: number): Promise<void> {
    const entry = ranked[index];
    if (entry === undefined) {
        return;
    }
    onclose();
    await tick();
    entry.item.run();
}

function onKeydown(event: KeyboardEvent): void {
    switch (event.key) {
        case 'ArrowDown':
            event.preventDefault();
            move(1);
            break;
        case 'ArrowUp':
            event.preventDefault();
            move(-1);
            break;
        case 'PageDown':
            event.preventDefault();
            move(PAGE_ROWS);
            break;
        case 'PageUp':
            event.preventDefault();
            move(-PAGE_ROWS);
            break;
        case 'Enter':
            event.preventDefault();
            void runAt(activeIndex);
            break;
        default:
            break;
    }
}

// Only a press that starts and ends on the backdrop dismisses (a drag out of
// the field must not), as in the shared Modal shell.
let pressedBackdrop = false;

function onBackdropPointerDown(event: PointerEvent): void {
    pressedBackdrop = event.target === event.currentTarget;
}

function onBackdropClick(event: MouseEvent): void {
    if (pressedBackdrop && event.target === event.currentTarget) {
        onclose();
    }
    pressedBackdrop = false;
}
</script>

<!-- The backdrop click is a pointer convenience on top of Escape (the shell's);
     the dialog keeps the focus trap, so it is not the only way out. -->
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="palette-backdrop" onpointerdown={onBackdropPointerDown} onclick={onBackdropClick}>
    <div
        class="palette"
        role="dialog"
        aria-modal="true"
        aria-label={t('palette.title')}
        tabindex="-1"
        data-testid="command-palette"
        use:trapFocus
    >
        <div class="palette-field">
            <Icon name="search" size={16} />
            <input
                bind:this={input}
                bind:value={query}
                type="text"
                role="combobox"
                aria-label={t('palette.title')}
                aria-expanded={ranked.length > 0}
                aria-controls="palette-results"
                aria-activedescendant={activeId}
                aria-autocomplete="list"
                autocomplete="off"
                spellcheck="false"
                placeholder={t('palette.placeholder')}
                oninput={() => (activeIndex = 0)}
                onkeydown={onKeydown}
            />
        </div>
        {#if ranked.length > 0}
            <ul
                id="palette-results"
                class="palette-results"
                role="listbox"
                aria-label={t('palette.results')}
                bind:this={results}
            >
                {#each ranked as entry, index (entry.item.id)}
                    <li
                        id={optionId(index)}
                        class="palette-option"
                        class:active={index === activeIndex}
                        role="option"
                        aria-selected={index === activeIndex}
                        tabindex="-1"
                        data-item={entry.item.id}
                        onpointermove={() => (activeIndex = index)}
                        onclick={() => void runAt(index)}
                    >
                        <span class="option-icon" aria-hidden="true">
                            <Icon name={GROUP_ICONS[entry.item.group]} size={15} />
                        </span>
                        <span class="option-text">
                            <span class="option-label truncate"
                                >{#each highlightRuns(entry.item.label, entry.indices) as run, position (position)}{#if run.match}<mark
                                            >{run.text}</mark
                                        >{:else}{run.text}{/if}{/each}</span
                            >
                            {#if entry.item.detail !== undefined}
                                <span class="option-detail truncate">{entry.item.detail}</span>
                            {/if}
                        </span>
                        {#if entry.item.current === true}
                            <span class="option-current">
                                <Icon name="check" size={14} />
                                <span class="sr-only">{t('palette.current')}</span>
                            </span>
                        {/if}
                        <span class="option-group">{t(PALETTE_GROUP_LABELS[entry.item.group])}</span>
                        {#if entry.item.kbd !== undefined}
                            <kbd>{entry.item.kbd}</kbd>
                        {/if}
                    </li>
                {/each}
            </ul>
        {:else}
            <p class="palette-empty">{t('palette.empty')}</p>
        {/if}
        <!-- The field is announced as a combobox with a listbox; the count
             tells a screen-reader user how many rows the query left. -->
        <span class="sr-only" aria-live="polite">{t('palette.resultCount', { count: ranked.length })}</span>
        <footer class="palette-footer" aria-hidden="true">
            <span><kbd>{keyLabel('ArrowUp')}</kbd><kbd>{keyLabel('ArrowDown')}</kbd> {t('palette.hint.move')}</span>
            <span><kbd>↵</kbd> {t('palette.hint.run')}</span>
            <span><kbd>{keyLabel('Escape')}</kbd> {t('palette.hint.close')}</span>
        </footer>
    </div>
</div>

<style>
    .palette-backdrop {
        position: fixed;
        inset: 0;
        display: flex;
        align-items: flex-start;
        justify-content: center;
        padding: max(var(--space-8), 12vh) var(--space-4) var(--space-4);
        background: color-mix(in srgb, #000, transparent 55%);
        backdrop-filter: blur(2px);
        z-index: 100;
        animation: palette-fade var(--dur-fast) var(--ease);
    }

    .palette {
        width: min(600px, 100%);
        max-height: min(70vh, 560px);
        display: flex;
        flex-direction: column;
        background: var(--surface-overlay);
        border: 1px solid var(--hairline);
        border-radius: var(--radius-xl);
        box-shadow: var(--shadow-lg);
        overflow: hidden;
        animation: palette-in var(--dur-base) var(--ease);
    }

    .palette:focus-visible {
        outline: none;
    }

    @keyframes palette-fade {
        from {
            opacity: 0;
        }
    }

    @keyframes palette-in {
        from {
            opacity: 0;
            transform: translateY(-6px) scale(0.99);
        }
    }

    .palette-field {
        flex: 0 0 auto;
        display: flex;
        align-items: center;
        gap: var(--space-3);
        padding: 0 var(--space-4);
        height: 52px;
        border-bottom: 1px solid var(--hairline);
        color: var(--fg-subtle);
    }

    .palette-field input {
        flex: 1 1 auto;
        min-width: 0;
        height: 100%;
        border: 0;
        background: transparent;
        color: var(--fg);
        font-size: var(--text-lg);
        outline: none;
    }

    .palette-field input::placeholder {
        color: var(--fg-subtle);
    }

    .palette-results {
        flex: 1 1 auto;
        min-height: 0;
        overflow-y: auto;
        list-style: none;
        padding: var(--space-2);
        overscroll-behavior: contain;
    }

    .palette-option {
        display: flex;
        align-items: center;
        gap: var(--space-3);
        min-height: 40px;
        padding: var(--space-1) var(--space-3);
        border-radius: var(--radius-md);
        color: var(--fg);
        cursor: pointer;
        -webkit-user-select: none;
        user-select: none;
    }

    .palette-option.active {
        background: var(--selected);
    }

    .option-icon {
        flex: 0 0 auto;
        display: inline-grid;
        place-items: center;
        width: 26px;
        height: 26px;
        border-radius: var(--radius-sm);
        background: var(--surface-raised);
        border: 1px solid var(--hairline);
        color: var(--fg-muted);
    }

    .palette-option.active .option-icon {
        color: var(--accent);
    }

    .option-text {
        flex: 1 1 auto;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 1px;
    }

    .option-label {
        font-size: var(--text-md);
    }

    .option-label mark {
        background: transparent;
        color: var(--accent);
        font-weight: var(--weight-semibold);
    }

    .option-detail {
        font-size: var(--text-xs);
        color: var(--fg-subtle);
    }

    .option-current {
        flex: 0 0 auto;
        display: inline-grid;
        place-items: center;
        color: var(--accent);
    }

    .option-group {
        flex: 0 0 auto;
        font-size: var(--text-xs);
        color: var(--fg-subtle);
    }

    kbd {
        flex: 0 0 auto;
        padding: 0.1em 0.45em;
        border: 1px solid var(--hairline);
        border-radius: var(--radius-sm);
        background: var(--surface-raised);
        box-shadow: 0 1px 0 var(--hairline-strong);
        font-family: var(--font-mono);
        font-size: 0.75rem;
        color: var(--fg-muted);
        white-space: nowrap;
    }

    .palette-empty {
        padding: var(--space-6) var(--space-4);
        text-align: center;
        font-size: var(--text-md);
        color: var(--fg-subtle);
    }

    .palette-footer {
        flex: 0 0 auto;
        display: flex;
        gap: var(--space-4);
        padding: var(--space-2) var(--space-4);
        border-top: 1px solid var(--hairline);
        font-size: var(--text-xs);
        color: var(--fg-subtle);
    }

    .palette-footer span {
        display: inline-flex;
        align-items: center;
        gap: var(--space-1);
    }

    @media (prefers-reduced-motion: reduce) {
        .palette-backdrop,
        .palette {
            animation: none;
        }
    }
</style>

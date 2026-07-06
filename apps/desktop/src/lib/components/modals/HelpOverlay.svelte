<script lang="ts">
/**
 * The `?` overlay: renders the shortcut registry verbatim — the table is
 * data, so this component can never drift from what the keys actually do.
 * Dismissal (Escape / `?`) is handled by the shell's keydown handler; the
 * close button covers the pointer path.
 */
import { type MessageKey, t } from '$lib/i18n';
import { SHORTCUTS, type ShortcutCategory } from '$lib/keyboard/registry';

let { onclose }: { onclose: () => void } = $props();

const CATEGORY_ORDER: readonly ShortcutCategory[] = ['navigation', 'actions', 'views', 'app'];

const CATEGORY_LABELS: Record<ShortcutCategory, MessageKey> = {
    navigation: 'shortcut.category.navigation',
    actions: 'shortcut.category.actions',
    views: 'shortcut.category.views',
    app: 'shortcut.category.app',
};

function shortcutsIn(category: ShortcutCategory) {
    return SHORTCUTS.filter((shortcut) => shortcut.category === category);
}

let dialog: HTMLElement | undefined = $state();

$effect(() => {
    dialog?.focus();
});
</script>

<div class="overlay-backdrop">
    <div
        class="overlay"
        role="dialog"
        aria-modal="true"
        aria-labelledby="help-title"
        tabindex="-1"
        bind:this={dialog}
    >
        <header class="overlay-header">
            <h2 id="help-title">{t('help.title')}</h2>
            <button class="overlay-close" onclick={onclose} aria-label={t('help.close')}>×</button>
        </header>
        <div class="overlay-body">
            {#each CATEGORY_ORDER as category (category)}
                <section class="shortcut-group">
                    <h3>{t(CATEGORY_LABELS[category])}</h3>
                    <ul>
                        {#each shortcutsIn(category) as shortcut (shortcut.id)}
                            <li>
                                <span class="keys">
                                    {#each shortcut.keys as keyName, position (position)}
                                        {#if position > 0}
                                            <span class="chord-sep">{t('help.chord.then')}</span>
                                        {/if}
                                        <kbd>{keyName === ' ' ? 'Space' : keyName}</kbd>
                                    {/each}
                                </span>
                                <span class="description">{t(shortcut.description)}</span>
                            </li>
                        {/each}
                    </ul>
                </section>
            {/each}
        </div>
    </div>
</div>

<style>
    .overlay-backdrop {
        position: fixed;
        inset: 0;
        display: flex;
        align-items: center;
        justify-content: center;
        background: rgb(0 0 0 / 45%);
        z-index: 100;
    }

    .overlay {
        width: min(560px, calc(100vw - var(--space-8)));
        max-height: min(70vh, 640px);
        display: flex;
        flex-direction: column;
        background: var(--bg-secondary);
        border: 1px solid var(--border);
        border-radius: var(--radius-lg);
        box-shadow: var(--shadow-lg);
    }

    .overlay:focus-visible {
        outline: 2px solid var(--accent);
        outline-offset: -2px;
    }

    .overlay-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: var(--space-4) var(--space-5);
        border-bottom: 1px solid var(--border-subtle);
    }

    .overlay-header h2 {
        font-size: 1rem;
    }

    .overlay-close {
        padding: 0 var(--space-2);
        font-size: 1.25rem;
        line-height: 1;
        color: var(--fg-muted);
        background: transparent;
        border-radius: var(--radius-sm);
    }

    .overlay-close:hover {
        color: var(--fg);
        background: var(--bg-hover);
    }

    .overlay-body {
        overflow-y: auto;
        padding: var(--space-4) var(--space-5) var(--space-5);
        display: flex;
        flex-direction: column;
        gap: var(--space-5);
    }

    .shortcut-group h3 {
        font-size: 0.6875rem;
        font-weight: 600;
        letter-spacing: 0.08em;
        text-transform: uppercase;
        color: var(--fg-subtle);
        margin-bottom: var(--space-2);
    }

    .shortcut-group ul {
        list-style: none;
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
    }

    .shortcut-group li {
        display: flex;
        align-items: center;
        gap: var(--space-4);
    }

    .keys {
        flex: 0 0 7rem;
        display: flex;
        align-items: center;
        gap: var(--space-1);
    }

    kbd {
        padding: 1px var(--space-2);
        border: 1px solid var(--border);
        border-bottom-width: 2px;
        border-radius: var(--radius-sm);
        background: var(--bg-tertiary);
        font-family: var(--font-mono);
        font-size: 0.75rem;
    }

    .chord-sep {
        font-size: 0.6875rem;
        color: var(--fg-subtle);
    }

    .description {
        font-size: 0.875rem;
        color: var(--fg);
    }
</style>

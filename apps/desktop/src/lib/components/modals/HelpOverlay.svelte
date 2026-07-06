<script lang="ts">
/**
 * The `?` overlay: renders the shortcut registry verbatim — the table is
 * data, so this component can never drift from what the keys actually do.
 * Dismissal (Escape / `?`) is handled by the shell's keydown handler; the
 * close button covers the pointer path.
 */
import Icon from '$components/common/Icon.svelte';
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
            <button class="overlay-close" onclick={onclose} aria-label={t('help.close')}
                ><Icon name="close" size={16} /></button
            >
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
        padding: var(--space-6);
        background: color-mix(in srgb, #000, transparent 45%);
        backdrop-filter: blur(3px);
        z-index: 100;
        animation: backdrop-in var(--dur-base) var(--ease);
    }

    .overlay {
        width: min(560px, calc(100vw - var(--space-8)));
        max-height: min(70vh, 640px);
        display: flex;
        flex-direction: column;
        background: var(--surface-overlay);
        border: 1px solid var(--hairline);
        border-radius: var(--radius-xl);
        box-shadow: var(--shadow-lg);
        animation: overlay-in var(--dur-base) var(--ease);
    }

    .overlay:focus-visible {
        outline: none;
    }

    @keyframes backdrop-in {
        from {
            opacity: 0;
        }
    }

    @keyframes overlay-in {
        from {
            opacity: 0;
            transform: translateY(8px) scale(0.99);
        }
    }

    .overlay-header {
        flex: 0 0 auto;
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: var(--space-4) var(--space-4) var(--space-4) var(--space-6);
        border-bottom: 1px solid var(--hairline);
    }

    .overlay-header h2 {
        font-size: var(--text-lg);
        font-weight: 640;
        letter-spacing: var(--tracking-snug);
    }

    .overlay-close {
        display: inline-grid;
        place-items: center;
        width: 32px;
        height: 32px;
        color: var(--fg-muted);
        background: transparent;
        border-radius: var(--radius-md);
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .overlay-close:hover {
        color: var(--fg);
        background: var(--hover);
    }

    .overlay-body {
        overflow-y: auto;
        padding: var(--space-6);
        display: flex;
        flex-direction: column;
        gap: var(--space-6);
    }

    .shortcut-group h3 {
        font-size: 0.6875rem;
        font-weight: 650;
        letter-spacing: var(--tracking-caps);
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
        padding: 0.15em 0.5em;
        border: 1px solid var(--hairline);
        border-radius: var(--radius-sm);
        background: var(--surface-raised);
        box-shadow: 0 1px 0 var(--hairline-strong);
        font-family: var(--font-mono);
        font-size: 0.82em;
        color: var(--fg-muted);
    }

    .chord-sep {
        font-size: var(--text-xs);
        color: var(--fg-subtle);
    }

    .description {
        font-size: var(--text-md);
        color: var(--fg);
    }
</style>

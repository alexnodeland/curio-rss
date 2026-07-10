<script lang="ts">
/**
 * The `?` overlay: renders the shortcut registry verbatim — the table is
 * data, so this component can never drift from what the keys actually do.
 * It rides the shared {@link Modal} shell (focus trap, restore, labelled
 * dialog); dismissal (Escape / `?`) is handled by the shell's keydown
 * handler, and the close button covers the pointer path.
 */
import Modal from '$components/common/Modal.svelte';
import { type MessageKey, t } from '$lib/i18n';
import { NAV_HINTS, SHORTCUTS, type ShortcutCategory } from '$lib/keyboard/registry';

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
</script>

<Modal title={t('help.title')} {onclose}>
    {#each CATEGORY_ORDER as category (category)}
        <section class="shortcut-group">
            <h3>{t(CATEGORY_LABELS[category])}</h3>
            <ul>
                {#if category === 'navigation'}
                    <!-- Arrow-key spatial nav is documented here even though it's
                         handled outside the shortcut matcher (see NAV_HINTS). -->
                    {#each NAV_HINTS as hint (hint.description)}
                        <li>
                            <span class="keys">
                                {#each hint.keys as keyName, position (position)}
                                    {#if position > 0}
                                        <span class="chord-sep">/</span>
                                    {/if}
                                    <kbd>{keyName}</kbd>
                                {/each}
                            </span>
                            <span class="description">{t(hint.description)}</span>
                        </li>
                    {/each}
                {/if}
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
    <p class="help-footer">{t('help.footer')}</p>
</Modal>

<style>
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

    .help-footer {
        margin-top: var(--space-4);
        padding-top: var(--space-3);
        border-top: 1px solid var(--hairline);
        font-size: var(--text-sm);
        color: var(--fg-subtle);
        line-height: 1.5;
    }
</style>

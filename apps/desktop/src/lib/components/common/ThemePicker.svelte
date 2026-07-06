<script lang="ts">
/**
 * The theme picker: the 9 CSS `[data-theme]` themes plus a `system` mode.
 * Each swatch previews its own colors by scoping `data-theme` locally, so
 * the preview reads the same CSS custom properties the app would apply.
 * Selection writes through `uiStore.setThemePreference` (settings table +
 * the localStorage FOUC mirror).
 */
import { t } from '$lib/i18n';
import { THEMES, type ThemePreference, uiStore } from '$lib/state/ui.svelte';

const OPTIONS: readonly { id: ThemePreference; name: string }[] = [
    { id: 'system', name: t('theme.system') },
    ...THEMES.map((theme) => ({ id: theme.id, name: theme.name })),
];

function pick(id: ThemePreference): void {
    void uiStore.setThemePreference(id);
}
</script>

<div class="theme-grid" role="group" aria-label={t('settings.theme')}>
    {#each OPTIONS as option (option.id)}
        {@const active = uiStore.themePreference === option.id}
        <button
            class="swatch"
            class:active
            type="button"
            aria-pressed={active}
            aria-label={t('theme.select', { name: option.name })}
            onclick={() => pick(option.id)}
        >
            <span class="preview" data-theme={option.id === 'system' ? null : option.id}>
                <span class="chip chip-bg"></span>
                <span class="chip chip-accent"></span>
                <span class="chip chip-fg"></span>
            </span>
            <span class="name">{option.name}</span>
        </button>
    {/each}
</div>

<style>
    .theme-grid {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(120px, 1fr));
        gap: var(--space-3);
    }

    .swatch {
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
        padding: var(--space-2);
        border: 1px solid var(--border);
        border-radius: var(--radius-md);
        background: var(--bg);
        text-align: left;
    }

    .swatch:hover {
        border-color: var(--fg-subtle);
    }

    .swatch.active {
        border-color: var(--accent);
        box-shadow: 0 0 0 1px var(--accent);
    }

    .preview {
        display: flex;
        gap: 4px;
        padding: var(--space-2);
        border-radius: var(--radius-sm);
        background: var(--bg);
    }

    .chip {
        width: 1.25rem;
        height: 1.25rem;
        border-radius: 50%;
        border: 1px solid var(--border-subtle);
    }

    .chip-bg {
        background: var(--bg-secondary);
    }

    .chip-accent {
        background: var(--accent);
    }

    .chip-fg {
        background: var(--fg);
    }

    .name {
        font-size: 0.8125rem;
        color: var(--fg);
    }
</style>

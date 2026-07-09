<script lang="ts">
/**
 * The theme picker: the 9 built-in `[data-theme]` themes plus a `system` mode,
 * and a Custom group for user-authored themes. Each swatch previews its own
 * colors by scoping `data-theme` locally (custom swatches resolve against the
 * injected `[data-theme="custom-<id>"]` rule). Selection writes through
 * `uiStore.setThemePreference`. Import/export go through the tauri-specta
 * commands with the same PathToken discipline OPML uses — a raw path never
 * crosses IPC. Imported values are validated (see `theme-format`) before they
 * ever become a stylesheet rule.
 */
import { commands } from '$lib/bindings';
import { tooltip } from '$lib/actions/tooltip';
import { t } from '$lib/i18n';
import { toastCommandError } from '$lib/state/actions';
import { type MenuItem, menuStore } from '$lib/state/menu.svelte';
import { THEMES, type CustomThemeId, type ThemePreference, uiStore } from '$lib/state/ui.svelte';
import {
    type CustomTheme,
    contrastFailures,
    parseTheme,
    serializeTheme,
} from '$lib/utils/theme-format';
import { readComputedTokens } from '$lib/utils/theme-dom';

const OPTIONS: readonly { id: ThemePreference; name: string }[] = [
    { id: 'system', name: t('theme.system') },
    ...THEMES.map((theme) => ({ id: theme.id, name: theme.name })),
];

function pick(id: ThemePreference): void {
    void uiStore.setThemePreference(id);
}

function customId(theme: CustomTheme): CustomThemeId {
    return `custom-${theme.id}`;
}

/** Metadata + live token values of the currently applied theme. */
function activeThemeExport(): CustomTheme {
    const resolved = uiStore.resolvedTheme;
    const custom = uiStore.customThemes.find((theme) => customId(theme) === resolved);
    const tokens = readComputedTokens(document.documentElement);
    if (custom !== undefined) {
        return { ...custom, tokens };
    }
    const builtin = THEMES.find((theme) => theme.id === resolved);
    return {
        id: resolved,
        name: builtin?.name ?? resolved,
        isDark: builtin?.isDark ?? true,
        tokens,
    };
}

async function writeThemeFile(theme: CustomTheme): Promise<void> {
    const yaml = serializeTheme(theme);
    try {
        const picked = await commands.pickThemeExportPath();
        if (picked.status === 'error') {
            toastCommandError(picked.error);
            return;
        }
        if (picked.data === null) {
            uiStore.showToast(t('opml.cancelled'), 'info');
            return;
        }
        const result = await commands.exportTextFile(picked.data.token, yaml);
        if (result.status === 'error') {
            toastCommandError(result.error);
            return;
        }
        uiStore.showToast(t('theme.exported'), 'success');
    } catch {
        uiStore.showToast(t('app.error.internal'), 'error');
    }
}

function exportCurrent(): void {
    void writeThemeFile(activeThemeExport());
}

function uniqueThemeId(base: string): string {
    let n = 2;
    let candidate = `${base}-${n}`;
    while (uiStore.hasCustomTheme(candidate)) {
        n += 1;
        candidate = `${base}-${n}`;
    }
    return candidate;
}

async function importTheme(): Promise<void> {
    try {
        const picked = await commands.pickThemeImportFile();
        if (picked.status === 'error') {
            toastCommandError(picked.error);
            return;
        }
        if (picked.data === null) {
            uiStore.showToast(t('opml.cancelled'), 'info');
            return;
        }
        const read = await commands.readTextFile(picked.data.token);
        if (read.status === 'error') {
            toastCommandError(read.error);
            return;
        }
        const parsed = parseTheme(read.data);
        if (!parsed.ok) {
            uiStore.showToast(t('theme.importFailed', { error: parsed.error }), 'error', 6000);
            return;
        }
        let theme = parsed.theme;
        if (uiStore.hasCustomTheme(theme.id)) {
            const id = uniqueThemeId(theme.id);
            theme = { ...theme, id };
            uiStore.showToast(t('theme.renamedOnImport', { id }), 'info');
        }
        await uiStore.addCustomTheme(theme);
        await uiStore.setThemePreference(customId(theme));
        uiStore.showToast(t('theme.imported', { name: theme.name }), 'success');

        const failures = contrastFailures(theme);
        if (failures.length > 0) {
            const pairs = failures
                .map((failure) => `--${failure.token} ${failure.ratio.toFixed(2)}`)
                .join(', ');
            uiStore.showToast(t('theme.contrastWarning', { pairs }), 'warning', 8000);
        }
    } catch {
        uiStore.showToast(t('app.error.internal'), 'error');
    }
}

function openCustomMenu(theme: CustomTheme, event: MouseEvent): void {
    event.stopPropagation();
    const items: MenuItem[] = [
        { id: 'export', label: t('theme.export'), onSelect: () => void writeThemeFile(theme) },
        {
            id: 'delete',
            label: t('theme.delete'),
            danger: true,
            onSelect: () => void uiStore.removeCustomTheme(theme.id),
        },
    ];
    menuStore.openBelow(items, event.currentTarget as HTMLElement, t('theme.customActions'));
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

<div class="custom-section">
    <div class="custom-head">
        <h3>{t('settings.customThemes')}</h3>
        <div class="custom-actions">
            <button type="button" class="action" onclick={() => void importTheme()}>
                {t('theme.import')}
            </button>
            <button type="button" class="action" onclick={exportCurrent}>
                {t('theme.export')}
            </button>
        </div>
    </div>

    {#if uiStore.customThemes.length > 0}
        <div class="theme-grid" role="group" aria-label={t('settings.customThemes')}>
            {#each uiStore.customThemes as theme (theme.id)}
                {@const id = customId(theme)}
                {@const active = uiStore.themePreference === id}
                <div class="custom-swatch">
                    <button
                        class="swatch"
                        class:active
                        type="button"
                        aria-pressed={active}
                        aria-label={t('theme.select', { name: theme.name })}
                        onclick={() => pick(id)}
                    >
                        <span class="preview" data-theme={id}>
                            <span class="chip chip-bg"></span>
                            <span class="chip chip-accent"></span>
                            <span class="chip chip-fg"></span>
                        </span>
                        <span class="name">{theme.name}</span>
                    </button>
                    <button
                        class="menu-trigger"
                        type="button"
                        aria-label={t('theme.customActions')}
                        use:tooltip={t('theme.customActions')}
                        onclick={(event) => openCustomMenu(theme, event)}
                    >
                        <span aria-hidden="true">⋯</span>
                    </button>
                </div>
            {/each}
        </div>
    {:else}
        <p class="custom-empty">{t('theme.customEmpty')}</p>
    {/if}
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
        border: 1px solid var(--hairline);
        border-radius: var(--radius-lg);
        background: var(--surface-raised);
        text-align: left;
        width: 100%;
        transition:
            background var(--dur-fast) var(--ease),
            border-color var(--dur-fast) var(--ease),
            box-shadow var(--dur-fast) var(--ease);
    }

    .swatch:hover {
        background: var(--surface-overlay);
        border-color: var(--hairline-strong);
    }

    .swatch.active {
        border-color: var(--accent);
        box-shadow: inset 0 0 0 1px var(--accent);
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
        border: 1px solid var(--hairline);
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
        font-size: var(--text-sm);
        color: var(--fg);
    }

    .custom-section {
        display: flex;
        flex-direction: column;
        gap: var(--space-3);
    }

    .custom-head {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: var(--space-3);
        flex-wrap: wrap;
    }

    .custom-head h3 {
        font-size: 0.6875rem;
        font-weight: 650;
        letter-spacing: var(--tracking-caps);
        text-transform: uppercase;
        color: var(--fg-subtle);
    }

    .custom-actions {
        display: flex;
        gap: var(--space-2);
    }

    .action {
        padding: var(--space-1) var(--space-3);
        border-radius: var(--radius-md);
        background: transparent;
        color: var(--fg-muted);
        border: 1px solid var(--hairline-strong);
        font-size: var(--text-sm);
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .action:hover {
        background: var(--hover);
        color: var(--fg);
    }

    .custom-swatch {
        position: relative;
    }

    .menu-trigger {
        position: absolute;
        top: var(--space-2);
        right: var(--space-2);
        display: inline-grid;
        place-items: center;
        width: 24px;
        height: 24px;
        border-radius: var(--radius-sm);
        background: color-mix(in srgb, var(--surface-raised), transparent 15%);
        color: var(--fg-muted);
        font-size: var(--text-md);
        line-height: 1;
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .menu-trigger:hover {
        background: var(--hover);
        color: var(--fg);
    }

    .custom-empty {
        font-size: var(--text-xs);
        color: var(--fg-subtle);
    }
</style>

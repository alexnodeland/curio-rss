<script lang="ts">
export const compact = false;
</script>

<div class="theme-picker" class:compact>
    <h3 class="theme-picker-title">Theme</h3>
    <div class="theme-grid">
        {#each THEMES as theme (theme.id)}
            <button
                class="theme-option"
                class:selected={$currentTheme === theme.id}
                class:dark={theme.isDark}
                on:click={() => setTheme(theme.id)}
                aria-pressed={$currentTheme === theme.id}
            >
                <div class="theme-preview" data-theme={theme.id}>
                    <div class="preview-sidebar"></div>
                    <div class="preview-content">
                        <div class="preview-header"></div>
                        <div class="preview-line"></div>
                        <div class="preview-line short"></div>
                    </div>
                </div>
                <span class="theme-name">{theme.name}</span>
                {#if $currentTheme === theme.id}
                    <svg class="check-icon" xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <polyline points="20 6 9 17 4 12"/>
                    </svg>
                {/if}
            </button>
        {/each}
    </div>
</div>

<style>
    .theme-picker {
        padding: var(--space-4);
    }

    .theme-picker.compact {
        padding: var(--space-2);
    }

    .theme-picker-title {
        font-size: 14px;
        font-weight: 600;
        color: var(--fg);
        margin-bottom: var(--space-3);
    }

    .compact .theme-picker-title {
        display: none;
    }

    .theme-grid {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
        gap: var(--space-3);
    }

    .compact .theme-grid {
        grid-template-columns: repeat(auto-fill, minmax(100px, 1fr));
        gap: var(--space-2);
    }

    .theme-option {
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: var(--space-2);
        padding: var(--space-3);
        background: var(--bg-secondary);
        border: 2px solid var(--border);
        border-radius: var(--radius-md);
        cursor: pointer;
        transition: border-color 0.15s, background 0.15s;
        position: relative;
    }

    .theme-option:hover {
        background: var(--bg-tertiary);
    }

    .theme-option.selected {
        border-color: var(--accent);
    }

    .theme-preview {
        width: 100%;
        height: 64px;
        border-radius: var(--radius-sm);
        overflow: hidden;
        display: flex;
        background: var(--bg);
    }

    .preview-sidebar {
        width: 24%;
        background: var(--bg-secondary);
        border-right: 1px solid var(--border);
    }

    .preview-content {
        flex: 1;
        padding: 6px;
        display: flex;
        flex-direction: column;
        gap: 4px;
    }

    .preview-header {
        width: 60%;
        height: 8px;
        background: var(--accent);
        border-radius: 2px;
    }

    .preview-line {
        width: 100%;
        height: 6px;
        background: var(--fg-subtle);
        border-radius: 2px;
        opacity: 0.5;
    }

    .preview-line.short {
        width: 70%;
    }

    .theme-name {
        font-size: 12px;
        font-weight: 500;
        color: var(--fg);
    }

    .compact .theme-name {
        font-size: 11px;
    }

    .check-icon {
        position: absolute;
        top: var(--space-2);
        right: var(--space-2);
        color: var(--accent);
    }
</style>

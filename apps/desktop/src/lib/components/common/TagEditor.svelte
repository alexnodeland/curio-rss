<script lang="ts">
/**
 * A chip editor for a feed's folders & tags. Each tag is a removable pill; a
 * trailing input adds new ones (Enter or comma commits, Backspace on an empty
 * field removes the last). Path tags (`Tech/Databases`) render with a folder
 * glyph so folder membership reads distinctly from a flat label. Purely
 * presentational: it never calls IPC — it emits the next tag list through
 * `onchange`, and the caller persists it (via `set_feed_tags`).
 */
import { t } from '$lib/i18n';

let {
    tags,
    onchange,
    label,
    describedById,
}: {
    tags: string[];
    onchange: (next: string[]) => void;
    label: string;
    describedById?: string;
} = $props();

let draft = $state('');

function addFrom(raw: string): void {
    // A pasted "a, b/c" adds both; existing tags and blanks are dropped.
    const additions = raw
        .split(',')
        .map((tag) => tag.trim())
        .filter((tag) => tag.length > 0 && !tags.includes(tag));
    if (additions.length > 0) {
        onchange([...tags, ...additions]);
    }
    draft = '';
}

function remove(tag: string): void {
    onchange(tags.filter((candidate) => candidate !== tag));
}

function onKeydown(event: KeyboardEvent): void {
    if (event.key === 'Enter' || event.key === ',') {
        event.preventDefault();
        addFrom(draft);
    } else if (event.key === 'Backspace' && draft === '' && tags.length > 0) {
        event.preventDefault();
        remove(tags[tags.length - 1]);
    }
}
</script>

<div class="tag-editor" role="group" aria-label={label}>
    {#each tags as tag (tag)}
        <span class="chip" class:chip-path={tag.includes('/')}>
            {#if tag.includes('/')}
                <span class="chip-glyph" aria-hidden="true">▸</span>
            {/if}
            <span class="chip-label">{tag}</span>
            <button
                type="button"
                class="chip-remove"
                aria-label={t('tagEditor.remove', { tag })}
                onclick={() => remove(tag)}>×</button
            >
        </span>
    {/each}
    <input
        class="tag-input"
        type="text"
        bind:value={draft}
        placeholder={t('tagEditor.placeholder')}
        aria-label={label}
        aria-describedby={describedById}
        onkeydown={onKeydown}
        onblur={() => addFrom(draft)}
    />
</div>

<style>
    .tag-editor {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--space-2);
        padding: var(--space-2);
        border-radius: var(--radius-md);
        background: var(--surface-raised);
        border: 1px solid var(--hairline-strong);
    }

    .chip {
        display: inline-flex;
        align-items: center;
        gap: var(--space-1);
        padding: 2px var(--space-1) 2px var(--space-2);
        border-radius: var(--radius-pill);
        background: var(--surface-inset);
        border: 1px solid var(--hairline);
        font-size: var(--text-xs);
        color: var(--fg-muted);
        max-width: 100%;
    }

    .chip-path {
        color: var(--fg);
    }

    .chip-glyph {
        color: var(--accent);
        font-size: 0.625rem;
    }

    .chip-label {
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .chip-remove {
        flex: 0 0 auto;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        width: 1rem;
        height: 1rem;
        border-radius: 50%;
        background: transparent;
        color: var(--fg-subtle);
        font-size: var(--text-md);
        line-height: 1;
        transition: color var(--dur-fast) var(--ease);
    }

    .chip-remove:hover {
        color: var(--error-text);
    }

    .tag-input {
        flex: 1 1 6rem;
        min-width: 6rem;
        padding: var(--space-1) var(--space-1);
        background: transparent;
        border: none;
        color: var(--fg);
        font-size: var(--text-md);
    }

    .tag-input:focus {
        outline: none;
    }
</style>

<script lang="ts">
/**
 * Per-article tags: list, add, remove — over the tag commands. The tag list
 * rides the query cache (`article-tags:<id>`), so a successful add/remove
 * refreshes through the Rust-emitted `ArticlesChanged` event like every
 * other read; no optimistic client bookkeeping. Invalid tags surface as the
 * command's own `User`-tier error toast (core rejects them).
 */
import { commands } from '$lib/bindings';
import { t } from '$lib/i18n';
import { toastCommandError } from '$lib/state/actions';
import { ensureQuery, queryKeys } from '$lib/state/query-cache.svelte';
import { uiStore } from '$lib/state/ui.svelte';

let { articleId }: { articleId: number } = $props();

let draft = $state('');

function tags(): string[] {
    return (
        ensureQuery(queryKeys.articleTags(articleId), () => commands.getArticleTags(articleId))
            .data ?? []
    );
}

async function addTag(): Promise<void> {
    const tag = draft.trim();
    if (tag.length === 0) {
        return;
    }
    draft = '';
    try {
        const result = await commands.tagArticle(articleId, tag);
        if (result.status === 'error') {
            toastCommandError(result.error);
        }
    } catch {
        uiStore.showToast(t('app.error.internal'), 'error');
    }
}

async function removeTag(tag: string): Promise<void> {
    try {
        const result = await commands.untagArticle(articleId, tag);
        if (result.status === 'error') {
            toastCommandError(result.error);
        }
    } catch {
        uiStore.showToast(t('app.error.internal'), 'error');
    }
}

function onKeydown(event: KeyboardEvent): void {
    if (event.key === 'Enter') {
        event.preventDefault();
        void addTag();
    }
}
</script>

<div class="tags" aria-label={t('tags.label')}>
    <ul class="tag-list">
        {#each tags() as tag (tag)}
            <li class="tag">
                <span class="tag-name">{tag}</span>
                <button
                    class="tag-remove"
                    type="button"
                    aria-label={t('tags.remove', { tag })}
                    onclick={() => void removeTag(tag)}>×</button
                >
            </li>
        {/each}
    </ul>
    <input
        class="tag-input"
        type="text"
        spellcheck="false"
        bind:value={draft}
        placeholder={t('tags.placeholder')}
        aria-label={t('tags.add')}
        onkeydown={onKeydown}
    />
</div>

<style>
    .tags {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--space-2);
        margin-top: var(--space-3);
    }

    .tag-list {
        display: flex;
        flex-wrap: wrap;
        gap: var(--space-2);
        list-style: none;
    }

    .tag {
        display: inline-flex;
        align-items: center;
        gap: 3px;
        padding: 3px 3px 3px var(--space-2);
        border-radius: var(--radius-pill);
        background: var(--surface-raised);
        border: 1px solid var(--hairline);
        color: var(--fg-muted);
        font-family: var(--font-family);
        font-size: var(--text-xs);
        font-weight: 500;
    }

    .tag::before {
        content: '#';
        color: var(--fg-subtle);
    }

    .tag-remove {
        display: flex;
        align-items: center;
        justify-content: center;
        width: 1.05rem;
        height: 1.05rem;
        border-radius: 50%;
        background: transparent;
        color: var(--fg-subtle);
        font-size: 0.875rem;
        line-height: 1;
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .tag-remove:hover {
        color: var(--fg);
        background: var(--active);
    }

    .tag-input {
        flex: 0 0 auto;
        width: 8rem;
        padding: 3px var(--space-3);
        border-radius: var(--radius-pill);
        background: transparent;
        color: var(--fg);
        border: 1px dashed var(--hairline-strong);
        font-family: var(--font-family);
        font-size: var(--text-xs);
    }

    .tag-input:focus-visible {
        outline: none;
        border-style: solid;
        border-color: color-mix(in srgb, var(--accent), transparent 40%);
        box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent), transparent 82%);
    }
</style>

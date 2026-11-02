<script lang="ts">
/**
 * First run: with no subscriptions and nothing saved, this takes the list
 * and reader panes together (see `shouldWelcome`). One primary action (add a
 * feed), the starter sources that pre-fill the add-feed modal, the importers
 * for bringing a library over from another app, and a three-key primer whose
 * keys come from the shortcut registry — so it never names an unbound key.
 * It steps aside on its own: the first feed or saved article lands through
 * the usual invalidation events and the panes return.
 */
import Icon from '$components/common/Icon.svelte';
import type { ImportSourceDto } from '$lib/bindings';
import { type MessageKey, t } from '$lib/i18n';
import { shortcutKbd } from '$lib/keyboard/labels';
import type { ShortcutId } from '$lib/keyboard/registry';
import { importFromFile } from '$lib/state/actions';
import { layoutStore } from '$lib/state/layout.svelte';
import { STARTER_PRESETS } from '$lib/utils/source-presets';

const IMPORTS: readonly { source: ImportSourceDto; label: MessageKey }[] = [
    { source: 'opml', label: 'import.source.opml' },
    { source: 'pocket_csv', label: 'import.source.pocket_csv' },
    { source: 'instapaper_csv', label: 'import.source.instapaper_csv' },
    { source: 'readwise_csv', label: 'import.source.readwise_csv' },
];

const PRIMER: readonly { ids: readonly ShortcutId[]; label: MessageKey }[] = [
    { ids: ['nav.nextArticle', 'nav.previousArticle'], label: 'reader.hint.move' },
    { ids: ['article.toggleStar'], label: 'welcome.keys.star' },
    { ids: ['help.toggle'], label: 'reader.hint.help' },
];

/** The printed keys of one primer row (only keys the registry binds). */
function primerKeys(ids: readonly ShortcutId[]): string[] {
    return ids.flatMap((id) => {
        const kbd = shortcutKbd(id);
        return kbd === undefined ? [] : [kbd];
    });
}
</script>

<section class="welcome" aria-labelledby="welcome-title" data-testid="welcome">
    <div class="welcome-inner">
        <header class="welcome-head">
            <div class="welcome-mark" aria-hidden="true">
                <Icon name="rss" size={30} strokeWidth={1.5} />
            </div>
            <h1 id="welcome-title" class="welcome-title">{t('welcome.title')}</h1>
            <p class="welcome-intro">{t('welcome.intro')}</p>
            <button class="state-action primary welcome-cta" type="button" onclick={() => layoutStore.openAddFeed()}>
                <Icon name="plus" size={16} />
                {t('emptyState.addFeed')}
            </button>
        </header>

        <section class="welcome-group" aria-labelledby="welcome-presets">
            <h2 id="welcome-presets" class="welcome-label">{t('presets.title')}</h2>
            <div class="welcome-row">
                {#each STARTER_PRESETS as preset (preset.id)}
                    <button
                        class="state-action"
                        type="button"
                        onclick={() => layoutStore.openAddFeed({ url: preset.url, tags: preset.tags })}
                        >{t(preset.labelKey)}</button
                    >
                {/each}
            </div>
        </section>

        <section class="welcome-group" aria-labelledby="welcome-import">
            <h2 id="welcome-import" class="welcome-label">{t('welcome.import')}</h2>
            <div class="welcome-row">
                {#each IMPORTS as entry (entry.source)}
                    <button class="state-action" type="button" onclick={() => void importFromFile(entry.source)}
                        >{t(entry.label)}</button
                    >
                {/each}
            </div>
        </section>

        <section class="welcome-group" aria-labelledby="welcome-keys">
            <h2 id="welcome-keys" class="welcome-label">{t('welcome.keys')}</h2>
            <ul class="welcome-keys">
                {#each PRIMER as row (row.label)}
                    <li>
                        <span class="keys">
                            {#each primerKeys(row.ids) as key (key)}<kbd>{key}</kbd>{/each}
                        </span>
                        {t(row.label)}
                    </li>
                {/each}
            </ul>
        </section>
    </div>
</section>

<style>
    .welcome {
        flex: 1 1 auto;
        display: flex;
        min-width: 0;
        min-height: 0;
        overflow-y: auto;
        background: var(--surface-app);
    }

    /* `margin: auto` centres the column while the pane is taller than it, and
       lets it scroll from the top (never clipped) once it isn't. */
    .welcome-inner {
        display: flex;
        flex-direction: column;
        gap: var(--space-8);
        width: 100%;
        max-width: 44rem;
        margin: auto;
        padding: var(--space-10) var(--space-6);
    }

    .welcome-head {
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: var(--space-3);
        text-align: center;
    }

    .welcome-mark {
        display: grid;
        place-items: center;
        width: 68px;
        height: 68px;
        margin-bottom: var(--space-2);
        border-radius: var(--radius-xl);
        color: var(--accent);
        background: var(--selected);
        box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent), transparent 70%);
    }

    .welcome-title {
        font-size: var(--text-xl);
        font-weight: 620;
        letter-spacing: var(--tracking-tight);
        color: var(--fg);
    }

    .welcome-intro {
        max-width: 28rem;
        font-size: var(--text-md);
        line-height: 1.55;
        color: var(--fg-muted);
    }

    .welcome-cta {
        margin-top: var(--space-3);
        padding: var(--space-2) var(--space-4);
        font-size: var(--text-md);
    }

    .welcome-group {
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: var(--space-3);
    }

    .welcome-label {
        font-size: 0.6875rem;
        font-weight: 650;
        letter-spacing: var(--tracking-caps);
        text-transform: uppercase;
        color: var(--fg-subtle);
    }

    .welcome-row {
        display: flex;
        flex-wrap: wrap;
        justify-content: center;
        gap: var(--space-2);
    }

    .welcome-keys {
        display: flex;
        flex-wrap: wrap;
        justify-content: center;
        gap: var(--space-2) var(--space-6);
        list-style: none;
        font-size: var(--text-sm);
        color: var(--fg-muted);
    }

    .welcome-keys li {
        display: inline-flex;
        align-items: center;
        gap: var(--space-2);
        white-space: nowrap;
    }

    .keys {
        display: inline-flex;
        gap: var(--space-1);
    }

    kbd {
        display: inline-block;
        min-width: 1.6em;
        padding: 0.1em 0.45em;
        border-radius: var(--radius-sm);
        background: var(--surface-raised);
        border: 1px solid var(--hairline);
        box-shadow: 0 1px 0 var(--hairline-strong);
        font-family: var(--font-mono);
        font-size: 0.82em;
        color: var(--fg-muted);
        text-align: center;
    }
</style>

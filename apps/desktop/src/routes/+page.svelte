<script lang="ts">
import { t } from '$lib/i18n';
import { feedsStore } from '$lib/state/feeds.svelte';
</script>

<!--
  The WP2 shell: token-styled chrome plus the live subscription/unread
  status straight from the runes stores. WP3 replaces the status section
  with the three-pane reader (Sidebar / ArticleList / ReaderPane).
-->
<main class="shell">
    <header class="shell-header">
        <h1>{t('app.title')}</h1>
        <p class="tagline">{t('app.tagline')}</p>
    </header>
    <section class="shell-status" aria-live="polite">
        {#if feedsStore.error !== null}
            <p class="status-error" role="alert">
                {feedsStore.error.kind === 'user'
                    ? feedsStore.error.message
                    : t('app.error.internal')}
            </p>
        {:else if !feedsStore.loaded}
            <p class="status-muted">{t('shell.feeds.loading')}</p>
        {:else if feedsStore.feeds.length === 0}
            <p class="status-muted">{t('shell.feeds.empty')}</p>
        {:else}
            <p>
                {t('shell.feeds.count', { count: feedsStore.feeds.length })}
                <span class="dot" aria-hidden="true">·</span>
                {t('shell.unread.count', { count: feedsStore.unreadTotal })}
            </p>
        {/if}
    </section>
</main>

<style>
    .shell {
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        gap: var(--space-4);
        height: 100vh;
        padding: var(--space-8);
        text-align: center;
    }

    .shell-header h1 {
        font-size: 2rem;
        letter-spacing: -0.02em;
    }

    .tagline {
        color: var(--fg-muted);
        margin-top: var(--space-1);
    }

    .shell-status {
        background: var(--bg-secondary);
        border: 1px solid var(--border-subtle);
        border-radius: var(--radius-lg);
        padding: var(--space-3) var(--space-5);
    }

    .status-muted {
        color: var(--fg-muted);
    }

    .status-error {
        color: var(--error);
    }

    .dot {
        color: var(--fg-subtle);
        margin: 0 var(--space-1);
    }
</style>

<script lang="ts">
/**
 * A live reader-typography preview for Settings → Reading. It renders a small
 * sample article through the *real* `SanitizedHtml` component and mirrors
 * *every* reader typography input (font / size / line-height / measure /
 * paragraph-spacing / text-align from the uiStore) and the active theme, so
 * what you see is exactly what the reader will render — and it updates as the
 * `TypographyControls` controls move. Keep this style list in lockstep with
 * `ReaderPane`'s `.reader-article`; a setting the reader honours but the
 * preview drops reads as "the control does nothing".
 */
import SanitizedHtml from '$components/reader/SanitizedHtml.svelte';
import { t } from '$lib/i18n';
import { uiStore } from '$lib/state/ui.svelte';

// Assembled from i18n strings so the sample text localizes; the markup (a
// heading, prose, and a pull-quote) exercises the prose rhythm the controls
// affect. It flows through SanitizedHtml → DOMPurify like any article body.
const sampleHtml = $derived(
    `<h2>${t('typography.preview.heading')}</h2>` +
        `<p>${t('typography.preview.body')}</p>` +
        `<blockquote><p>${t('typography.preview.quote')}</p></blockquote>`,
);
</script>

<div class="preview" role="group" aria-label={t('typography.preview')}>
    <span class="preview-label">{t('typography.preview')}</span>
    <div class="preview-surface">
        <article
            class="preview-article"
            class:justify={uiStore.textAlign === 'justify'}
            style:font-size="{uiStore.fontSize}px"
            style:line-height={uiStore.lineHeight}
            style:max-width="{uiStore.measure}px"
            style:font-family={uiStore.readerFontStack}
            style:--para-spacing={uiStore.paragraphSpacing}
        >
            <SanitizedHtml html={sampleHtml} />
        </article>
    </div>
</div>

<style>
    .preview {
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
    }

    .preview-label {
        font-size: var(--text-sm);
        color: var(--fg-muted);
    }

    /* The reader's own background + a hairline frame, so the sample reads as a
       faithful mini-reader rather than part of the settings surface. */
    .preview-surface {
        max-height: 260px;
        overflow-y: auto;
        border-radius: var(--radius-md);
        border: 1px solid var(--hairline);
        background: var(--bg);
        color: var(--ink);
    }

    .preview-article {
        margin: 0 auto;
        padding: var(--space-5) var(--space-6);
    }

    /* Mirrors ReaderPane's justify affordance so the Align control previews. */
    .preview-article.justify :global(.sanitized-content) {
        text-align: justify;
        hyphens: auto;
    }
</style>

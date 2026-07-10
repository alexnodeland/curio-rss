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

// Assembled from i18n strings so the sample text localizes; the markup
// deliberately exercises every block the reader stylesheet styles — heading,
// prose, a bulleted list, an inline-code + link line, and a pull-quote — so the
// preview genuinely tests the reader CSS, not just paragraph rhythm. It flows
// through SanitizedHtml → DOMPurify like any article body. The link is an
// in-page anchor (`#`) so a stray click in the settings preview can't navigate.
const sampleHtml = $derived(
    `<h2>${t('typography.preview.heading')}</h2>` +
        `<p>${t('typography.preview.body')}</p>` +
        `<p>${t('typography.preview.listIntro')}</p>` +
        `<ul><li>${t('typography.preview.item1')}</li>` +
        `<li>${t('typography.preview.item2')}</li>` +
        `<li>${t('typography.preview.item3')}</li></ul>` +
        `<p>${t('typography.preview.inlineLead')} <code>text-align: justify</code>, ` +
        `<a href="#">${t('typography.preview.linkText')}</a>.</p>` +
        `<blockquote><p>${t('typography.preview.quote')}</p></blockquote>`,
);
</script>

<div class="preview" role="group" aria-label={t('typography.preview')}>
    <span class="preview-label">{t('typography.preview')}</span>
    <div class="preview-surface" data-reading-theme={uiStore.readingTheme}>
        <article
            class="preview-article"
            class:justify={uiStore.textAlign === 'justify'}
            class:hyphenate={uiStore.hyphenate}
            style:font-size="{uiStore.fontSize}px"
            style:line-height={uiStore.lineHeight}
            style:max-width="{uiStore.measure}px"
            style:font-family={uiStore.readerFontStack}
            style:font-weight={uiStore.fontWeight}
            style:letter-spacing="{uiStore.letterSpacing}em"
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

    .preview-article.hyphenate :global(.sanitized-content) {
        hyphens: auto;
    }

    /* Mirrors ReaderPane's reading-surface tints so the control previews. The
       ink tokens are forced directly (`--ink` is a :root color-mix that
       inherits its computed value, so overriding `--fg` alone wouldn't repaint
       the prose). */
    .preview-surface[data-reading-theme='sepia'] {
        background: #f1e5cf;
        --ink: #43392b;
        --ink-soft: #6b5b45;
        --fg: #43392b;
        --fg-muted: #5c4f3b;
        --fg-subtle: #7a6a52;
    }

    .preview-surface[data-reading-theme='paper'] {
        background: #faf6ee;
        --ink: #2b2b2b;
        --ink-soft: #555555;
        --fg: #2b2b2b;
        --fg-muted: #4a4a4a;
        --fg-subtle: #6b6b6b;
    }
</style>

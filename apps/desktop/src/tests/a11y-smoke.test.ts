/**
 * Accessibility smoke test: runs axe-core over the components most likely
 * to regress on ARIA semantics — the shared modal shell (via the help
 * dialog), the toast outlet, and the whole shell's list + reader driven by
 * the keyboard alone — and fails on any violation.
 *
 * jsdom can't lay elements out, so `color-contrast` (which needs real
 * geometry) is disabled here; contrast is enforced separately over the
 * theme tokens. Page-scoped rules (landmarks, a single main, one h1) don't
 * apply to a component rendered in isolation and are disabled too.
 */
import Toasts from '$components/common/Toasts.svelte';
import HelpOverlay from '$components/modals/HelpOverlay.svelte';
import { articlesStore } from '$lib/state/articles.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { toastStore } from '$lib/state/toasts.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import axe from 'axe-core';
import { tick } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import Page from '../routes/+page.svelte';
import {
    type IpcHarness,
    articleFixture,
    articleStateFixture,
    articleSummaryFixture,
    feedFixture,
    flushIpc,
    installIpcHarness,
    unreadCountsFixture,
} from './ipc-harness';
import { resetUiStores } from './ui-stores';

const AXE_OPTIONS: axe.RunOptions = {
    rules: {
        'color-contrast': { enabled: false },
        region: { enabled: false },
        'landmark-one-main': { enabled: false },
        'page-has-heading-one': { enabled: false },
    },
};

/** Runs axe over a node and returns a readable list of any violations. */
async function auditViolations(node: Element): Promise<string[]> {
    const results = await axe.run(node, AXE_OPTIONS);
    return results.violations.map((violation) => {
        const where = violation.nodes.map((n) => n.target.join(' ')).join(', ');
        return `${violation.id} (${violation.impact}): ${violation.help} — at ${where}`;
    });
}

describe('accessibility smoke', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        resetQueryCache();
        articlesStore.reset();
        selectionStore.reset();
        resetUiStores();
        harness?.teardown();
        harness = null;
    });

    it('the help dialog has no axe violations', async () => {
        const { container } = render(HelpOverlay, { props: { onclose: vi.fn() } });
        const violations = await auditViolations(container);
        expect(violations, violations.join('\n')).toEqual([]);
    });

    it('the toast outlet has no axe violations', async () => {
        const { container } = render(Toasts);
        toastStore.showToast('note saved', 'success', 0);
        toastStore.showToast('fetch failed', 'error', 0);
        await tick();
        const violations = await auditViolations(container);
        expect(violations, violations.join('\n')).toEqual([]);
    });

    it('the list and reader pass axe while driven by the keyboard alone', async () => {
        harness = installIpcHarness({
            list_feeds: [feedFixture({ id: 1, title: 'Alpha' })],
            get_unread_counts: unreadCountsFixture({ total: 3, by_feed: [[1, 3]] }),
            list_articles: [301, 300, 299].map((id) =>
                articleSummaryFixture({ id, feed_id: 1, title: `Article ${id}` }),
            ),
            get_article: (args) =>
                articleFixture({
                    id: args.articleId as number,
                    title: `Article ${String(args.articleId)}`,
                    content_html: '<p>Body text with <a href="https://example.org">a link</a>.</p>',
                }),
            get_article_state: articleStateFixture(),
            mark_read: true,
            list_tags: [],
        });
        const { container } = render(Page);
        await flushIpc();
        const audit = async (step: string): Promise<void> => {
            await flushIpc();
            await tick();
            const violations = await auditViolations(container);
            expect(violations, `${step}:\n${violations.join('\n')}`).toEqual([]);
        };

        await audit('at rest');

        // ↓ from an unfocused page selects the first row; the listbox tracks it.
        await fireEvent.keyDown(document.body, { key: 'ArrowDown' });
        const listbox = container.querySelector('[role="listbox"]') as HTMLElement;
        listbox.focus();
        await fireEvent.keyDown(listbox, { key: 'ArrowDown' });
        expect(selectionStore.selectedArticleId).toBe(300);
        await audit('list focused with a selection');

        // Enter hands the keyboard to the reader body: a named, focusable region.
        await fireEvent.keyDown(listbox, { key: 'Enter' });
        await flushIpc();
        const body = container.querySelector('.reader-scroll') as HTMLElement;
        expect(document.activeElement).toBe(body);
        await audit('reader body focused');

        // Space pages (jsdom: already at the end → next unread), Esc goes back.
        await fireEvent.keyDown(body, { key: ' ' });
        await flushIpc();
        await fireEvent.keyDown(container.querySelector('.reader-scroll') as HTMLElement, {
            key: 'Escape',
        });
        expect(document.activeElement).toBe(listbox);
        await audit('back in the list');
    });
});

/**
 * First run (#151): the whole shell over mockIPC with no subscriptions — the
 * welcome view takes the list and reader's place, its actions reach the
 * add-feed modal (pre-filled by a starter source) and every importer, its
 * primer names the registry's keys, and it steps aside once a feed lands. A
 * feed-less library of imported saves, or another view, keeps the panes.
 */
import { events } from '$lib/bindings';
import { selectView } from '$lib/state/actions';
import { articlesStore } from '$lib/state/articles.svelte';
import { layoutStore } from '$lib/state/layout.svelte';
import { resetQueryCache, wireInvalidation } from '$lib/state/query-cache.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { cleanup, fireEvent, render, within } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import Page from '../routes/+page.svelte';
import {
    type IpcHarness,
    type Responder,
    articleStateFixture,
    articleSummaryFixture,
    feedFixture,
    flushIpc,
    installIpcHarness,
    unreadCountsFixture,
} from './ipc-harness';
import { resetUiStores } from './ui-stores';

function firstRun(overrides: Record<string, Responder> = {}): Record<string, Responder> {
    return {
        list_feeds: [],
        get_unread_counts: unreadCountsFixture({ total: 0, by_feed: [] }),
        list_articles: [],
        get_article_state: articleStateFixture(),
        set_setting: null,
        ...overrides,
    };
}

describe('welcome view', () => {
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

    it('with 0 feeds, renders across the list and reader panes', async () => {
        harness = installIpcHarness(firstRun());
        const { getByTestId, queryByRole, getByRole } = render(Page);
        await flushIpc();

        const welcome = getByTestId('welcome');
        expect(within(welcome).getByRole('heading', { level: 1 }).textContent).toBe(
            'Welcome to Curio',
        );
        // Neither the list (search, listbox, list empty state) nor the reader.
        expect(queryByRole('searchbox')).toBeNull();
        expect(queryByRole('listbox')).toBeNull();
        expect(queryByRole('toolbar', { name: 'Article actions' })).toBeNull();
        expect(document.querySelector('.article-list')).toBeNull();
        expect(document.querySelector('.reader')).toBeNull();
        // The sidebar stays.
        expect(getByRole('heading', { name: 'Feeds' })).toBeTruthy();
    });

    it('its primary action opens the add-feed modal', async () => {
        harness = installIpcHarness(firstRun());
        const { getByTestId } = render(Page);
        await flushIpc();

        await fireEvent.click(
            within(getByTestId('welcome')).getByRole('button', { name: 'Add your first feed' }),
        );
        expect(layoutStore.activeModal).toBe('add-feed');
        expect(layoutStore.addFeedSeed).toBeNull();
    });

    it('a starter source opens the modal pre-filled with its pattern', async () => {
        harness = installIpcHarness(firstRun());
        const { getByTestId, findByTestId } = render(Page);
        await flushIpc();

        await fireEvent.click(
            within(getByTestId('welcome')).getByRole('button', { name: 'Hacker News' }),
        );
        const input = (await findByTestId('add-feed-url')) as HTMLInputElement;
        expect(input.value).toBe('https://hnrss.org/frontpage');
        const tags = document.querySelector<HTMLInputElement>('input.tags-input');
        expect(tags?.value).toBe('Community/Hacker News');

        layoutStore.closeModal();
        expect(layoutStore.addFeedSeed).toBeNull();
    });

    it('each importer runs the native import flow with its source', async () => {
        harness = installIpcHarness(
            firstRun({
                pick_import_file: { token: 'tok', path: '/tmp/export.csv' },
                import_file: {
                    feeds_added: 0,
                    articles_added: 2,
                    feeds_skipped: 0,
                    articles_skipped: 0,
                    feeds_invalid: 0,
                    articles_invalid: 0,
                },
            }),
        );
        const { getByTestId } = render(Page);
        await flushIpc();

        const welcome = within(getByTestId('welcome'));
        for (const name of [
            'OPML — subscriptions',
            'Pocket — CSV',
            'Instapaper — CSV',
            'Readwise Reader — CSV',
        ]) {
            await fireEvent.click(welcome.getByRole('button', { name }));
            await flushIpc();
        }
        expect(harness.callsFor('import_file').map((call) => call.source)).toEqual([
            'opml',
            'pocket_csv',
            'instapaper_csv',
            'readwise_csv',
        ]);
    });

    it('the primer prints the registry keys: j k, s and ?', async () => {
        harness = installIpcHarness(firstRun());
        const { getByTestId } = render(Page);
        await flushIpc();

        const keys = [...getByTestId('welcome').querySelectorAll('kbd')].map(
            (kbd) => kbd.textContent,
        );
        expect(keys).toEqual(['j', 'k', 's', '?']);
    });

    it('steps aside once a subscription lands', async () => {
        harness = installIpcHarness(firstRun());
        const unwire = await wireInvalidation();
        const { queryByTestId } = render(Page);
        await flushIpc();
        expect(queryByTestId('welcome')).not.toBeNull();

        harness.respond('list_feeds', [feedFixture({ id: 1, title: 'Alpha' })]);
        await events.feedsChanged.emit();
        await flushIpc();
        expect(queryByTestId('welcome')).toBeNull();
        expect(document.querySelector('.article-list')).not.toBeNull();
        unwire();
    });

    it('keeps the panes for a feed-less library of imported saves', async () => {
        harness = installIpcHarness(
            firstRun({
                list_articles: [articleSummaryFixture({ id: 9, feed_id: null, title: 'Saved' })],
            }),
        );
        const { queryByTestId, getByText } = render(Page);
        await flushIpc();
        expect(queryByTestId('welcome')).toBeNull();
        expect(getByText('Saved')).toBeTruthy();
    });

    it('another view keeps its own empty state', async () => {
        harness = installIpcHarness(firstRun());
        const { queryByTestId, findByText } = render(Page);
        await flushIpc();

        selectView('starred');
        await flushIpc();
        expect(queryByTestId('welcome')).toBeNull();
        expect(await findByText('No starred articles')).toBeTruthy();
    });
});

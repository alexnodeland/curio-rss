/**
 * Zero-states (#151): every built-in view has its own empty copy, in every
 * locale; a failing feed explains itself from its classified failure (never
 * the engine's English text); the hint's `{key}` slot splits out for a
 * `<kbd>`; and the first-run gate only opens for an empty, feed-less All view.
 */
import type { FetchErrorKindDto } from '$lib/bindings';
import { localeStore } from '$lib/i18n';
import { de } from '$lib/i18n/de';
import { en } from '$lib/i18n/en';
import { es } from '$lib/i18n/es';
import { fr } from '$lib/i18n/fr';
import { it as itCatalog } from '$lib/i18n/it';
import { pl } from '$lib/i18n/pl';
import { yue } from '$lib/i18n/yue';
import { zhHans } from '$lib/i18n/zh-Hans';
import { ALL_ARTICLES, type ArticleFilters } from '$lib/state/articles.svelte';
import {
    type WelcomeInput,
    feedErrorMessage,
    feedHasError,
    listEmptyState,
    shouldWelcome,
    splitAtKey,
} from '$lib/utils/empty-states';
import { STARTER_PRESETS, detectSource } from '$lib/utils/source-presets';
import { afterEach, describe, expect, it } from 'vitest';
import { feedFixture } from './ipc-harness';

/** The sidebar's built-in views, plus unread-only over All. */
const VIEWS: Record<string, ArticleFilters> = {
    all: ALL_ARTICLES,
    unread: { ...ALL_ARTICLES, read: false },
    starred: { ...ALL_ARTICLES, starred: true },
    readLater: { ...ALL_ARTICLES, readLater: true },
    archived: { ...ALL_ARTICLES, archived: true },
};

const CATALOGS = { en, es, fr, de, it: itCatalog, pl, 'zh-Hans': zhHans, yue };

afterEach(() => {
    localeStore.reset();
});

describe('listEmptyState', () => {
    it('gives each built-in view its own empty key, translated in all 8 locales', () => {
        const titles = Object.entries(VIEWS).map(([view, filters]) => {
            const state = listEmptyState(filters, null);
            expect(state.kind).toBe(view);
            return state.title;
        });
        expect(new Set(titles).size).toBe(titles.length);
        for (const [locale, catalog] of Object.entries(CATALOGS)) {
            for (const key of titles) {
                expect(catalog[key], `${locale} lacks ${key}`).toBeTruthy();
            }
            // No two views read the same in any locale either.
            expect(new Set(titles.map((key) => catalog[key])).size, locale).toBe(titles.length);
        }
    });

    it('offers "Show read" in an unread-only view, in any scope', () => {
        expect(listEmptyState(VIEWS.unread, null).actions).toEqual(['showRead']);
        const starredUnread = { ...VIEWS.starred, read: false };
        expect(listEmptyState(starredUnread, null).kind).toBe('unread');
    });

    it('names the registry key that fills Starred, Read later and Archived', () => {
        expect(listEmptyState(VIEWS.starred, null).hintKey).toBe('article.toggleStar');
        expect(listEmptyState(VIEWS.readLater, null).hintKey).toBe('article.toggleReadLater');
        expect(listEmptyState(VIEWS.archived, null).hintKey).toBe('article.toggleArchived');
    });

    it('words folder, tag and feed scopes for what they are', () => {
        expect(listEmptyState({ ...ALL_ARTICLES, feedTag: 'Tech' }, null).kind).toBe('folder');
        expect(listEmptyState({ ...ALL_ARTICLES, tag: 'rust' }, null).kind).toBe('tag');
        const feed = feedFixture({ id: 7 });
        const state = listEmptyState({ ...ALL_ARTICLES, feedId: 7 }, feed);
        expect(state.kind).toBe('feed');
        expect(state.actions).toEqual(['refreshFeed']);
    });

    it('explains a failing feed before anything else, with Retry and its health panel', () => {
        const feed = feedFixture({ id: 7, last_error: 'boom', last_error_kind: 'dns' });
        const state = listEmptyState({ ...ALL_ARTICLES, feedId: 7, read: false }, feed);
        expect(state.kind).toBe('feedError');
        expect(state.actions).toEqual(['retryFeed', 'feedHealth']);
        // Another feed's failure is not this view's.
        expect(listEmptyState({ ...ALL_ARTICLES, feedId: 8 }, feed).kind).toBe('feed');
    });

    it('does not treat a busy skip (another process refreshing) as a failure', () => {
        expect(feedHasError(feedFixture({ last_error: 'busy', last_error_kind: 'busy' }))).toBe(
            false,
        );
        expect(feedHasError(feedFixture({ last_error: 'boom', last_error_kind: null }))).toBe(true);
        expect(feedHasError(feedFixture())).toBe(false);
    });
});

describe('feedErrorMessage', () => {
    const feedWith = (kind: FetchErrorKindDto | null) =>
        feedFixture({
            url: 'https://news.example.org/rss',
            last_error: 'raw engine text',
            last_error_kind: kind,
        });

    it('names the host for the transport classes', () => {
        expect(feedErrorMessage(feedWith('dns'))).toBe(
            'Curio couldn’t find news.example.org. Check the address and your connection.',
        );
        expect(feedErrorMessage(feedWith('read_timeout'))).toContain(
            'news.example.org took too long',
        );
        expect(feedErrorMessage(feedWith('tls_certificate'))).toContain(
            'secure connection to news.example.org',
        );
        expect(feedErrorMessage(feedWith('refused'))).toContain('connect to news.example.org');
        expect(feedErrorMessage(feedWith('blocked'))).toContain('private network address');
        expect(feedErrorMessage(feedWith('too_large'))).toContain('too large');
    });

    it('explains the classes where the host did answer', () => {
        expect(feedErrorMessage(feedWith('not_a_feed'))).toContain('web page, not a feed');
        expect(feedErrorMessage(feedWith('parse'))).toContain('RSS or Atom');
        expect(feedErrorMessage(feedWith('gone'))).toContain('410');
        expect(feedErrorMessage(feedWith('http'))).toContain('answered with an error');
        expect(feedErrorMessage(feedWith('rate_limited'))).toContain('rate-limiting');
        expect(feedErrorMessage(feedWith('invalid_url'))).toContain('https://news.example.org/rss');
    });

    it('falls back to the generic line, never the engine text', () => {
        for (const kind of [null, 'busy'] as const) {
            const message = feedErrorMessage(feedWith(kind));
            expect(message).toBe(en['feedError.generic']);
            expect(message).not.toContain('raw engine text');
        }
    });

    it('is localized', async () => {
        await localeStore.set('fr');
        expect(feedErrorMessage(feedWith('gone'))).toBe(fr['feedError.gone']);
    });
});

describe('splitAtKey', () => {
    it('splits a hint around its {key} slot', () => {
        expect(splitAtKey('list.empty.starred.hint')).toEqual([
            'Press ',
            ' on an article to star it and keep it here.',
        ]);
    });

    it('keeps a line with no slot whole', () => {
        expect(splitAtKey('list.empty.all.hint')).toEqual([en['list.empty.all.hint'], null]);
    });
});

describe('shouldWelcome', () => {
    const firstRun: WelcomeInput = {
        feedsLoaded: true,
        feedsFailed: false,
        feedCount: 0,
        filters: ALL_ARTICLES,
        listLoaded: true,
        listCount: 0,
        searching: false,
    };

    it('opens for an empty, feed-less All view', () => {
        expect(shouldWelcome(firstRun)).toBe(true);
    });

    it.each([
        ['feeds still loading', { feedsLoaded: false }],
        ['feeds failed to load', { feedsFailed: true }],
        ['a subscription exists', { feedCount: 1 }],
        ['the list is still loading', { listLoaded: false }],
        ['imported saves exist', { listCount: 3 }],
        ['a search is active', { searching: true }],
        ['another view is open', { filters: { ...ALL_ARTICLES, starred: true } }],
        ['the unread-only toggle is on', { filters: { ...ALL_ARTICLES, read: false } }],
    ])('stays closed when %s', (_, change) => {
        expect(shouldWelcome({ ...firstRun, ...change })).toBe(false);
    });
});

describe('STARTER_PRESETS', () => {
    it('each starter pattern, finished, is a source the add-feed modal recognizes', () => {
        const finished: Record<string, string> = {
            reddit: 'rust',
            mastodon: 'curio@mastodon.social',
            github: 'tauri-apps/tauri',
        };
        for (const preset of STARTER_PRESETS) {
            if (preset.id === 'hackernews') {
                // A whole feed URL already, with the HN preset's folder.
                expect(preset.url).toBe('https://hnrss.org/frontpage');
                expect(preset.tags).toBe('Community/Hacker News');
                continue;
            }
            if (preset.id === 'youtube') {
                // An @handle resolves through discovery, not by pattern.
                expect(preset.url).toBe('https://www.youtube.com/@');
                continue;
            }
            expect(detectSource(`${preset.url}${finished[preset.id]}`)?.kind, preset.id).toBe(
                preset.id,
            );
        }
    });
});

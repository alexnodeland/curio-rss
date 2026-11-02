import type { IconName } from '$components/common/Icon.svelte';
/**
 * Zero-states — what an empty list says, and what it offers, for the view
 * the user is in. Every built-in view has its own copy (an unread-only view
 * is "all caught up", Starred explains how to star), and a selected feed
 * whose last fetch failed says why instead of looking merely empty. The
 * first-run gate (`shouldWelcome`) lives here too: with no subscriptions and
 * nothing saved, the list and reader give way to the welcome view.
 *
 * Pure over the filters and the DTOs, so each branch is unit-tested without
 * a component or IPC.
 */
import type { FeedDto, FetchErrorKindDto } from '$lib/bindings';
import { type MessageKey, t } from '$lib/i18n';
import type { ShortcutId } from '$lib/keyboard/registry';
import { ALL_ARTICLES, type ArticleFilters, filterKey } from '$lib/state/articles.svelte';
import { hostOf } from './source-presets';

/** Which zero-state a list shows. */
export type ListEmptyKind =
    | 'all'
    | 'unread'
    | 'starred'
    | 'readLater'
    | 'archived'
    | 'tag'
    | 'folder'
    | 'feed'
    | 'feedError';

/** The action a zero-state offers (the component binds each to the store). */
export type ListEmptyAction = 'showRead' | 'refreshFeed' | 'retryFeed' | 'feedHealth';

/** A list zero-state: its medallion, copy and offered actions. */
export interface ListEmptyState {
    readonly kind: ListEmptyKind;
    readonly icon: IconName;
    readonly title: MessageKey;
    /** The secondary line; a `{key}` in it is the `hintKey` shortcut. */
    readonly hint: MessageKey | null;
    /** The shortcut a hint names, printed from the registry (never hard-coded). */
    readonly hintKey: ShortcutId | null;
    readonly actions: readonly ListEmptyAction[];
}

function state(
    kind: ListEmptyKind,
    icon: IconName,
    title: MessageKey,
    hint: MessageKey | null = null,
    hintKey: ShortcutId | null = null,
    actions: readonly ListEmptyAction[] = [],
): ListEmptyState {
    return { kind, icon, title, hint, hintKey, actions };
}

/**
 * Whether a feed's last fetch failed in a way worth explaining. `busy` is not
 * a failure — another Curio process was refreshing it — and a paused or dead
 * feed already says so in the sidebar.
 */
export function feedHasError(feed: FeedDto): boolean {
    return feed.last_error !== null && feed.last_error_kind !== 'busy';
}

/**
 * The zero-state for an empty list over `filters`. `feed` is the selected
 * feed (when the view is scoped to one), so a broken feed explains itself.
 * The order is most specific first: a failing feed, then unread-only (its
 * "all caught up" holds in any view), then the view dimensions.
 */
export function listEmptyState(filters: ArticleFilters, feed: FeedDto | null): ListEmptyState {
    if (feed !== null && filters.feedId === feed.id && feedHasError(feed)) {
        return state('feedError', 'alert', 'list.empty.feedError', null, null, [
            'retryFeed',
            'feedHealth',
        ]);
    }
    if (filters.read === false) {
        return state('unread', 'check', 'list.empty.unread', null, null, ['showRead']);
    }
    if (filters.starred === true) {
        return state(
            'starred',
            'star',
            'list.empty.starred',
            'list.empty.starred.hint',
            'article.toggleStar',
        );
    }
    if (filters.readLater === true) {
        return state(
            'readLater',
            'bookmark',
            'list.empty.readLater',
            'list.empty.readLater.hint',
            'article.toggleReadLater',
        );
    }
    if (filters.archived === true) {
        return state(
            'archived',
            'archive',
            'list.empty.archived',
            'list.empty.archived.hint',
            'article.toggleArchived',
        );
    }
    if (filters.tag !== null) {
        return state('tag', 'tag', 'list.empty');
    }
    if (filters.feedTag !== null) {
        return state('folder', 'folder', 'list.empty');
    }
    if (filters.feedId !== null) {
        return state('feed', 'rss', 'list.empty.feed', 'list.empty.feed.hint', null, [
            'refreshFeed',
        ]);
    }
    return state('all', 'inbox', 'list.empty.all', 'list.empty.all.hint');
}

/** Copy for a failure class that needs no params. */
const FEED_ERROR_COPY: Partial<Record<FetchErrorKindDto, MessageKey>> = {
    not_a_feed: 'feedError.notAFeed',
    parse: 'feedError.parse',
    gone: 'feedError.gone',
    http: 'feedError.http',
    rate_limited: 'feedHealth.hint.rateLimited',
};

/** Copy for a failure class whose line names the feed's host. */
const FEED_ERROR_HOST_COPY: Partial<Record<FetchErrorKindDto, MessageKey>> = {
    dns: 'error.hostNotFound',
    refused: 'error.connectionFailed',
    reset: 'error.connectionFailed',
    transport: 'error.connectionFailed',
    redirect: 'error.connectionFailed',
    tls_certificate: 'error.tls',
    tls_handshake: 'error.tls',
    connect_timeout: 'error.networkTimeout',
    read_timeout: 'error.networkTimeout',
    blocked: 'error.blockedAddress',
    too_large: 'error.tooLarge',
};

/**
 * Why `feed`'s last fetch failed, in the active locale, from its classified
 * `last_error_kind` — never the engine's English `last_error` text. A failure
 * with no class gets the generic line.
 */
export function feedErrorMessage(feed: FeedDto): string {
    const kind = feed.last_error_kind;
    if (kind === null) {
        return t('feedError.generic');
    }
    if (kind === 'invalid_url') {
        return t('error.invalidUrl', { input: feed.url });
    }
    const plain = FEED_ERROR_COPY[kind];
    if (plain !== undefined) {
        return t(plain);
    }
    const withHost = FEED_ERROR_HOST_COPY[kind];
    return withHost === undefined
        ? t('feedError.generic')
        : t(withHost, { host: hostOf(feed.url) ?? feed.url });
}

/**
 * Splits a translated line around its `{key}` slot so the component can set
 * the shortcut in a `<kbd>`: `[before, after]`, or `[text, null]` when the
 * line names no key.
 */
export function splitAtKey(key: MessageKey): readonly [string, string | null] {
    const marker = '\u0000';
    const text = t(key, { key: marker });
    const at = text.indexOf(marker);
    return at === -1 ? [text, null] : [text.slice(0, at), text.slice(at + marker.length)];
}

/** What the first-run gate reads. */
export interface WelcomeInput {
    readonly feedsLoaded: boolean;
    readonly feedsFailed: boolean;
    readonly feedCount: number;
    readonly filters: ArticleFilters;
    readonly listLoaded: boolean;
    readonly listCount: number;
    readonly searching: boolean;
}

/**
 * Whether the welcome view replaces the list and reader: no subscriptions,
 * the plain All view, and that view settled empty. Waiting for the list (not
 * just the feeds) keeps a library of imported saves — articles with no feed
 * — on screen instead of hiding it behind onboarding; any other view (Starred,
 * Archived, unread-only, a search) keeps its own zero-state.
 */
export function shouldWelcome(input: WelcomeInput): boolean {
    return (
        input.feedsLoaded &&
        !input.feedsFailed &&
        input.feedCount === 0 &&
        !input.searching &&
        filterKey(input.filters) === filterKey(ALL_ARTICLES) &&
        input.listLoaded &&
        input.listCount === 0
    );
}

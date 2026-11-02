/**
 * The ⌘K command palette's catalogue and ranking. Everything the palette can
 * do is a {@link PaletteItem} built from live store state — the shortcut
 * registry's commands (with the keys that run them), the built-in views,
 * every feed and folder, the settings tabs and the themes — and each item's
 * `run` goes through the same action layer a key press or a click does, so
 * the palette can never do something the rest of the app can't.
 *
 * Pure apart from reading the stores: the component wraps
 * {@link paletteItems} in a `$derived`, so a feed added while the palette is
 * open shows up in it.
 */
import { type MessageKey, displayName, t } from '$lib/i18n';
import { shortcutKbd } from '$lib/keyboard/labels';
import { SHORTCUTS, type ShortcutId } from '$lib/keyboard/registry';
import { isLinkless } from '$lib/utils/article';
import { foldText, fuzzyMatch } from '$lib/utils/fuzzy';
import { type ViewId, activeView, handleShortcut, selectFolder } from './actions';
import { articlesStore } from './articles.svelte';
import { type FeedFolder, buildFeedTree, tagSegments } from './feed-tree';
import { feedsStore } from './feeds.svelte';
import { SETTINGS_SECTIONS, SETTINGS_SECTION_LABELS, layoutStore } from './layout.svelte';
import { searchStore } from './search.svelte';
import { selectionStore } from './selection.svelte';
import { THEMES, type ThemePreference, themeStore } from './theme.svelte';

/** What kind of thing an item is — its tag in the list, and its rank on ties. */
export type PaletteGroup = 'command' | 'view' | 'feed' | 'folder' | 'settings' | 'theme';

/** Display order of the groups (and the tie-break between equal scores). */
export const PALETTE_GROUPS: readonly PaletteGroup[] = [
    'command',
    'view',
    'feed',
    'folder',
    'settings',
    'theme',
];

/** Each group's tag. */
export const PALETTE_GROUP_LABELS: Readonly<Record<PaletteGroup, MessageKey>> = {
    command: 'palette.group.command',
    view: 'palette.group.view',
    feed: 'palette.group.feed',
    folder: 'palette.group.folder',
    settings: 'palette.group.settings',
    theme: 'palette.group.theme',
};

/** One thing the palette can run. */
export interface PaletteItem {
    /** Unique across the palette (`command:app.addFeed`, `feed:12`). */
    readonly id: string;
    readonly group: PaletteGroup;
    /** What the row says, and what the query matches. */
    readonly label: string;
    /** Secondary text (a feed's folders), never matched. */
    readonly detail?: string;
    /** The key binding that runs it outside the palette, printed beside it. */
    readonly kbd?: string;
    /** It is what's showing now (the open view, feed or folder; the theme). */
    readonly current?: boolean;
    /** Runs it, once the palette has closed. */
    readonly run: () => void;
}

/** A ranked item: the item plus the label characters the query matched. */
export interface RankedItem {
    readonly item: PaletteItem;
    /** Matched positions in `Array.from(item.label)`, for highlighting. */
    readonly indices: readonly number[];
}

/** The most rows the palette renders for a query (the best ones). */
export const PALETTE_LIMIT = 50;

/**
 * The registry commands offered as palette commands, in display order: the
 * things worth running by name. The row-by-row navigation keys (j/k, the
 * selection extenders, paging, pane focus) and the palette itself are left
 * out — they only make sense as keys.
 */
const COMMANDS: readonly ShortcutId[] = [
    'app.addFeed',
    'app.refreshAll',
    'feed.refresh',
    'view.markAllRead',
    'nav.nextUnread',
    'search.focus',
    'article.toggleRead',
    'article.toggleStar',
    'article.toggleReadLater',
    'article.toggleArchived',
    'article.open',
    'article.promote',
    'view.feeds',
    'view.toggleSidebar',
    'view.toggleFocusMode',
    'app.settings',
    'help.toggle',
];

/** The built-in views, each with the shortcut that opens it. */
const VIEWS: readonly { readonly view: ViewId; readonly shortcut: ShortcutId }[] = [
    { view: 'all', shortcut: 'view.all' },
    { view: 'starred', shortcut: 'view.starred' },
    { view: 'readLater', shortcut: 'view.readLater' },
    { view: 'archived', shortcut: 'view.archived' },
];

/** The registry's description of `id` (every listed id is in the table). */
function describe(id: ShortcutId): string {
    const shortcut = SHORTCUTS.find((candidate) => candidate.id === id);
    return shortcut === undefined ? id : t(shortcut.description);
}

/**
 * Whether a command applies right now: the article commands need a selected
 * article (opening and promoting need exactly one, with a page of its own to
 * open), refreshing "the selected feed" needs one.
 */
function available(id: ShortcutId): boolean {
    switch (id) {
        case 'article.toggleRead':
        case 'article.toggleStar':
        case 'article.toggleReadLater':
        case 'article.toggleArchived':
            return selectionStore.selectedArticleId !== null;
        case 'article.promote':
            return selectionStore.selectedArticleId !== null && !selectionStore.multiActive;
        case 'article.open': {
            const selected = selectionStore.selectedArticle;
            return (
                selected !== null &&
                !selectionStore.multiActive &&
                !isLinkless(selected, feedsStore.feeds)
            );
        }
        case 'feed.refresh':
            return selectionStore.selectedFeedId !== null;
        default:
            return true;
    }
}

function commandItems(): PaletteItem[] {
    return COMMANDS.filter(available).map((id) => ({
        id: `command:${id}`,
        group: 'command',
        label: describe(id),
        kbd: shortcutKbd(id),
        run: () => handleShortcut(id),
    }));
}

/**
 * Leaves focus mode before showing a list: picking a view, feed or folder
 * from the palette is asking to see it, and focus mode hides the list.
 */
function showList(): void {
    void layoutStore.revealList();
}

function viewItems(): PaletteItem[] {
    const open = searchStore.active ? null : activeView(articlesStore.filters);
    return VIEWS.map(({ view, shortcut }) => ({
        id: `view:${view}`,
        group: 'view',
        label: describe(shortcut),
        kbd: shortcutKbd(shortcut),
        current: open === view,
        run: () => {
            showList();
            handleShortcut(shortcut);
        },
    }));
}

/** A `/`-path tag as the sidebar nests it: `Tech / Databases`. */
function folderLabel(path: string): string {
    return tagSegments(path).join(' / ');
}

function feedItems(): PaletteItem[] {
    const openFeed = searchStore.active ? null : selectionStore.selectedFeedId;
    return feedsStore.feeds.map((feed) => {
        const folders = feed.tags.map(folderLabel).filter((label) => label.length > 0);
        return {
            id: `feed:${feed.id}`,
            group: 'feed',
            label: feed.title ?? feed.url,
            detail: folders.length > 0 ? folders.join(', ') : undefined,
            current: openFeed === feed.id,
            run: () => {
                showList();
                selectionStore.selectFeed(feed.id);
            },
        };
    });
}

/** Every folder of the tree, parents before their subfolders. */
function allFolders(folders: readonly FeedFolder[], into: FeedFolder[] = []): FeedFolder[] {
    for (const folder of folders) {
        into.push(folder);
        allFolders(folder.subfolders, into);
    }
    return into;
}

function folderItems(): PaletteItem[] {
    const tree = buildFeedTree(feedsStore.feeds, feedsStore.pendingPaths);
    const openFolder = searchStore.active ? undefined : articlesStore.filters.feedTag;
    return allFolders(tree.folders).map((folder) => ({
        id: `folder:${folder.path}`,
        group: 'folder',
        label: folderLabel(folder.path),
        current: openFolder === folder.path,
        run: () => {
            showList();
            selectFolder(folder.path);
        },
    }));
}

function settingsItems(): PaletteItem[] {
    return SETTINGS_SECTIONS.map((section) => ({
        id: `settings:${section}`,
        group: 'settings',
        label: t(SETTINGS_SECTION_LABELS[section]),
        run: () => layoutStore.openSettings(section),
    }));
}

function themeItems(): PaletteItem[] {
    const themes: { id: ThemePreference; name: string }[] = [
        { id: 'system', name: t('theme.system') },
        ...THEMES.map((theme) => ({ id: theme.id, name: displayName(theme.name) })),
        ...themeStore.customThemes.map((theme) => ({
            id: `custom-${theme.id}` as const,
            name: theme.name,
        })),
    ];
    return themes.map(({ id, name }) => ({
        id: `theme:${id}`,
        group: 'theme',
        label: name,
        current: themeStore.themePreference === id,
        run: () => void themeStore.setThemePreference(id),
    }));
}

/** Everything the palette offers right now, in group order. */
export function paletteItems(): PaletteItem[] {
    return [
        ...commandItems(),
        ...viewItems(),
        ...feedItems(),
        ...folderItems(),
        ...settingsItems(),
        ...themeItems(),
    ];
}

/** Scored below any direct label match of the same text (see {@link matchItem}). */
const TAGGED_PENALTY = 1;

/**
 * How `query` matches `item`: against its label, or — so "theme nord" and
 * "folder tech" narrow to one group — as a first word that starts the
 * item's group tag followed by the rest matching the label, scored a little
 * below the same label match on its own. A one-word query never matches by
 * tag ("feed" must not list every feed). The indices are label positions.
 */
function matchItem(query: string, item: PaletteItem): RankedItem & { score: number } {
    const direct = fuzzyMatch(query, item.label);
    if (direct !== null) {
        return { item, indices: direct.indices, score: direct.score };
    }
    const [head, ...rest] = query.trim().split(/\s+/);
    const tag = foldText(t(PALETTE_GROUP_LABELS[item.group]));
    const tagged =
        rest.length > 0 && tag.startsWith(foldText(head))
            ? fuzzyMatch(rest.join(' '), item.label)
            : null;
    if (tagged === null) {
        return { item, indices: [], score: Number.NEGATIVE_INFINITY };
    }
    return { item, indices: tagged.indices, score: tagged.score - TAGGED_PENALTY };
}

/**
 * Ranks `items` for `query`: every item whose label (or group tag + label)
 * fuzzy-matches, best first, ties kept in group order and then catalogue
 * order. A blank query keeps the whole catalogue in order. At most `limit`
 * rows come back.
 */
export function rankPaletteItems(
    items: readonly PaletteItem[],
    query: string,
    limit: number = PALETTE_LIMIT,
): RankedItem[] {
    if (query.trim().length === 0) {
        return items.slice(0, limit).map((item) => ({ item, indices: [] }));
    }
    return items
        .map((item, order) => ({ ...matchItem(query, item), order }))
        .filter((ranked) => ranked.score !== Number.NEGATIVE_INFINITY)
        .sort(
            (a, b) =>
                b.score - a.score ||
                PALETTE_GROUPS.indexOf(a.item.group) - PALETTE_GROUPS.indexOf(b.item.group) ||
                a.order - b.order,
        )
        .slice(0, limit)
        .map(({ item, indices }) => ({ item, indices }));
}

/** `label` cut into plain and matched runs, for `<mark>`-ing the matches. */
export function highlightRuns(
    label: string,
    indices: readonly number[],
): { text: string; match: boolean }[] {
    const matched = new Set(indices);
    const runs: { text: string; match: boolean }[] = [];
    Array.from(label).forEach((char, index) => {
        const match = matched.has(index);
        const last = runs.at(-1);
        if (last !== undefined && last.match === match) {
            last.text += char;
        } else {
            runs.push({ text: char, match });
        }
    });
    return runs;
}

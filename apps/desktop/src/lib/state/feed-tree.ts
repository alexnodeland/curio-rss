/**
 * Feed-tree derivation: turns the flat `feeds.tags` list into a folder tree
 * for the sidebar. A tag is a `/`-joined path (`Tech/Databases`, produced by
 * OPML import or hand-entered), so folders are a *rendering* of tags — no
 * separate folder entity. A feed with several tags appears under several
 * folders (tags are a set, not a single location); a feed with no tags is
 * "ungrouped" and sits outside the hierarchy. Pure + synchronous so it is
 * unit-tested without IPC; the sidebar wraps it in a `$derived`.
 */
import type { FeedDto } from '$lib/bindings';

export interface FeedFolder {
    /** The leaf segment name, e.g. `Databases`. */
    name: string;
    /** The full path from the root, e.g. `Tech/Databases`. */
    path: string;
    /** Nested subfolders, sorted by name. */
    subfolders: FeedFolder[];
    /** Feeds whose tag equals this folder's path exactly, in `sort_order`. */
    feeds: FeedDto[];
}

export interface FeedTree {
    /** Top-level folders, sorted by name. */
    folders: FeedFolder[];
    /** Feeds carrying no tags — rendered outside the folder hierarchy. */
    ungrouped: FeedDto[];
}

/** A tag split into its non-empty, trimmed path segments. */
export function tagSegments(tag: string): string[] {
    return tag
        .split('/')
        .map((segment) => segment.trim())
        .filter((segment) => segment.length > 0);
}

interface MutableFolder {
    name: string;
    path: string;
    subfolders: Map<string, MutableFolder>;
    feeds: FeedDto[];
}

function freeze(node: MutableFolder): FeedFolder {
    return {
        name: node.name,
        path: node.path,
        subfolders: [...node.subfolders.values()]
            .map(freeze)
            .sort((a, b) => a.name.localeCompare(b.name)),
        // Feeds preserve their incoming order (which is `sort_order`), so a
        // drag-reorder is visible; folders still sort by name above.
        feeds: [...node.feeds],
    };
}

/** Walks (creating as needed) the folder chain for one path, returning the leaf. */
function walkToLeaf(roots: Map<string, MutableFolder>, segments: string[]): MutableFolder | null {
    let level = roots;
    let prefix = '';
    let leaf: MutableFolder | null = null;
    for (const segment of segments) {
        prefix = prefix === '' ? segment : `${prefix}/${segment}`;
        let child = level.get(segment);
        if (child === undefined) {
            child = { name: segment, path: prefix, subfolders: new Map(), feeds: [] };
            level.set(segment, child);
        }
        leaf = child;
        level = child.subfolders;
    }
    return leaf;
}

/**
 * Buckets feeds into a folder tree keyed on their `/`-path tags. Building is
 * order-independent per feed, but each folder's feeds preserve the incoming
 * `feeds` array order (i.e. `sort_order`). `pendingPaths` overlays empty
 * folders (user-created, not yet holding any feed) so they are visible.
 */
export function buildFeedTree(feeds: FeedDto[], pendingPaths: string[] = []): FeedTree {
    const roots = new Map<string, MutableFolder>();
    const ungrouped: FeedDto[] = [];

    for (const path of pendingPaths) {
        const segments = tagSegments(path);
        if (segments.length > 0) {
            walkToLeaf(roots, segments);
        }
    }

    for (const feed of feeds) {
        const paths = feed.tags.map(tagSegments).filter((segments) => segments.length > 0);
        if (paths.length === 0) {
            ungrouped.push(feed);
            continue;
        }
        for (const segments of paths) {
            const leaf = walkToLeaf(roots, segments);
            leaf?.feeds.push(feed);
        }
    }

    return {
        folders: [...roots.values()].map(freeze).sort((a, b) => a.name.localeCompare(b.name)),
        ungrouped,
    };
}

/**
 * The distinct feed ids in a folder subtree — the deduped basis for a
 * folder's roll-up unread count (a feed under two subfolders counts once).
 */
export function subtreeFeedIds(folder: FeedFolder, into: Set<number> = new Set()): Set<number> {
    for (const feed of folder.feeds) {
        into.add(feed.id);
    }
    for (const sub of folder.subfolders) {
        subtreeFeedIds(sub, into);
    }
    return into;
}

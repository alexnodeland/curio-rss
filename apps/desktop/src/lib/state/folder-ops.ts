/**
 * Pure tag-path transforms behind the sidebar's folder operations. Folders are
 * a rendering of each feed's `/`-path tags (see feed-tree), so every folder
 * operation — move a feed in, rename a folder, delete a folder — is really a
 * per-feed tag rewrite. These functions compute one feed's new tag list; the
 * store applies them with `setFeedTags`. Kept pure so the (fiddly) prefix logic
 * is unit-tested without IPC.
 */
import type { FeedDto } from '$lib/bindings';
import { tagSegments } from './feed-tree';

function dedupe(tags: string[]): string[] {
    return [...new Set(tags.filter((tag) => tag.trim().length > 0))];
}

/** A tag is a folder path when it has at least one path segment. */
function isPathTag(tag: string): boolean {
    return tagSegments(tag).length > 0;
}

/**
 * The feed's tags after moving it into `folderPath`: replace its first path
 * tag with the folder path (preserving any other tags); if it has none, add it.
 */
export function tagsForMove(tags: string[], folderPath: string): string[] {
    const first = tags.findIndex(isPathTag);
    const next =
        first === -1
            ? [...tags, folderPath]
            : tags.map((tag, index) => (index === first ? folderPath : tag));
    return dedupe(next);
}

/** The feed's tags after dropping it on "ungrouped": drop every path tag. */
export function tagsForUngroup(tags: string[]): string[] {
    return dedupe(tags.filter((tag) => !isPathTag(tag)));
}

/** The feed's tags after renaming folder `oldPath` → `newPath` (prefix rewrite). */
export function tagsForRename(tags: string[], oldPath: string, newPath: string): string[] {
    return dedupe(
        tags.map((tag) => {
            if (tag === oldPath) return newPath;
            if (tag.startsWith(`${oldPath}/`)) return `${newPath}${tag.slice(oldPath.length)}`;
            return tag;
        }),
    );
}

/**
 * The feed's tags after deleting folder `path` — its feeds move to the parent
 * folder (never unsubscribed). A top-level folder's feeds become ungrouped.
 */
export function tagsForDelete(tags: string[], path: string): string[] {
    const parent = path.split('/').slice(0, -1).join('/');
    return dedupe(
        tags.flatMap((tag) => {
            if (tag !== path && !tag.startsWith(`${path}/`)) return [tag];
            const relocated = tagSegments(`${parent}${tag.slice(path.length)}`).join('/');
            return relocated.length > 0 ? [relocated] : [];
        }),
    );
}

/** Whether a feed belongs to folder `path` (exactly, or a descendant of it). */
export function feedInFolder(feed: FeedDto, path: string): boolean {
    return feed.tags.some((tag) => tag === path || tag.startsWith(`${path}/`));
}

/** Every distinct folder path across all feeds (each tag and its ancestors). */
export function allFolderPaths(feeds: FeedDto[]): string[] {
    const paths = new Set<string>();
    for (const feed of feeds) {
        for (const tag of feed.tags) {
            let prefix = '';
            for (const segment of tagSegments(tag)) {
                prefix = prefix === '' ? segment : `${prefix}/${segment}`;
                paths.add(prefix);
            }
        }
    }
    return [...paths].sort((a, b) => a.localeCompare(b));
}

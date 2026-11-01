/**
 * Neighbour prefetch for the reader: while one article is open, the bodies
 * (and flags) of the rows around it in the backend-ordered window are loaded
 * into the query cache, so j/k onto a neighbour renders the body straight
 * from cache instead of a skeleton.
 *
 * The prefetcher owns only the body queries it created itself, and drops each
 * one once the selection has moved more than `PREFETCH_RADIUS` rows away from
 * it — every live detail query is refetched on each `ArticlesChanged`, so an
 * unbounded trail of prefetched bodies would make every mark-read fan out
 * across the whole scan history. Queries created by anyone else (the reader
 * for a directly selected article, list rows for flags) are never dropped.
 */
import { commands } from '$lib/bindings';
import { untrack } from 'svelte';
import { type Query, dropQuery, ensureQuery, hasQuery, queryKeys } from './query-cache.svelte';

/** How many rows either side of the selection get their body prefetched. */
export const PREFETCH_RADIUS = 2;

/**
 * The ids to prefetch around `center` in `items`, nearest first and forward
 * before backward at each distance (j is the common direction). The selected
 * id itself is excluded; when the selected row has left the window (an
 * auto-mark-read dropped it from an unread view), `center` is its last
 * position, so the row that shifted into that slot is included as the very
 * next one j will land on.
 */
export function neighbourIds(
    items: readonly { id: number }[],
    center: number,
    selectedId: number | null,
    radius: number = PREFETCH_RADIUS,
): number[] {
    if (items.length === 0 || center < 0) {
        return [];
    }
    const offsets = [0];
    for (let distance = 1; distance <= radius; distance += 1) {
        offsets.push(distance, -distance);
    }
    const ids: number[] = [];
    for (const offset of offsets) {
        const id = items[center + offset]?.id;
        if (id !== undefined && id !== selectedId && !ids.includes(id)) {
            ids.push(id);
        }
    }
    return ids;
}

/** Tracks the body queries it created so it can evict them out of range. */
export class ArticlePrefetcher {
    /** id → the exact query instance this prefetcher created. */
    #owned = new Map<number, Query<unknown>>();

    /**
     * Ensures body + flag queries for `ids`, then drops the bodies this
     * prefetcher created that are neither in `ids` nor the selected article.
     * Runs untracked: registry reads here are bookkeeping, not dependencies
     * of the calling effect (which would otherwise re-run on its own drops).
     */
    prefetch(ids: readonly number[], selectedId: number | null): void {
        untrack(() => {
            for (const id of ids) {
                this.#ensure(id);
            }
            this.#evict(ids, selectedId);
        });
    }

    #ensure(id: number): void {
        const key = queryKeys.article(id);
        const created = !hasQuery(key);
        const query = ensureQuery(key, () => commands.getArticle(id));
        if (created) {
            this.#owned.set(id, query as Query<unknown>);
        }
        ensureQuery(queryKeys.articleState(id), () => commands.getArticleState(id));
    }

    #evict(keep: readonly number[], selectedId: number | null): void {
        for (const [id, query] of [...this.#owned]) {
            if (id !== selectedId && !keep.includes(id)) {
                // Instance-checked: if the entry was dropped and re-created by
                // someone else meanwhile, it is theirs now — leave it alone.
                dropQuery(queryKeys.article(id), query);
                this.#owned.delete(id);
            }
        }
    }

    /** The ids whose body query this prefetcher currently owns (tests). */
    get owned(): number[] {
        return [...this.#owned.keys()];
    }

    /** Test isolation. */
    reset(): void {
        this.#owned.clear();
    }
}

/** The app-wide singleton the reader drives. */
export const articlePrefetcher = new ArticlePrefetcher();

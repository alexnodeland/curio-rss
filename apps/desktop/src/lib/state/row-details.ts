/**
 * List rows carry each article's flags and tags (`ArticleSummaryDto.state` /
 * `.tags`), so a page that lands lands the per-article detail entries the
 * rows, the reader and the prefetcher read: a page renders from its one
 * `list_articles` (or `search_articles`) call, with no `get_article_state` /
 * `get_article_tags` call per row — and an `ArticlesChanged` that refetches
 * the list refreshes its rows' entries through that one call too.
 */
import { type ArticleSummaryDto, commands } from '$lib/bindings';
import { type CarriedEntry, landCarried, queryKeys } from './query-cache.svelte';

/** The detail entries a page of list rows carries whole. */
export function rowDetails(rows: readonly ArticleSummaryDto[]): CarriedEntry[] {
    return rows.flatMap(({ id, state, tags }) => [
        {
            key: queryKeys.articleState(id),
            fetcher: () => commands.getArticleState(id),
            data: state,
        },
        {
            key: queryKeys.articleTags(id),
            fetcher: () => commands.getArticleTags(id),
            data: tags,
        },
    ]);
}

/** Lands a page of rows' details (a `loadMore` page, a search result). */
export function landRowDetails(rows: readonly ArticleSummaryDto[]): void {
    landCarried(rowDetails(rows));
}

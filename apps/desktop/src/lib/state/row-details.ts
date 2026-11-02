/**
 * List rows carry each article's flags and tags (`ArticleSummaryDto.state` /
 * `.tags`), so a page that lands seeds the per-article detail entries the
 * rows, the reader and the prefetcher read: a page renders from its one
 * `list_articles` (or `search_articles`) call, with no `get_article_state` /
 * `get_article_tags` call per row. Entries that already exist are left to
 * their own invalidation.
 */
import { type ArticleSummaryDto, commands } from '$lib/bindings';
import { queryKeys, seedQuery } from './query-cache.svelte';

export function seedRowDetails(rows: readonly ArticleSummaryDto[]): void {
    for (const row of rows) {
        const id = row.id;
        seedQuery(queryKeys.articleState(id), () => commands.getArticleState(id), row.state);
        seedQuery(queryKeys.articleTags(id), () => commands.getArticleTags(id), row.tags);
    }
}

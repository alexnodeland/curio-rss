/**
 * Article display facts the core encodes in its data rather than in flags —
 * pure functions over the wire DTOs, shared by the list row, the reader pane
 * and the keyboard actions so each surface reads them the same way.
 */
import type { ArticleSummaryDto, FeedDto } from '$lib/bindings';

/**
 * Whether the article is a link-less feed entry. The published frontmatter
 * contract requires every article to carry a source URI, so core stores the
 * feed's own URL as `source_url` when an entry has no `<link>` (and never
 * hydrates it). Such an article has no page of its own: opening its "source"
 * would open the raw feed XML. Matched exactly, against the feed's current
 * URL — the same test core applies. An article whose feed is unknown (not
 * loaded yet, or a manual save with no feed) is not link-less.
 */
export function isLinkless(
    article: Pick<ArticleSummaryDto, 'feed_id' | 'source_url'>,
    feeds: readonly Pick<FeedDto, 'id' | 'url'>[],
): boolean {
    const feedId = article.feed_id;
    if (feedId === null) {
        return false;
    }
    const feed = feeds.find((candidate) => candidate.id === feedId);
    return feed !== undefined && feed.url === article.source_url;
}

/** The date an article is shown under, and whether it is a first-seen stand-in. */
export interface ArticleDate {
    /** RFC 3339 timestamp to display. */
    iso: string;
    /**
     * True when the source declared no date, so `iso` is when Curio first
     * stored the article — it must not read as a publication date.
     */
    firstSeen: boolean;
}

/**
 * The publication date, or — when the feed gave neither a published nor an
 * updated time (`published_at` stays null by contract: "null if unknown") —
 * the first-seen `saved_at`, flagged so the UI can mark it as such.
 */
export function articleDate(
    article: Pick<ArticleSummaryDto, 'published_at' | 'saved_at'>,
): ArticleDate {
    return article.published_at === null
        ? { iso: article.saved_at, firstSeen: true }
        : { iso: article.published_at, firstSeen: false };
}

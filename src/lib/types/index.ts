// Core types matching Rust backend

export type Uuid = string;

export interface Feed {
    id: Uuid;
    url: string;
    title: string;
    description: string | null;
    site_url: string | null;
    icon_url: string | null;
    view_mode: ViewMode;
    tags: string[];
    folder_id: Uuid | null;
    etag: string | null;
    last_modified: string | null;
    last_fetched: string | null;
    last_error: string | null;
    refresh_interval: number | null;
    notify_new: boolean;
    position: number;
    created_at: string;
    updated_at: string;
}

export type ViewMode = 'article' | 'reddit' | 'youtube' | 'podcast' | 'gallery';

export interface Article {
    id: Uuid;
    feed_id: Uuid;
    guid: string;
    title: string;
    url: string | null;
    content_html: string | null;
    content_text: string | null;
    summary: string | null;
    author: string | null;
    thumbnail_url: string | null;
    media: MediaAttachment[];
    published_at: string | null;
    updated_at: string | null;
    fetched_at: string;
    is_read: boolean;
    is_starred: boolean;
    is_read_later: boolean;
    is_archived: boolean;
    read_at: string | null;
    read_later_at: string | null;
    read_later_position: number | null;
    reddit_score: number | null;
    reddit_num_comments: number | null;
    youtube_duration: number | null;
    podcast_duration: number | null;
    podcast_progress: number | null;
}

export interface MediaAttachment {
    url: string;
    mime_type: string;
    size_bytes: number | null;
    title: string | null;
    is_downloaded: boolean;
    local_path: string | null;
}

export interface Folder {
    id: Uuid;
    name: string;
    parent_id: Uuid | null;
    icon: string | null;
    color: string | null;
    view_mode: ViewMode | null;
    position: number;
    created_at: string;
    updated_at: string;
}

export interface FolderNode {
    folder: Folder;
    children: FolderNode[];
    feeds: FolderFeedItem[];
    unread_count: number;
}

export interface FolderFeedItem {
    id: Uuid;
    title: string;
    icon_url: string | null;
    unread_count: number;
    position: number;
}

export interface ArticleFilter {
    feed_id?: Uuid | null;
    folder_id?: Uuid | null;
    unread_only?: boolean;
    starred_only?: boolean;
    read_later_only?: boolean;
    archived?: boolean;
    tags?: string[] | null;
    search_query?: string | null;
    published_after?: string | null;
    published_before?: string | null;
}

export interface Pagination {
    page: number;
    per_page: number;
}

export interface ArticlePage {
    items: Article[];
    total: number;
    page: number;
    per_page: number;
    total_pages: number;
}

export interface FeedUpdate {
    title?: string;
    description?: string;
    site_url?: string;
    icon_url?: string;
    view_mode?: ViewMode;
    tags?: string[];
    folder_id?: Uuid | null;
    refresh_interval?: number | null;
    notify_new?: boolean;
    position?: number;
}

export interface RefreshResult {
    feed_id: Uuid;
    new_count: number;
    updated_count: number;
    unchanged: boolean;
    error: string | null;
}

export interface CommandError {
    message: string;
    code: ErrorCode;
    recoverable: boolean;
}

export type ErrorCode =
    | 'NOT_FOUND'
    | 'INVALID_INPUT'
    | 'NETWORK_ERROR'
    | 'RATE_LIMITED'
    | 'PARSE_ERROR'
    | 'STORAGE_ERROR'
    | 'EXTERNAL_SERVICE_ERROR'
    | 'DUPLICATE_ENTRY';

// Reddit types
export interface RedditPost {
    reddit_id: string;
    subreddit: string;
    author: string;
    author_flair: string | null;
    title: string;
    score: number;
    upvote_ratio: number;
    num_comments: number;
    permalink: string;
    is_self: boolean;
    selftext_html: string | null;
    link_url: string | null;
    thumbnail: string | null;
    created_utc: number;
    flair_text: string | null;
}

export interface RedditComment {
    reddit_id: string;
    author: string;
    author_flair: string | null;
    body_html: string;
    score: number;
    created_utc: number;
    depth: number;
    replies: RedditComment[];
    is_collapsed: boolean;
}

export interface RedditThread {
    post: RedditPost;
    comments: RedditComment[];
}

export interface RedditUrlInfo {
    subreddit: string;
    post_id: string | null;
    is_comment: boolean;
}

export type CommentSort = 'best' | 'top' | 'new' | 'controversial' | 'old';

// YouTube types
export interface YouTubeMetadata {
    id: string;
    title: string;
    description: string;
    channel: string;
    channel_id: string;
    channel_url: string;
    channel_follower_count: number | null;
    upload_date: string;
    view_count: number | null;
    like_count: number | null;
    comment_count: number | null;
    duration: number;
    categories: string[];
    tags: string[];
    chapters: Chapter[] | null;
    thumbnails: Thumbnail[];
}

export interface Chapter {
    start_time: number;
    end_time: number;
    title: string;
}

export interface Thumbnail {
    url: string;
    width: number | null;
    height: number | null;
    id: string;
}

export interface YouTubeComment {
    id: string;
    text: string;
    author: string;
    author_id: string;
    author_thumbnail: string;
    author_is_uploader: boolean;
    parent: string | null;
    like_count: number | null;
    is_pinned: boolean;
    is_favorited: boolean;
    timestamp: number | null;
}

export interface YouTubeUrlInfo {
    video_id: string | null;
    channel_id: string | null;
    playlist_id: string | null;
    url_type: YouTubeUrlType;
}

export type YouTubeUrlType = 'video' | 'channel' | 'playlist' | 'unknown';

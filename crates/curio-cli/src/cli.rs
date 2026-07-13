//! The clap command tree — `curio`'s entire surface, declaratively.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

/// curio — a local-first RSS reader + read-later engine.
///
/// Feeds and articles live in a profile directory (`curio.toml`,
/// `curio.db`, `.curio/events/`); saved articles export as markdown notes
/// with `curio.frontmatter.v1` YAML into named destinations, and every
/// reading action appends to the `curio.events.v1` JSONL log.
#[derive(Debug, Parser)]
#[command(name = "curio", version, propagate_version = true)]
pub(crate) struct Cli {
    /// Profile directory (config, database, event log). Defaults to the
    /// platform data dir, e.g. ~/.local/share/curio.
    #[arg(long, global = true, env = "CURIO_PROFILE", value_name = "DIR")]
    pub(crate) profile: Option<PathBuf>,

    /// Emit one machine-readable JSON document on stdout instead of
    /// human text.
    #[arg(long, global = true)]
    pub(crate) json: bool,

    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Scaffold the profile directory: curio.toml, curio.db, events log.
    Init,
    /// Manage feed subscriptions.
    #[command(subcommand)]
    Feed(FeedCommand),
    /// Refresh one feed (by id, URL, or unique substring) or all
    /// active feeds, reporting new-article counts.
    Fetch {
        /// Feed to refresh; omit to refresh every active feed.
        feed: Option<String>,
    },
    /// List articles, newest rows first.
    List(ListArgs),
    /// Print one article as markdown (marks it read).
    Show {
        /// A unique fragment of the article's id (listings show the
        /// typable 8-character tail).
        id: String,
    },
    /// Record an open (emits article.opened) and launch $BROWSER on the
    /// article's source URL.
    Open {
        /// A unique fragment of the article's id (listings show the
        /// typable 8-character tail).
        id: String,
    },
    /// Star an article (emits article.starred).
    Star { id: String },
    /// Unstar an article (emits the article.unstarred negation).
    Unstar { id: String },
    /// Add an article to read-later (emits the read-later-added event).
    Later { id: String },
    /// Remove an article from read-later (emits the negation).
    Unlater { id: String },
    /// Archive an article (emits article.archived).
    Archive { id: String },
    /// Unarchive an article (emits the article.unarchived negation).
    Unarchive { id: String },
    /// Add a tag to an article (emits article.tagged).
    Tag { id: String, tag: String },
    /// Remove a tag from an article (emits the article.untagged negation).
    Untag { id: String, tag: String },
    /// Save a single URL as a feedless read-later article: the page is
    /// fetched through the policed client and readability-extracted
    /// (emits the read-later-added event). An unreachable page still
    /// saves the bare link. Re-clipping a known URL re-flags it without
    /// refetching.
    Clip {
        /// The http(s) URL to save.
        url: String,
        /// Tags for the saved article (repeat or comma-separate).
        #[arg(long, value_delimiter = ',')]
        tags: Vec<String>,
    },
    /// Export articles to a named destination as curio.frontmatter.v1
    /// markdown notes (emits article.saved / article.updated). Pass an
    /// article id for one note, or --all / a filter flag for a bulk
    /// export ("my library as markdown") — idempotent either way.
    Save {
        /// A unique fragment of one article's id (listings show the
        /// typable 8-character tail). Omit for a bulk export.
        id: Option<String>,
        /// Destination name; defaults to the default destination
        /// configured in curio.toml.
        #[arg(long, value_name = "NAME")]
        dest: Option<String>,
        /// Export the whole library.
        #[arg(long, conflicts_with = "id")]
        all: bool,
        /// Bulk: only read-later articles.
        #[arg(long, conflicts_with = "id")]
        read_later: bool,
        /// Bulk: only starred articles.
        #[arg(long, conflicts_with = "id")]
        starred: bool,
        /// Bulk: only articles from this feed (id, URL, or substring).
        #[arg(long, value_name = "FEED", conflicts_with = "id")]
        feed: Option<String>,
        /// Bulk: only articles carrying this tag.
        #[arg(long, value_name = "TAG", conflicts_with = "id")]
        tag: Option<String>,
    },
    /// Manage named export destinations.
    #[command(subcommand)]
    Dest(DestCommand),
    /// Import or export subscriptions as OPML 2.0.
    #[command(subcommand)]
    Opml(OpmlCommand),
    /// Import subscriptions or saved articles from a third-party export
    /// (OPML, or a Pocket / Instapaper / Readwise CSV). Feeds become
    /// subscriptions; saved articles become read-later items with their
    /// tags. Re-importing the same file is idempotent.
    Import {
        /// The export file to read.
        file: PathBuf,
        /// Which service the file came from.
        #[arg(long = "from", value_enum, default_value_t = ImportFormat::Opml)]
        from: ImportFormat,
    },
    /// Manage the optional Reddit API credentials (BYO OAuth, stored in
    /// the OS keychain). Without them Reddit enrichment works
    /// unauthenticated at ~10 requests/min; with your own free app from
    /// reddit.com/prefs/apps it runs at the 100/min tier.
    #[command(subcommand)]
    Reddit(RedditCommand),
    /// Inspect the curio.events.v1 log.
    #[command(subcommand)]
    Events(EventsCommand),
    /// Health checks: database integrity, FTS sync, events-log health.
    Doctor,
    /// Full-text search over stored articles (FTS5).
    Search {
        query: String,
        /// Maximum number of hits.
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum FeedCommand {
    /// Subscribe to a feed URL.
    Add {
        url: String,
        /// Tags for the subscription (repeat or comma-separate).
        #[arg(long, value_delimiter = ',')]
        tags: Vec<String>,
        /// Contract W1: exempt this feed from the SSRF private-network
        /// guard (e.g. a digest feed on 127.0.0.1). Recorded in
        /// curio.toml — an explicit config edit, never feed content.
        #[arg(long)]
        allow_private_network: bool,
    },
    /// List every subscription.
    List,
    /// Unsubscribe a feed (by id, URL, or unique substring). Articles
    /// keep their rows; emits the feed.removed negation.
    Rm { feed: String },
    /// Turn full-text mode on or off for a feed. When on, every refresh
    /// fetches each NEW article's source page and stores the
    /// readability-extracted full body (for feeds that ship excerpts).
    FullText {
        /// Feed to change (by id, URL, or unique substring).
        feed: String,
        /// `on` or `off`.
        #[arg(value_enum)]
        mode: OnOff,
    },
}

/// A human-friendly boolean for mode switches.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum OnOff {
    /// Enable.
    On,
    /// Disable.
    Off,
}

impl OnOff {
    pub(crate) fn as_bool(self) -> bool {
        matches!(self, Self::On)
    }
}

#[derive(Debug, Args)]
pub(crate) struct ListArgs {
    /// Only unread articles.
    #[arg(long)]
    pub(crate) unread: bool,
    /// Only starred articles.
    #[arg(long)]
    pub(crate) starred: bool,
    /// Only read-later articles.
    #[arg(long)]
    pub(crate) read_later: bool,
    /// Only articles carrying this tag.
    #[arg(long, value_name = "TAG")]
    pub(crate) tag: Option<String>,
    /// Only articles from this feed (id, URL, or unique substring).
    #[arg(long, value_name = "FEED")]
    pub(crate) feed: Option<String>,
    /// Maximum number of articles.
    #[arg(long, default_value_t = 20)]
    pub(crate) limit: u32,
}

#[derive(Debug, Subcommand)]
pub(crate) enum DestCommand {
    /// Register (or re-root) a named destination directory. This is the
    /// only place a destination path ever enters — every other command
    /// refers to destinations by name.
    Add { name: String, path: PathBuf },
    /// List registered destinations.
    List,
}

#[derive(Debug, Subcommand)]
pub(crate) enum OpmlCommand {
    /// Subscribe to every feed in an OPML file (already-known URLs are
    /// skipped; the folder path becomes one `/`-joined tag).
    Import { file: PathBuf },
    /// Write every subscription as OPML 2.0 ("-" for stdout).
    Export { file: PathBuf },
}

/// The third-party export format `curio import` parses.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum ImportFormat {
    /// OPML 2.0 subscription list.
    Opml,
    /// Pocket CSV export.
    Pocket,
    /// Instapaper CSV export.
    Instapaper,
    /// Readwise Reader CSV export.
    Readwise,
}

impl From<ImportFormat> for curio_core::ImportSource {
    fn from(format: ImportFormat) -> Self {
        match format {
            ImportFormat::Opml => Self::Opml,
            ImportFormat::Pocket => Self::PocketCsv,
            ImportFormat::Instapaper => Self::InstapaperCsv,
            ImportFormat::Readwise => Self::ReadwiseCsv,
        }
    }
}

#[derive(Debug, Subcommand)]
pub(crate) enum RedditCommand {
    /// Store your Reddit app's credentials in the OS keychain and start
    /// using the authenticated API. Create the (free) app at
    /// reddit.com/prefs/apps — type "script" works for Curio.
    Login {
        /// The app's client id.
        #[arg(long)]
        client_id: String,
        /// The app's client secret. Omit to read it from stdin instead
        /// of leaving it in your shell history.
        #[arg(long)]
        client_secret: Option<String>,
    },
    /// Show whether credentials are stored (never prints the secret).
    Status,
    /// Remove the stored credentials and go back to unauthenticated.
    Logout,
}

#[derive(Debug, Subcommand)]
pub(crate) enum EventsCommand {
    /// Print the last N events (debug view of the JSONL stream).
    Tail {
        /// How many events to show.
        #[arg(short = 'n', long = "lines", default_value_t = 20)]
        n: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_tree_is_well_formed() {
        use clap::CommandFactory as _;
        Cli::command().debug_assert();
    }
}

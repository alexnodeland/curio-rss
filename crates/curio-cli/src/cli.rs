//! The clap command tree — `curio`'s entire surface, declaratively.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

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
    /// Export an article to a named destination as a curio.frontmatter.v1
    /// markdown note (emits article.saved / article.updated).
    Save {
        /// A unique fragment of the article's id (listings show the
        /// typable 8-character tail).
        id: String,
        /// Destination name; defaults to the default destination
        /// configured in curio.toml.
        #[arg(long, value_name = "NAME")]
        dest: Option<String>,
    },
    /// Manage named export destinations.
    #[command(subcommand)]
    Dest(DestCommand),
    /// Import or export subscriptions as OPML 2.0.
    #[command(subcommand)]
    Opml(OpmlCommand),
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

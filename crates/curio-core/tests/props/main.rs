//! Property tests over the pure ingest entry points (issue #133) — one
//! suite per module, all in one test binary:
//!
//! | suite | entry point | properties |
//! |---|---|---|
//! | `feed_parse` | `feeds::parse_feed` | never panics; generated RSS / Atom / JSON Feed round-trip their items |
//! | `sanitize` | `content::sanitize` | output passes the inertness audit; idempotent |
//! | `extract` | `content::extract_main_content` | stays inert; never invents text |
//! | `plain_text` | `content::plain_text` | escaped text round-trips whitespace-normalized |
//! | `markdown` | `content::process` → `content::to_markdown` | always converts; exported markdown is inert |
//! | `opml` | `feeds::import_opml` / `export_opml` | never panics; export → import is lossless |
//! | `import_csv` | `import::parse` | never panics; generated Pocket exports round-trip |
//! | `url_screen` | `fetch::screen_url`, `fetch::policy::is_public_address` | only http(s) with a host and no userinfo passes; every IPv4-embedding IPv6 form is judged like its IPv4 |
//!
//! (The FTS query builder's suite lives next to it, in
//! `src/storage/repo/rows.rs`, since the builder is crate-private;
//! discovery's lives in the desktop head, `src-tauri/src/discovery.rs`.)

#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "../common/inert.rs"]
mod inert;

mod generators;

mod extract;
mod feed_parse;
mod import_csv;
mod markdown;
mod opml;
mod plain_text;
mod sanitize;
mod url_screen;

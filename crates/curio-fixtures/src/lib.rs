//! Deterministic seeded fixture generator for the Curio workspace.
//!
//! [`generate`] builds a `curio.db` of `spec.feeds` feeds and
//! `spec.articles` articles into a directory and returns a [`FixtureReport`]
//! (path, size, SHA-256, counts). The **determinism contract**
//! (docs/design/roadmap.md Phase 1) holds: the same [`FixtureSpec`] produces
//! a byte-identical database, so the hash is stable and assertable — proven
//! by [`tests::generation_is_byte_reproducible`].
//!
//! # How determinism is achieved
//!
//! The real write path is non-deterministic in exactly two places —
//! `CurioId::new()` (a `UUIDv7` carrying wall-clock time + OS randomness)
//! and `Timestamp::now()`. Both are replaced here with values derived only
//! from the seed and the row index, and rows are inserted in a fixed order.
//! The schema itself is the real migrated schema (obtained by opening a
//! throwaway [`CoreHandle`] so there is no DDL to keep in sync); rows are
//! then written through a raw [`rusqlite`] connection so every byte — the
//! `curio_id`, every timestamp, the FTS index the insert triggers build — is
//! controlled. A final `wal_checkpoint(TRUNCATE)` + `journal_mode = DELETE` +
//! `VACUUM` folds the WAL back in and normalises page layout, leaving one
//! self-contained, reproducible file.

use std::fs::File;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};

use curio_core::CoreHandle;
use curio_types::Timestamp;
use rusqlite::Connection;
use sha2::{Digest as _, Sha256};
use uuid::Builder;

/// The reproducible fixture is anchored at 2024-01-01T00:00:00Z so every
/// derived timestamp is a fixed offset from a fixed epoch.
const BASE_EPOCH_MILLIS: i64 = 1_704_067_200_000;

/// What to generate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixtureSpec {
    /// Seed for the deterministic PRNG — the sole source of variability.
    pub seed: u64,
    /// Number of feeds.
    pub feeds: usize,
    /// Number of articles (spread round-robin across the feeds).
    pub articles: usize,
}

impl FixtureSpec {
    /// The performance fixture the Phase-4 cold-start gate targets:
    /// 1000 feeds / 50 000 articles (docs/design/roadmap.md).
    #[must_use]
    pub const fn perf_50k() -> Self {
        Self {
            seed: 0x0C51_9A11_C0FF_EE00,
            feeds: 1_000,
            articles: 50_000,
        }
    }
}

/// The result of a generation run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureReport {
    /// Absolute path of the written `curio.db`.
    pub db_path: PathBuf,
    /// Lowercase hex SHA-256 of the database file.
    pub sha256: String,
    /// Size of the database file, in bytes.
    pub bytes: u64,
    /// Feeds written.
    pub feeds: usize,
    /// Articles written.
    pub articles: usize,
}

/// Everything that can go wrong while generating a fixture.
#[derive(Debug, thiserror::Error)]
pub enum GenError {
    /// Opening the throwaway handle that migrates the schema failed.
    #[error("could not create the migrated schema: {0}")]
    Schema(#[from] curio_core::CoreError),
    /// A raw database operation failed.
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// A filesystem operation failed.
    #[error("io at {path}: {source}")]
    Io {
        /// The path being touched.
        path: PathBuf,
        /// The underlying error.
        source: io::Error,
    },
}

/// Generates a deterministic fixture database into `dir`, creating `dir` if
/// absent, and returns its [`FixtureReport`].
///
/// The written file is `dir/curio.db`. Any pre-existing `curio.db` (and its
/// WAL sidecars and the `.curio` events directory a previous open created)
/// is removed first so the output depends only on `spec`.
///
/// # Errors
///
/// [`GenError`] on schema creation, database, or filesystem failure.
pub fn generate(dir: impl AsRef<Path>, spec: &FixtureSpec) -> Result<FixtureReport, GenError> {
    let dir = dir.as_ref();
    reset_dir(dir)?;

    // The real migrated schema, no DDL duplication: open then drop a handle,
    // which runs migrations and closes cleanly (the writer thread joins on
    // drop) before we take the file over ourselves.
    let db_path = {
        let handle = CoreHandle::open(dir)?;
        handle.storage().path().to_path_buf()
    };
    // Remove the events scaffolding the open created — the fixture is one file.
    remove_path(&dir.join(".curio"))?;

    let mut conn = Connection::open(&db_path)?;
    conn.pragma_update(None, "foreign_keys", true)?;
    write_rows(&mut conn, spec)?;
    finalize(&conn)?;
    drop(conn);
    remove_sidecars(&db_path)?;

    let (sha256, bytes) = hash_file(&db_path)?;
    Ok(FixtureReport {
        db_path,
        sha256,
        bytes,
        feeds: spec.feeds,
        articles: spec.articles,
    })
}

/// Inserts feeds then articles in a single transaction, in a fixed order.
fn write_rows(conn: &mut Connection, spec: &FixtureSpec) -> Result<(), GenError> {
    let mut rng = SplitMix64::new(spec.seed);
    let tx = conn.transaction()?;
    for f in 0..spec.feeds {
        insert_feed(&tx, f, &mut rng)?;
    }
    for a in 0..spec.articles {
        // Round-robin ownership keeps every feed populated; feed rowids are
        // 1-based (sequential inserts above).
        let feed_id = if spec.feeds == 0 {
            None
        } else {
            Some(i64::try_from(a % spec.feeds).unwrap_or(0) + 1)
        };
        insert_article(&tx, a, feed_id, &mut rng)?;
    }
    tx.commit()?;
    Ok(())
}

fn insert_feed(tx: &Connection, index: usize, rng: &mut SplitMix64) -> Result<(), GenError> {
    let n = index + 1;
    let id = i64::try_from(n).unwrap_or_default();
    let added = millis_string(raw_millis(i64::try_from(index).unwrap_or(0)));
    let host = ADJECTIVES[rng.below(ADJECTIVES.len())];
    let topic = NOUNS[rng.below(NOUNS.len())];
    // Deterministic lifecycle spread so feed-health surfaces have data.
    let status = match n % 20 {
        0 => "dead",
        5 | 15 => "paused",
        _ => "active",
    };
    tx.prepare_cached(
        "INSERT INTO feeds \
         (id, url, title, site_url, description, status, allow_private_network, \
          added_at, last_fetched_at, modified_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?7, ?7)",
    )?
    .execute((
        id,
        format!("https://{host}-{topic}-{n}.example.com/feed.xml"),
        format!("{} {} Weekly", capitalize(host), capitalize(topic)),
        format!("https://{host}-{topic}-{n}.example.com/"),
        format!("Dispatches on {topic} for the {host} reader."),
        status,
        &added,
    ))?;
    Ok(())
}

fn insert_article(
    tx: &Connection,
    index: usize,
    feed_id: Option<i64>,
    rng: &mut SplitMix64,
) -> Result<(), GenError> {
    let n = index + 1;
    let id = i64::try_from(n).unwrap_or_default();
    let idx = i64::try_from(index).unwrap_or(0);
    let published_millis = raw_millis(idx);
    let published = millis_string(published_millis);
    let (html, text, words) = body(rng);
    let title = format!(
        "{} {} {} #{n}",
        capitalize(ADJECTIVES[rng.below(ADJECTIVES.len())]),
        capitalize(NOUNS[rng.below(NOUNS.len())]),
        VERBS[rng.below(VERBS.len())],
    );
    let curio_id = deterministic_uuid_v7(published_millis, rng);
    let dedupe_key = match feed_id {
        Some(fid) => format!("f{fid}:guid-{index}"),
        None => format!("m:guid-{index}"),
    };
    tx.prepare_cached(
        "INSERT INTO articles \
         (id, curio_id, feed_id, dedupe_key, title, source_url, author, published_at, \
          content_html, content_text, lang, word_count, saved_at, source_updated_at, modified_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'en', ?11, ?12, ?8, ?12)",
    )?
    .execute((
        id,
        curio_id,
        feed_id,
        dedupe_key,
        title,
        format!("https://example.com/articles/{index}"),
        AUTHORS[rng.below(AUTHORS.len())],
        &published,
        html,
        text,
        i64::from(words),
        &published, // saved_at == published for reproducibility
    ))?;
    // A deterministic slice of state so unread counts and filters have signal.
    if n % 3 == 0 || n % 7 == 0 {
        tx.prepare_cached(
            "INSERT INTO article_state \
             (article_id, is_read, is_starred, is_read_later, is_archived, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?
        .execute((
            id,
            i64::from(n % 3 == 0),
            i64::from(n % 7 == 0),
            i64::from(n % 11 == 0),
            i64::from(n % 23 == 0),
            &published,
        ))?;
    }
    Ok(())
}

/// Folds the WAL back into the main file and normalises page layout so the
/// output is a single, byte-reproducible file.
fn finalize(conn: &Connection) -> Result<(), GenError> {
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
    // journal_mode is a query pragma (returns the new mode) — read it back.
    let _mode: String = conn.query_row("PRAGMA journal_mode = DELETE", [], |row| row.get(0))?;
    conn.execute_batch("VACUUM")?;
    Ok(())
}

// ------------------------------------------------------------- determinism

/// A deterministic `UUIDv7` built from an explicit millisecond timestamp and
/// ten seed-derived "random" bytes (never the OS RNG). Version 7 + the RFC
/// 4122 variant are set by the builder, so it round-trips through
/// `CurioId::from_str` on read.
fn deterministic_uuid_v7(millis: i64, rng: &mut SplitMix64) -> String {
    let mut rand_bytes = [0u8; 10];
    let a = rng.next_u64().to_le_bytes();
    let b = rng.next_u64().to_le_bytes();
    rand_bytes[..8].copy_from_slice(&a);
    rand_bytes[8..].copy_from_slice(&b[..2]);
    let millis_u64 = u64::try_from(millis).unwrap_or(0);
    Builder::from_unix_timestamp_millis(millis_u64, &rand_bytes)
        .into_uuid()
        .to_string()
}

/// Raw epoch millis for row `index`: one minute apart, anchored at the fixed
/// base so ordering is `id`-monotonic and every value is reproducible.
fn raw_millis(index: i64) -> i64 {
    BASE_EPOCH_MILLIS + index * 60_000
}

/// The RFC 3339 millisecond string the schema stores for a raw millis value.
/// `from_unix_millis` only returns `None` outside chrono's representable
/// range, which a fixed near-epoch base never reaches — the literal fallback
/// keeps the code panic-free under the workspace lints.
fn millis_string(millis: i64) -> String {
    match Timestamp::from_unix_millis(millis) {
        Some(ts) => ts.to_string(),
        None => "2024-01-01T00:00:00.000Z".to_owned(),
    }
}

/// `SplitMix64` — a tiny, fast, fully deterministic PRNG. No external crate,
/// so nothing new to license-triage.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        match u64::try_from(n) {
            Ok(0) | Err(_) => 0,
            Ok(modulus) => usize::try_from(self.next_u64() % modulus).unwrap_or(0),
        }
    }
}

// ------------------------------------------------------------------ corpus

const ADJECTIVES: &[&str] = &[
    "quiet", "bright", "hollow", "amber", "northern", "restless", "gilded", "distant", "crimson",
    "muted", "static", "lucid", "feral", "modular", "arctic", "velvet",
];
const NOUNS: &[&str] = &[
    "signal", "harbor", "lattice", "meridian", "cipher", "orchard", "foundry", "atlas", "cascade",
    "beacon", "quorum", "ledger", "prism", "delta", "kernel", "canopy",
];
const VERBS: &[&str] = &[
    "revisited",
    "reconsidered",
    "unpacked",
    "measured",
    "mapped",
    "traced",
    "rebuilt",
    "surveyed",
];
const AUTHORS: &[&str] = &[
    "R. Okoye",
    "M. Salcedo",
    "J. Vantapp",
    "L. Fenwick",
    "T. Aaltonen",
    "K. Devar",
    "P. Ito",
    "S. Marchetti",
];
const SENTENCES: &[&str] = &[
    "The system converged after a single deterministic pass.",
    "Every measurement was taken against the same fixed baseline.",
    "A keyset cursor advanced newest-first without re-sorting a thing.",
    "The index survived the vacuum because the rowids never moved.",
    "Nothing here touched the network; the corpus is entirely local.",
    "Latency stayed flat as the row count crossed six figures.",
    "The reader opened the page before the pane finished its transition.",
    "Backpressure from the writer thread never reached the interface.",
];

/// Assembles a small multi-paragraph HTML body and its plain-text mirror.
fn body(rng: &mut SplitMix64) -> (String, String, u32) {
    let paragraphs = 2 + rng.below(3); // 2..=4
    let mut html = String::new();
    let mut text = String::new();
    let mut words = 0u32;
    for _ in 0..paragraphs {
        let lines = 2 + rng.below(3); // 2..=4 sentences
        let mut para = String::new();
        for _ in 0..lines {
            let s = SENTENCES[rng.below(SENTENCES.len())];
            para.push_str(s);
            para.push(' ');
            words += u32::try_from(s.split_whitespace().count()).unwrap_or(0);
        }
        let para = para.trim_end();
        html.push_str("<p>");
        html.push_str(para);
        html.push_str("</p>\n");
        text.push_str(para);
        text.push('\n');
    }
    (html, text.trim_end().to_owned(), words)
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

// -------------------------------------------------------------- filesystem

fn reset_dir(dir: &Path) -> Result<(), GenError> {
    std::fs::create_dir_all(dir).map_err(|source| GenError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    remove_path(&dir.join(".curio"))?;
    let db = dir.join("curio.db");
    remove_sidecars(&db)?;
    remove_path(&db)?;
    Ok(())
}

fn remove_sidecars(db_path: &Path) -> Result<(), GenError> {
    for ext in ["-wal", "-shm"] {
        let mut sidecar = db_path.as_os_str().to_os_string();
        sidecar.push(ext);
        remove_path(Path::new(&sidecar))?;
    }
    Ok(())
}

fn remove_path(path: &Path) -> Result<(), GenError> {
    let result = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    match result {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(GenError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn hash_file(path: &Path) -> Result<(String, u64), GenError> {
    let mut file = File::open(path).map_err(|source| GenError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8_192];
    let mut total = 0u64;
    loop {
        let read = file.read(&mut buf).map_err(|source| GenError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
        total += u64::try_from(read).unwrap_or_default();
    }
    Ok((hex(&hasher.finalize()), total))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        // Writing to a String is infallible.
        let _ = write!(out, "{b:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use curio_core::storage::ListArticles;

    fn small() -> FixtureSpec {
        FixtureSpec {
            seed: 0xDEAD_BEEF,
            feeds: 8,
            articles: 240,
        }
    }

    #[test]
    fn generation_is_byte_reproducible() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let ra = generate(a.path(), &small()).unwrap();
        let rb = generate(b.path(), &small()).unwrap();
        assert_eq!(ra.sha256, rb.sha256, "same seed must yield the same bytes");
        assert_eq!(ra.bytes, rb.bytes);
        assert_eq!(ra.sha256.len(), 64);
    }

    #[test]
    fn regenerating_in_place_is_stable() {
        // The blob-guard-respecting `just fixtures` overwrites in place; a
        // second run over the same dir must reproduce the first byte for byte.
        let dir = tempfile::tempdir().unwrap();
        let first = generate(dir.path(), &small()).unwrap();
        let second = generate(dir.path(), &small()).unwrap();
        assert_eq!(first.sha256, second.sha256);
    }

    #[test]
    fn different_seeds_diverge() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let ra = generate(a.path(), &small()).unwrap();
        let rb = generate(
            b.path(),
            &FixtureSpec {
                seed: 0x1234,
                ..small()
            },
        )
        .unwrap();
        assert_ne!(ra.sha256, rb.sha256);
    }

    #[test]
    fn the_fixture_opens_as_a_real_core_and_has_the_requested_rows() {
        let dir = tempfile::tempdir().unwrap();
        let report = generate(dir.path(), &small()).unwrap();
        let handle = CoreHandle::open(dir.path()).unwrap();
        assert_eq!(
            handle.storage().count_articles().unwrap(),
            u64::try_from(report.articles).unwrap()
        );
        assert_eq!(handle.list_feeds().unwrap().len(), report.feeds);
        // A first page comes back newest-first and is fully hydrated.
        let page = handle
            .list_articles(ListArticles {
                limit: 50,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.len(), 50);
        assert!(page[0].id.0 > page[1].id.0, "id DESC keyset order");
        // curio_id round-tripped through the storage layer (valid v7).
        assert!(!page[0].content.text.is_empty());
        // FTS index was built by the insert triggers.
        let hits = handle.storage().search_articles("signal", 10).unwrap();
        assert!(!hits.is_empty(), "FTS should find a corpus token");
        // Backend-owned unread counts have real signal (not all-unread).
        let unread = handle.unread_counts().unwrap();
        let total: u64 = unread.values().copied().sum();
        assert!(total > 0 && total < u64::try_from(report.articles).unwrap());
    }
}

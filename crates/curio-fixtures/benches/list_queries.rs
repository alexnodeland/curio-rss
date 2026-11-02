//! List read-path benchmark (#136): the three reads the desktop list makes
//! on every state change, on the 100 000-article fixture —
//!
//! - `first_page`: the default view's first page of list rows (state and
//!   tags included, `archived: false`, 50 rows);
//! - `starred_page`: the Starred view's first page (`starred: true`,
//!   `archived: false`);
//! - `published_page`: the default view in publication-date order, one
//!   page deep (its keyset cursor seeks the date index);
//! - `unread_counts`: the per-feed unread badges.
//!
//! The budget is p95 < 10 ms for each. Criterion reports the mean; before
//! it runs, a plain timing pass prints p50 / p95 / max per read to stderr,
//! since the budget is a tail latency. The handle is opened once, so this
//! is warm-cache query latency — what the UI sees after start-up. A cold
//! page cache is not measured: there, a list row still costs SQLite the
//! reads of its stored body's overflow pages (see
//! `curio_core::model::ListedArticle`), which this bench never sees.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::hint::black_box;
use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};

use curio_core::CoreHandle;
use curio_core::storage::{ListArticles, ListCursor, ListOrder};
use curio_fixtures::{FixtureSpec, generate};

const PAGE: u32 = 50;

fn default_view() -> ListArticles {
    ListArticles {
        limit: PAGE,
        archived: Some(false),
        ..ListArticles::default()
    }
}

fn starred_view() -> ListArticles {
    ListArticles {
        limit: PAGE,
        starred: Some(true),
        archived: Some(false),
        ..ListArticles::default()
    }
}

/// p50 / p95 / max of `runs` timed calls of `read`.
fn percentiles(runs: usize, mut read: impl FnMut()) -> (Duration, Duration, Duration) {
    let mut samples: Vec<Duration> = (0..runs)
        .map(|_| {
            let start = Instant::now();
            read();
            start.elapsed()
        })
        .collect();
    samples.sort_unstable();
    let at = |q: usize| samples[(samples.len() * q / 100).min(samples.len() - 1)];
    (at(50), at(95), samples[samples.len() - 1])
}

fn list_queries(c: &mut Criterion) {
    let dir = tempfile::tempdir().expect("tempdir");
    let report = generate(dir.path(), &FixtureSpec::perf_100k()).expect("generate fixture");
    eprintln!(
        "fixture: {} articles / {} feeds, {} bytes",
        report.articles, report.feeds, report.bytes,
    );
    let handle = CoreHandle::open(dir.path()).expect("open core");

    let first_page = || {
        let rows = handle
            .list_article_rows(default_view())
            .expect("first page");
        assert_eq!(rows.len(), PAGE as usize);
        black_box(rows);
    };
    let starred_page = || {
        let rows = handle.list_article_rows(starred_view()).expect("starred");
        assert_eq!(rows.len(), PAGE as usize);
        black_box(rows);
    };
    let first_published = handle
        .list_article_rows(ListArticles {
            order: ListOrder::Published,
            ..default_view()
        })
        .expect("published first page");
    let cursor = first_published.last().map(ListCursor::from);
    let published_page = || {
        let rows = handle
            .list_article_rows(ListArticles {
                order: ListOrder::Published,
                after: cursor,
                ..default_view()
            })
            .expect("published page");
        assert_eq!(rows.len(), PAGE as usize);
        black_box(rows);
    };
    let unread_counts = || {
        black_box(handle.unread_counts().expect("unread counts"));
    };

    for (name, read) in [
        ("first_page", &first_page as &dyn Fn()),
        ("starred_page", &starred_page),
        ("published_page", &published_page),
        ("unread_counts", &unread_counts),
    ] {
        let (p50, p95, max) = percentiles(500, read);
        eprintln!("{name}: p50 {p50:?}, p95 {p95:?}, max {max:?} (budget: p95 < 10ms)");
    }

    let mut group = c.benchmark_group("list_queries_100k");
    group.bench_function("first_page", |b| b.iter(first_page));
    group.bench_function("starred_page", |b| b.iter(starred_page));
    group.bench_function("published_page", |b| b.iter(published_page));
    group.bench_function("unread_counts", |b| b.iter(unread_counts));
    group.finish();
}

criterion_group!(benches, list_queries);
criterion_main!(benches);

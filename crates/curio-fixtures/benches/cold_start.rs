//! Cold-start benchmark — the Phase-4 acceptance gate (docs/design/roadmap.md:
//! "cold start to interactive < 1.5s on the 50k-article fixture").
//!
//! It measures exactly what the first paint waits on: opening a
//! `CoreHandle` on the 50 000-article fixture and pulling the first page
//! (`list_articles`, newest-first, 50 rows) — the same call the reader's
//! initial article list makes. The fixture is generated once, outside the
//! timing loop, from the deterministic `curio-fixtures` generator.
//!
//! The nightly workflow runs this under github-action-benchmark with a 130%
//! regression alert (see .github/workflows/nightly.yml).
//!
//! Caveat recorded honestly: criterion re-opens the same file many times, so
//! the OS page cache is warm after the first iteration — this is the
//! open+query latency, not a fresh-process cold boot. That is also what the
//! CI budget gate measures, so the two agree.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};

use curio_core::CoreHandle;
use curio_core::storage::ListArticles;
use curio_fixtures::{FixtureSpec, generate};

fn cold_start(c: &mut Criterion) {
    // One 50k fixture, generated once and reused for every sample.
    let dir = tempfile::tempdir().expect("tempdir");
    let spec = FixtureSpec::perf_50k();
    let report = generate(dir.path(), &spec).expect("generate fixture");
    eprintln!(
        "fixture: {} articles / {} feeds, {} bytes, sha256 {}",
        report.articles, report.feeds, report.bytes, report.sha256,
    );

    let mut group = c.benchmark_group("cold_start");
    // Each sample opens a fresh handle on a multi-megabyte DB; keep the run
    // bounded but still statistically meaningful.
    group.sample_size(20);
    group.bench_function("open_and_first_page_50k", |b| {
        b.iter(|| {
            let handle = CoreHandle::open(dir.path()).expect("open core");
            let page = handle
                .list_articles(ListArticles {
                    limit: 50,
                    ..Default::default()
                })
                .expect("first page");
            assert_eq!(page.len(), 50);
            black_box(page);
            drop(handle);
        });
    });
    group.finish();
}

criterion_group!(benches, cold_start);
criterion_main!(benches);

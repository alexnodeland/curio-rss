//! No Rust source in the repo reads `CARGO_MANIFEST_DIR` at compile time
//! (issue #183). Agent worktrees share one `CARGO_TARGET_DIR`, cargo reuses
//! a test binary built in another worktree as fresh, and a baked-in crate
//! path then points into that worktree — the pre-push hook goes red once it
//! is removed. Read it at run time (`std::env::var_os`), which cargo sets
//! for every process it runs. Runs in the workspace suite, so CI and the
//! pre-push hook both enforce it.

#![allow(clippy::unwrap_used)]

#[test]
fn no_source_reads_the_manifest_dir_at_compile_time() {
    let root = xtask::paths::repo_root().unwrap();
    // The scan really walks the workspace — an empty result below means
    // "clean", not "looked in the wrong place".
    for dir in [
        "crates/curio-core/tests",
        "crates/curio-types/tests",
        "crates/curio-cli/tests",
        "apps/desktop/src-tauri/tests",
        "xtask/tests",
    ] {
        assert!(root.join(dir).is_dir(), "{dir} missing under {root:?}");
    }
    let hits = xtask::paths::scan_tree(&root).unwrap();
    assert!(
        hits.is_empty(),
        "compile-time CARGO_MANIFEST_DIR reads — use \
         std::env::var_os(\"CARGO_MANIFEST_DIR\") at run time (issue #183):\n{}",
        hits.join("\n")
    );
}

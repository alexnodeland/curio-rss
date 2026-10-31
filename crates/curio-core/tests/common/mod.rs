//! Shared helpers for the curio-core integration suites.

use std::path::PathBuf;

/// This crate's directory (`crates/curio-core`), read at run time.
///
/// Cargo sets `CARGO_MANIFEST_DIR` for every test process it launches.
/// Never use the compile-time `env!` form for paths: with one
/// `CARGO_TARGET_DIR` shared across git worktrees, cargo reuses a test
/// binary built in another worktree as fresh, and a baked-in path points
/// into that worktree — gone once it is removed (issue #183). The xtask
/// `compile_time_paths` test keeps the form out of the tree.
#[must_use]
pub fn manifest_dir() -> PathBuf {
    let Some(dir) = std::env::var_os("CARGO_MANIFEST_DIR") else {
        panic!("CARGO_MANIFEST_DIR unset: run the tests through cargo");
    };
    PathBuf::from(dir)
}

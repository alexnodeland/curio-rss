//! Run-time path resolution, and the guard that keeps compile-time crate
//! paths out of the tree (issue #183).
//!
//! Agent sessions share one `CARGO_TARGET_DIR` across every git worktree of
//! the checkout. Cargo fingerprints a workspace member by its sources and
//! metadata, not by the directory it was built from, so a test binary built
//! in worktree A is fresh in worktree B — and runs there with whatever paths
//! were baked in at compile time. The compile-time `env!` form of
//! `CARGO_MANIFEST_DIR` is exactly such a path: once A is removed, B's tests
//! fail with "No such file or directory" (or, worse, read and write A's
//! files). Cargo also sets `CARGO_MANIFEST_DIR` in the environment of every
//! process it runs (`cargo test`, `cargo run`), so the run-time read is
//! always the current worktree's.
//!
//! The guard scans this repo's sources only. A dependency macro that
//! expands to the same compile-time read is invisible to it and needs its
//! own run-time fix: insta's workspace-root lookup is one, handled by
//! `INSTA_WORKSPACE_ROOT` in `.cargo/config.toml` and pinned by the
//! `content_golden` suite in curio-core.

use std::path::{Path, PathBuf};

/// The banned spelling, assembled so this file does not contain it.
const NEEDLE: &str = concat!("env!(", "\"CARGO_MANIFEST_DIR\")");

/// The repo root (the parent of the `xtask` crate dir), resolved from the
/// `CARGO_MANIFEST_DIR` cargo sets when it runs xtask or its tests.
///
/// # Errors
///
/// When the variable is unset, i.e. the binary was not started by cargo.
pub fn repo_root() -> Result<PathBuf, String> {
    let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR")
        .ok_or("CARGO_MANIFEST_DIR is unset: run via `cargo run -p xtask -- <task>`")?;
    Ok(Path::new(&manifest_dir).join(".."))
}

/// 1-based line numbers of every compile-time `CARGO_MANIFEST_DIR` read in
/// `source` — `env!` or `option_env!`, whatever the whitespace inside the
/// call, including a call split across lines.
#[must_use]
pub fn compile_time_manifest_dir_lines(source: &str) -> Vec<usize> {
    // Drop all whitespace, remembering which line each kept char came from.
    let mut squashed = String::with_capacity(source.len());
    let mut line_of = Vec::with_capacity(source.len());
    let mut line = 1;
    for ch in source.chars() {
        if ch == '\n' {
            line += 1;
        }
        if !ch.is_whitespace() {
            squashed.push(ch);
            line_of.extend(std::iter::repeat_n(line, ch.len_utf8()));
        }
    }
    let raw_needle = NEEDLE.replace("(\"", "(r\"");
    let mut lines: Vec<usize> = [NEEDLE, raw_needle.as_str()]
        .iter()
        .flat_map(|needle| squashed.match_indices(needle).map(|(at, _)| line_of[at]))
        .collect();
    lines.sort_unstable();
    lines.dedup();
    lines
}

/// Every compile-time `CARGO_MANIFEST_DIR` read in the `.rs` files under
/// `root`, as `path:line` (path relative to `root`, `/`-separated), sorted.
/// Skips build output (`target`), npm trees (`node_modules`) and hidden
/// directories (`.git`, and `.claude/worktrees`, where other worktrees'
/// checkouts live).
///
/// # Errors
///
/// Any directory or file that cannot be read.
pub fn scan_tree(root: &Path) -> std::io::Result<Vec<String>> {
    let mut hits = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if entry.file_type()?.is_dir() {
                if !(name.starts_with('.') || name == "target" || name == "node_modules") {
                    stack.push(path);
                }
            } else if name.ends_with(".rs") {
                let source = std::fs::read_to_string(&path)?;
                let shown = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                for line in compile_time_manifest_dir_lines(&source) {
                    hits.push(format!("{shown}:{line}"));
                }
            }
        }
    }
    hits.sort();
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_env_and_option_env_with_their_lines() {
        let source = format!(
            "fn a() {{}}\nlet p = {NEEDLE};\nlet q = option_{NEEDLE};\nlet r = concat!({NEEDLE}, \"/x\");\n"
        );
        assert_eq!(compile_time_manifest_dir_lines(&source), vec![2, 3, 4]);
    }

    #[test]
    fn flags_calls_split_by_whitespace_and_raw_strings() {
        let split = "let p = env! (\n    \"CARGO_MANIFEST_DIR\"\n);\n";
        assert_eq!(compile_time_manifest_dir_lines(split), vec![1]);
        let raw = "let p = env!(r\"CARGO_MANIFEST_DIR\");";
        assert_eq!(compile_time_manifest_dir_lines(raw), vec![1]);
        // Line numbers stay right after multi-byte chars.
        let unicode = format!("// é — ü\n\nlet p = {NEEDLE};");
        assert_eq!(compile_time_manifest_dir_lines(&unicode), vec![3]);
    }

    #[test]
    fn allows_run_time_reads_and_other_env_vars() {
        let source = "let a = std::env::var(\"CARGO_MANIFEST_DIR\");\n\
                      let b = std::env::var_os(\"CARGO_MANIFEST_DIR\");\n\
                      let c = env!(\"CARGO_PKG_VERSION\");\n\
                      let d = env!(\"CARGO_BIN_EXE_curio\");\n";
        assert_eq!(compile_time_manifest_dir_lines(source), Vec::<usize>::new());
        assert_eq!(compile_time_manifest_dir_lines(""), Vec::<usize>::new());
    }

    /// A scratch tree, removed on drop.
    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn scan_reports_rs_files_and_skips_build_npm_and_hidden_dirs() {
        let scratch =
            Scratch(std::env::temp_dir().join(format!("xtask-paths-scan-{}", std::process::id())));
        let offending = format!("fn f() {{}}\nlet p = {NEEDLE};\n");
        for dir in [
            "crates/a/tests",
            "target/debug",
            "apps/node_modules/x",
            ".claude/worktrees/w",
        ] {
            let dir = scratch.0.join(dir);
            std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
            std::fs::write(dir.join("t.rs"), &offending).unwrap_or_else(|e| panic!("{e}"));
        }
        std::fs::write(scratch.0.join("crates/a/notes.md"), &offending)
            .unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(scratch.0.join("crates/a/clean.rs"), "fn g() {}\n")
            .unwrap_or_else(|e| panic!("{e}"));

        let hits = scan_tree(&scratch.0).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(hits, vec!["crates/a/tests/t.rs:2".to_owned()]);
        assert!(scan_tree(&scratch.0.join("missing")).is_err());
    }
}

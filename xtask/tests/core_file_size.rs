//! No `.rs` file under `crates/curio-core/src` grows past 700 lines (issue
//! #178). Many sweep issues touch the engine at once; files split along
//! their sections keep those changes from colliding, and the ceiling caught
//! `export/mod.rs` creeping back to 836 lines after the first split. Grow a
//! module by giving it a submodule, not by raising this number. Runs in the
//! workspace suite, so CI and the pre-push hook both enforce it.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

/// The ceiling, in lines, tests included.
const MAX_LINES: usize = 700;

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_core_source_file_exceeds_the_line_ceiling() {
    let src = xtask::paths::repo_root()
        .unwrap()
        .join("crates/curio-core/src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    // The scan really walks the engine — an empty result below means
    // "within the ceiling", not "looked in the wrong place".
    assert!(
        files.iter().any(|f| f.ends_with("export/mod.rs")),
        "curio-core sources not found under {src:?}"
    );
    let mut oversized: Vec<String> = files
        .iter()
        .filter_map(|file| {
            let lines = std::fs::read_to_string(file).unwrap().lines().count();
            (lines > MAX_LINES).then(|| {
                let rel = file.strip_prefix(&src).unwrap_or(file);
                format!("{} ({lines} lines)", rel.display())
            })
        })
        .collect();
    oversized.sort();
    assert!(
        oversized.is_empty(),
        "curio-core source files over {MAX_LINES} lines — split them along their \
         sections into submodules (issue #178):\n{}",
        oversized.join("\n")
    );
}

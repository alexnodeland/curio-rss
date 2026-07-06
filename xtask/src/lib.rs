//! Library surface of the xtask automation binary.
//!
//! Exists so the boundary rules are one definition shared by the `boundary`
//! subcommand (what CI runs) and the canary self-tests in
//! `tests/boundary_canary.rs` (the tripwire that keeps `deny.toml` honest).

/// Crate names that must never appear in curio-core's dependency tree.
/// Phase 4 moved the webview boundary: the desktop head
/// (`apps/desktop/src-tauri`) is a workspace member and the only crate
/// allowed to pull these (deny.toml `wrappers`); curio-core's own tree
/// stays webview-free forever — that is the invariant this scan holds.
///
/// Matching rule: a dependency violates the boundary if its name equals a
/// banned name or starts with `<banned>-` (the `tauri-*` family).
pub const BANNED: &[&str] = &["tauri", "wry", "tao"];

/// Scan a `cargo tree --prefix none` listing and return every crate name
/// that violates the webview boundary, in tree order (duplicates kept —
/// they mirror the tree).
#[must_use]
pub fn banned_crates_in_tree(tree: &str) -> Vec<&str> {
    tree.lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| {
            BANNED
                .iter()
                .any(|banned| name == banned || name.starts_with(&format!("{banned}-")))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_banned_names_and_their_dash_families() {
        let tree = "curio-core v0.1.0\ntauri v2.0.0\ntauri-utils v2.0.0\nwry v0.50.0\ntao v0.30.0\nserde v1.0.0\n";
        assert_eq!(
            banned_crates_in_tree(tree),
            vec!["tauri", "tauri-utils", "wry", "tao"]
        );
    }

    #[test]
    fn ignores_clean_trees_and_lookalike_prefixes() {
        // `taos`/`wryneck` share a prefix but are not `<banned>-` family.
        let tree = "curio-core v0.1.0\nserde v1.0.0\ntaos v0.1.0\nwryneck v0.1.0\n";
        assert!(banned_crates_in_tree(tree).is_empty());
        assert!(banned_crates_in_tree("").is_empty());
    }
}

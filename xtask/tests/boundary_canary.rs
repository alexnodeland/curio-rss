//! Tripwire against config rot in the webview boundary.
//!
//! The boundary was proven live once: adding `tauri` as a `curio-core`
//! dependency makes both `cargo deny check` (the `[bans]` table) and
//! `cargo run -p xtask -- boundary` fail. This canary keeps that proof
//! honest thereafter, cheaply: if someone edits `deny.toml` and drops a
//! ban, or the xtask ban list and `deny.toml` drift apart, the workspace
//! test suite goes red — no tauri download required.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;
use std::path::Path;

/// The `tauri-*` family members `deny.toml` pins explicitly (defense in
/// depth beyond xtask's prefix matching).
const TAURI_FAMILY: &[&str] = &[
    "tauri-build",
    "tauri-runtime",
    "tauri-runtime-wry",
    "tauri-codegen",
    "tauri-macros",
    "tauri-utils",
    "tauri-plugin",
];

fn deny_toml_banned_crates() -> BTreeSet<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../deny.toml");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("cannot read {}: {err}", path.display()));
    let doc: toml::Value = toml::from_str(&raw).expect("deny.toml must parse as TOML");
    let deny = doc
        .get("bans")
        .and_then(|bans| bans.get("deny"))
        .and_then(toml::Value::as_array)
        .expect("deny.toml must carry a [bans] table with a `deny` array");
    deny.iter()
        .map(|entry| {
            // cargo-deny accepts both bare strings and { crate = "..." } tables.
            entry
                .as_str()
                .or_else(|| entry.get("crate").and_then(toml::Value::as_str))
                .expect("every [bans].deny entry must name a crate")
                .to_owned()
        })
        .collect()
}

#[test]
fn deny_toml_still_bans_every_boundary_crate() {
    let banned = deny_toml_banned_crates();
    for name in xtask::BANNED {
        assert!(
            banned.contains(*name),
            "deny.toml [bans].deny lost the `{name}` ban — the webview boundary \
             (proven to fail the gate when violated) has config-rotted"
        );
    }
}

#[test]
fn deny_toml_still_pins_the_tauri_family() {
    let banned = deny_toml_banned_crates();
    for name in TAURI_FAMILY {
        assert!(
            banned.contains(*name),
            "deny.toml [bans].deny lost the `{name}` ban (tauri family)"
        );
    }
}

#[test]
fn xtask_scan_agrees_with_the_deny_list() {
    // Every crate deny.toml bans for the boundary must also be caught by
    // the xtask scan (exact name or `<banned>-` family) — the two layers
    // of defense must not drift apart.
    let scan_covers = |name: &str| {
        xtask::BANNED
            .iter()
            .any(|banned| name == *banned || name.starts_with(&format!("{banned}-")))
    };
    for name in deny_toml_banned_crates() {
        // webkit2gtk is a Linux-only transitive of wry; deny.toml bans it
        // directly, xtask catches it via wry's own presence upstream.
        if name == "webkit2gtk" {
            continue;
        }
        assert!(
            scan_covers(&name),
            "deny.toml bans `{name}` but `xtask boundary` would not flag it — \
             extend xtask::BANNED"
        );
    }
}

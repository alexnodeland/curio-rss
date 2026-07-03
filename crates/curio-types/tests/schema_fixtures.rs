//! Fixture-driven contract tests: every file under `fixtures/contracts/`
//! is validated against the published schemas in `schemas/`, and every
//! valid fixture must also round-trip through the corresponding Rust type.
//!
//! Adding a fixture file is enough to get it tested — the suites walk the
//! directories. A `*.noncanonical.json` valid fixture is schema-valid input
//! whose Rust round-trip normalizes the wire form (e.g. a `published`
//! timestamp without fractional seconds), so it is exempt from the
//! value-equality assertion but must still reserialize schema-valid.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::{Path, PathBuf};

use curio_types::{ArticleFrontmatter, EventEnvelope, ManifestV1};
use jsonschema::Validator;
use serde_json::Value;

use common::{EVENTS, FRONTMATTER, MANIFEST, assert_valid, repo_root};

fn fixtures(kind: &str, validity: &str) -> Vec<(PathBuf, Value)> {
    let dir = repo_root()
        .join("fixtures/contracts")
        .join(kind)
        .join(validity);
    let mut out: Vec<(PathBuf, Value)> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
        .map(|p| {
            let raw = std::fs::read_to_string(&p).unwrap();
            let value = serde_json::from_str(&raw)
                .unwrap_or_else(|e| panic!("parsing {}: {e}", p.display()));
            (p, value)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(!out.is_empty(), "no fixtures under {}", dir.display());
    out
}

fn is_noncanonical(path: &Path) -> bool {
    path.to_string_lossy().contains(".noncanonical")
}

/// Valid fixtures: schema-valid, deserializable into the Rust type, and the
/// reserialized form is schema-valid again (and value-identical unless the
/// fixture is marked noncanonical).
fn check_valid<T>(kind: &str, validator: &Validator)
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    for (path, value) in fixtures(kind, "valid") {
        let name = path.display().to_string();
        assert_valid(validator, &value, &name);
        let typed: T = serde_json::from_value(value.clone())
            .unwrap_or_else(|e| panic!("{name}: does not deserialize into the Rust type: {e}"));
        let reserialized = serde_json::to_value(&typed).unwrap();
        assert_valid(validator, &reserialized, &format!("{name} (reserialized)"));
        if !is_noncanonical(&path) {
            assert_eq!(
                reserialized, value,
                "{name}: Rust round-trip changed the wire form"
            );
        }
    }
}

fn check_invalid(kind: &str, validator: &Validator) {
    for (path, value) in fixtures(kind, "invalid") {
        assert!(
            !validator.is_valid(&value),
            "{}: expected the schema to reject this fixture",
            path.display()
        );
    }
}

#[test]
fn valid_event_fixtures_round_trip() {
    check_valid::<EventEnvelope>("events", &EVENTS);
}

#[test]
fn invalid_event_fixtures_are_rejected() {
    check_invalid("events", &EVENTS);
}

#[test]
fn valid_event_fixtures_cover_all_13_types() {
    let mut seen: Vec<String> = fixtures("events", "valid")
        .into_iter()
        .map(|(_, v)| v["type"].as_str().unwrap().to_owned())
        .collect();
    seen.sort();
    seen.dedup();
    let expected = [
        "article.archived",
        "article.opened",
        "article.read_later.added",
        "article.read_later.removed",
        "article.saved",
        "article.starred",
        "article.tagged",
        "article.unarchived",
        "article.unstarred",
        "article.untagged",
        "article.updated",
        "feed.added",
        "feed.removed",
    ];
    assert_eq!(
        seen, expected,
        "valid event fixtures must cover every v1 type"
    );
}

#[test]
fn valid_frontmatter_fixtures_round_trip() {
    check_valid::<ArticleFrontmatter>("frontmatter", &FRONTMATTER);
}

#[test]
fn invalid_frontmatter_fixtures_are_rejected() {
    check_invalid("frontmatter", &FRONTMATTER);
}

#[test]
fn frontmatter_preserves_user_keys() {
    let (path, value) = fixtures("frontmatter", "valid")
        .into_iter()
        .find(|(p, _)| p.to_string_lossy().contains("user-keys"))
        .expect("a user-keys fixture must exist");
    let typed: ArticleFrontmatter = serde_json::from_value(value).unwrap();
    assert!(
        typed.extra.contains_key("aliases") && typed.extra.contains_key("kp_enriched"),
        "{}: user keys must land in `extra`, not be dropped",
        path.display()
    );
}

#[test]
fn valid_manifest_fixtures_round_trip() {
    check_valid::<ManifestV1>("manifest", &MANIFEST);
}

#[test]
fn invalid_manifest_fixtures_are_rejected() {
    check_invalid("manifest", &MANIFEST);
}

#[test]
fn manifest_canonical_rendering_is_schema_valid() {
    for (path, value) in fixtures("manifest", "valid") {
        let typed: ManifestV1 = serde_json::from_value(value).unwrap();
        let canonical: Value = serde_json::from_str(&typed.to_canonical_json()).unwrap();
        assert_valid(&MANIFEST, &canonical, &path.display().to_string());
    }
}

#[test]
fn published_schemas_are_valid_draft_2020_12() {
    for schema in [
        common::frontmatter_schema_json(),
        common::events_schema_json(),
    ] {
        jsonschema::meta::validate(&schema)
            .unwrap_or_else(|e| panic!("schema is not a valid 2020-12 schema: {e}"));
    }
}

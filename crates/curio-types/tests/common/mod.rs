//! Shared helpers for the contract test suites: load the published JSON
//! Schemas from `schemas/` and build validators for them.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use jsonschema::Validator;
use serde_json::Value;

/// Repo root (two levels up from the crate).
#[must_use]
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|e| panic!("repo root: {e}"))
}

fn load_json(path: &Path) -> Value {
    let raw =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parsing {}: {e}", path.display()))
}

/// The raw `schemas/frontmatter.v1.json` document.
#[must_use]
pub fn frontmatter_schema_json() -> Value {
    load_json(&repo_root().join("schemas/frontmatter.v1.json"))
}

/// The raw `schemas/events.v1.json` document.
#[must_use]
pub fn events_schema_json() -> Value {
    load_json(&repo_root().join("schemas/events.v1.json"))
}

fn build(schema: &Value) -> Validator {
    jsonschema::validator_for(schema).unwrap_or_else(|e| panic!("compiling schema: {e}"))
}

/// Validator for the frontmatter.v1 root schema.
pub static FRONTMATTER: LazyLock<Validator> = LazyLock::new(|| build(&frontmatter_schema_json()));

/// Validator for the events.v1 root schema.
pub static EVENTS: LazyLock<Validator> = LazyLock::new(|| build(&events_schema_json()));

/// Validator for `curio.manifest.v1`, published as `$defs/manifest` inside
/// the frontmatter.v1 schema file. The root-level keywords are replaced by
/// a `$ref` into `$defs`, keeping every internal reference intact.
pub static MANIFEST: LazyLock<Validator> = LazyLock::new(|| {
    let mut schema = frontmatter_schema_json();
    let obj = schema
        .as_object_mut()
        .unwrap_or_else(|| panic!("frontmatter schema is not an object"));
    for key in ["type", "properties", "required", "additionalProperties"] {
        obj.remove(key);
    }
    obj.insert("$ref".into(), Value::String("#/$defs/manifest".into()));
    build(&schema)
});

/// Asserts `instance` validates, with a readable failure message.
pub fn assert_valid(validator: &Validator, instance: &Value, context: &str) {
    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|e| format!("{} @ {}", e, e.instance_path))
        .collect();
    assert!(
        errors.is_empty(),
        "{context}: schema violations: {errors:#?}"
    );
}

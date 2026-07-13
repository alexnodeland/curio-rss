//! Shared harness for the CLI integration suite: the hermetic axum
//! fixture server, subprocess builders, JSON helpers, and the published
//! contract-schema validators.

#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

use std::path::{Path, PathBuf};

use axum::Router;
use axum::routing::get;
use jsonschema::Validator;
use serde_json::Value;

/// The RSS fixture every test feeds through the real HTTP path.
pub const RSS: &str = include_str!("../../../../fixtures/feeds/rss2.xml");

/// Repo root (two levels up from the crate).
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// Serves `fixtures/feeds/rss2.xml` on an ephemeral 127.0.0.1 port and
/// returns the base URL. Hermetic: nothing leaves the loopback.
pub async fn fixture_server() -> String {
    let app = Router::new().route(
        "/feed.xml",
        get(|| async { ([("content-type", "application/rss+xml")], RSS) }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// A `curio` invocation against the given profile, with a scrubbed
/// environment (no inherited `$BROWSER`).
pub fn curio(profile: &Path) -> assert_cmd::Command {
    let mut cmd = assert_cmd::Command::cargo_bin("curio").unwrap();
    cmd.arg("--profile").arg(profile);
    cmd.env_remove("BROWSER");
    // Hermetic keychain: the mock store, never the real OS one.
    cmd.env("CURIO_MOCK_KEYRING", "1");
    cmd
}

/// Parses a successful run's stdout as one JSON document.
pub fn stdout_json(assert: &assert_cmd::assert::Assert) -> Value {
    serde_json::from_slice(&assert.get_output().stdout).unwrap()
}

/// init → subscribe (W1 exemption for the loopback fixture) → fetch.
pub fn seed(profile: &Path, server: &str) {
    curio(profile).arg("init").assert().success();
    curio(profile)
        .args(["feed", "add"])
        .arg(format!("{server}/feed.xml"))
        .args(["--tags", "fixtures", "--allow-private-network"])
        .assert()
        .success();
    curio(profile).arg("fetch").assert().success();
}

/// The full `curio_id` of the stored article with this title.
pub fn article_id_by_title(profile: &Path, title: &str) -> String {
    let list = stdout_json(
        &curio(profile)
            .args(["list", "--json", "--limit", "50"])
            .assert()
            .success(),
    );
    list.as_array()
        .unwrap()
        .iter()
        .find(|a| a["title"] == title)
        .unwrap_or_else(|| panic!("no article titled {title:?}"))["curio_id"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// Every event line under the profile's events dir, file- then
/// line-ordered, parsed as JSON.
pub fn events_lines(profile: &Path) -> Vec<Value> {
    let dir = profile.join(".curio").join("events");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "jsonl"))
        .collect();
    files.sort();
    let mut out = Vec::new();
    for file in files {
        for line in std::fs::read_to_string(&file).unwrap().lines() {
            if !line.trim().is_empty() {
                out.push(serde_json::from_str(line).unwrap());
            }
        }
    }
    out
}

/// The YAML frontmatter of an exported note, as a JSON value.
pub fn note_frontmatter(note: &str) -> Value {
    let rest = note.strip_prefix("---\n").expect("opening fence");
    let end = rest.find("\n---\n").expect("closing fence");
    let yaml: serde_yaml::Value = serde_yaml::from_str(&rest[..end]).unwrap();
    serde_json::to_value(yaml).unwrap()
}

fn load_schema(name: &str) -> Value {
    let path = repo_root().join("schemas").join(name);
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
}

/// Validator for the published `frontmatter.v1` schema.
pub fn frontmatter_validator() -> Validator {
    jsonschema::validator_for(&load_schema("frontmatter.v1.json")).unwrap()
}

/// Validator for the published `events.v1` schema.
pub fn events_validator() -> Validator {
    jsonschema::validator_for(&load_schema("events.v1.json")).unwrap()
}

/// Validator for `curio.manifest.v1`, published as `$defs/manifest`
/// inside the frontmatter schema file.
pub fn manifest_validator() -> Validator {
    let mut schema = load_schema("frontmatter.v1.json");
    let obj = schema.as_object_mut().unwrap();
    for key in ["type", "properties", "required", "additionalProperties"] {
        obj.remove(key);
    }
    obj.insert("$ref".into(), Value::String("#/$defs/manifest".into()));
    jsonschema::validator_for(&schema).unwrap()
}

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

/// Sets `settings.default_destination` in the profile's `curio.toml`,
/// the way a human would (an explicit config edit).
pub fn set_default_destination(profile: &Path, name: &str) {
    let path = profile.join("curio.toml");
    let mut value: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    value
        .as_table_mut()
        .unwrap()
        .entry("settings".to_owned())
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .unwrap()
        .insert(
            "default_destination".to_owned(),
            toml::Value::String(name.to_owned()),
        );
    std::fs::write(&path, toml::to_string(&value).unwrap()).unwrap();
}

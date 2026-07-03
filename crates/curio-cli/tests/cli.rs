//! End-to-end proof of the CLI head: real subprocess invocations against
//! a hermetic axum fixture server on 127.0.0.1, with every produced
//! artifact — notes, manifest, events — validated against the published
//! contract schemas in `schemas/`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use curio_core::events::{FoldedState, read_all};
use curio_types::CurioId;
use predicates::prelude::predicate;
use serde_json::json;

use support::{
    article_id_by_title, assert_valid, curio, events_lines, events_validator, fixture_server,
    frontmatter_validator, manifest_validator, note_frontmatter, repo_root, seed,
    set_default_destination, stdout_json,
};

#[test]
fn init_scaffolds_the_profile_and_is_idempotent() {
    let profile = tempfile::tempdir().unwrap();
    let first = stdout_json(
        &curio(profile.path())
            .args(["init", "--json"])
            .assert()
            .success(),
    );
    assert_eq!(first["created"], true);
    assert!(profile.path().join("curio.toml").is_file());
    assert!(profile.path().join("curio.db").is_file());
    assert!(profile.path().join(".curio/events").is_dir());
    let gitignore = std::fs::read_to_string(profile.path().join(".curio/.gitignore")).unwrap();
    assert!(
        gitignore.lines().any(|line| line.trim() == "events/"),
        "the events log must be gitignored by scaffold"
    );

    let second = stdout_json(
        &curio(profile.path())
            .args(["init", "--json"])
            .assert()
            .success(),
    );
    assert_eq!(second["created"], false, "re-init must not clobber config");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(
    clippy::too_many_lines,
    reason = "the happy path is one deliberate end-to-end narrative"
)]
async fn the_full_happy_path_produces_contract_valid_artifacts() {
    let server = fixture_server().await;
    let profile = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();

    curio(profile.path()).arg("init").assert().success();
    curio(profile.path())
        .args(["dest", "add", "vault"])
        .arg(vault.path())
        .assert()
        .success();
    curio(profile.path())
        .args(["feed", "add"])
        .arg(format!("{server}/feed.xml"))
        .args(["--tags", "fixtures", "--allow-private-network"])
        .assert()
        .success();

    // Contract W1 rides curio.toml — the flag is auditable configuration.
    let config = std::fs::read_to_string(profile.path().join("curio.toml")).unwrap();
    assert!(config.contains("allow_private_network = true"));

    // Refresh through the real HTTP path; the fixture has 3 items.
    let fetch = stdout_json(
        &curio(profile.path())
            .args(["fetch", "--json"])
            .assert()
            .success(),
    );
    assert_eq!(fetch[0]["status"], "ok");
    assert_eq!(fetch[0]["new_articles"], 3);

    let list = stdout_json(
        &curio(profile.path())
            .args(["list", "--json"])
            .assert()
            .success(),
    );
    assert_eq!(list.as_array().unwrap().len(), 3);

    // FTS through the head: only one item's body says "content".
    let hits = stdout_json(
        &curio(profile.path())
            .args(["search", "content", "--json"])
            .assert()
            .success(),
    );
    assert_eq!(hits.as_array().unwrap().len(), 1);
    let curio_id = hits[0]["curio_id"].as_str().unwrap().to_owned();
    let short = &curio_id[curio_id.len() - 8..];

    // show renders the sanitized body as markdown and marks it read.
    curio(profile.path())
        .args(["show", short])
        .assert()
        .success()
        .stdout(predicate::str::contains("**content**"));
    let shown = stdout_json(
        &curio(profile.path())
            .args(["show", short, "--json"])
            .assert()
            .success(),
    );
    assert_eq!(shown["read"], true);
    assert!(shown["markdown"].as_str().unwrap().contains("**content**"));

    // Tag, then export to the named destination.
    curio(profile.path())
        .args(["tag", short, "rust"])
        .assert()
        .success();
    let saved = stdout_json(
        &curio(profile.path())
            .args(["save", short, "--dest", "vault", "--json"])
            .assert()
            .success(),
    );
    assert_eq!(saved["disposition"], "created");
    let note_path = vault.path().join(saved["path"].as_str().unwrap());

    // The note on disk is a contract-valid curio.frontmatter.v1 document.
    let note = std::fs::read_to_string(&note_path).unwrap();
    assert!(note.contains(curio_types::MANAGED_REGION_BEGIN_V1));
    assert!(note.contains(curio_types::MANAGED_REGION_END_V1));
    let frontmatter = note_frontmatter(&note);
    assert_valid(&frontmatter_validator(), &frontmatter, "note frontmatter");
    assert_eq!(frontmatter["curio_id"], json!(curio_id));
    assert_eq!(frontmatter["tags"], json!(["rust"]));
    assert_eq!(frontmatter["checksum"], saved["checksum"]);

    // The manifest validates against $defs/manifest and maps id → path.
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(vault.path().join(".curio/manifest.json")).unwrap(),
    )
    .unwrap();
    assert_valid(&manifest_validator(), &manifest, "manifest");
    assert_eq!(manifest["schema"], "curio.manifest.v1");
    assert_eq!(manifest["notes"][&curio_id]["path"], saved["path"]);
    assert_eq!(manifest["notes"][&curio_id]["checksum"], saved["checksum"]);

    // Every event line is schema-valid; the story ends with article.saved.
    let events = events_lines(profile.path());
    let validator = events_validator();
    for (idx, event) in events.iter().enumerate() {
        assert_valid(&validator, event, &format!("event line {}", idx + 1));
    }
    let types: Vec<&str> = events.iter().map(|e| e["type"].as_str().unwrap()).collect();
    assert_eq!(types.first().copied(), Some("feed.added"));
    assert_eq!(types.last().copied(), Some("article.saved"));
    assert!(types.contains(&"article.tagged"));
    let saved_event = events.last().unwrap();
    assert_eq!(
        saved_event["payload"]["tags"],
        json!(["rust"]),
        "tags-in-payload rule"
    );
    assert_eq!(saved_event["payload"]["destination"], "vault");
    assert_eq!(saved_event["payload"]["path"], saved["path"]);

    // events tail -n shows exactly the last two, in order.
    let tail = stdout_json(
        &curio(profile.path())
            .args(["events", "tail", "-n", "2", "--json"])
            .assert()
            .success(),
    );
    let tail_types: Vec<&str> = tail
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["type"].as_str().unwrap())
        .collect();
    assert_eq!(&tail_types[..], &types[types.len() - 2..]);

    // Idempotent re-save: no write, no event.
    let resaved = stdout_json(
        &curio(profile.path())
            .args(["save", short, "--dest", "vault", "--json"])
            .assert()
            .success(),
    );
    assert_eq!(resaved["disposition"], "unchanged");
    assert_eq!(events_lines(profile.path()).len(), events.len());

    // settings.default_destination makes --dest optional.
    set_default_destination(profile.path(), "vault");
    let defaulted = stdout_json(
        &curio(profile.path())
            .args(["save", short, "--json"])
            .assert()
            .success(),
    );
    assert_eq!(defaulted["disposition"], "unchanged");

    // The healthy profile passes doctor.
    let doctor = stdout_json(
        &curio(profile.path())
            .args(["doctor", "--json"])
            .assert()
            .success(),
    );
    assert_eq!(doctor["ok"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn state_flips_emit_negations_and_fold_correctly() {
    let server = fixture_server().await;
    let profile = tempfile::tempdir().unwrap();
    seed(profile.path(), &server);
    let curio_id = article_id_by_title(profile.path(), "With a guid");
    let short = &curio_id[curio_id.len() - 8..];

    let flip = |args: &[&str]| {
        let mut full = args.to_vec();
        full.push("--json");
        stdout_json(&curio(profile.path()).args(&full).assert().success())
    };
    assert_eq!(flip(&["star", short])["changed"], true);
    assert_eq!(
        flip(&["star", short])["changed"],
        false,
        "idempotent re-star must not emit"
    );
    assert_eq!(flip(&["unstar", short])["changed"], true);
    assert_eq!(flip(&["later", short])["changed"], true);
    assert_eq!(flip(&["archive", short])["changed"], true);
    assert_eq!(flip(&["unarchive", short])["changed"], true);
    assert_eq!(flip(&["tag", short, "rust"])["changed"], true);
    assert_eq!(flip(&["untag", short, "rust"])["changed"], true);

    // The stream records exactly one event per real change, negations
    // included — and no event for the idempotent re-star.
    let events = read_all(&profile.path().join(".curio/events")).unwrap();
    let types: Vec<&str> = events.iter().map(|e| e.event.event_type()).collect();
    assert_eq!(
        types,
        vec![
            "feed.added",
            "article.starred",
            "article.unstarred",
            "article.read_later.added",
            "article.archived",
            "article.unarchived",
            "article.tagged",
            "article.untagged",
        ]
    );

    // Folding the stream honors the negations (histories are not
    // monotone): starred/archived/tags net to empty, read-later stays.
    let folded = FoldedState::fold(events);
    let id: CurioId = curio_id.parse().unwrap();
    assert!(!folded.starred.contains(&id));
    assert!(folded.read_later.contains(&id));
    assert!(!folded.archived.contains(&id));
    assert!(
        folded
            .tags
            .get(&id)
            .is_none_or(std::collections::BTreeSet::is_empty)
    );

    // The list filters agree with the fold.
    let read_later = stdout_json(
        &curio(profile.path())
            .args(["list", "--read-later", "--json"])
            .assert()
            .success(),
    );
    assert_eq!(read_later.as_array().unwrap().len(), 1);
    assert_eq!(read_later[0]["curio_id"], json!(curio_id));
    let starred = stdout_json(
        &curio(profile.path())
            .args(["list", "--starred", "--json"])
            .assert()
            .success(),
    );
    assert_eq!(starred.as_array().unwrap().len(), 0);
}

#[test]
fn opml_import_export_round_trips() {
    let profile = tempfile::tempdir().unwrap();
    curio(profile.path()).arg("init").assert().success();

    let opml = repo_root().join("fixtures/opml/simple.opml");
    let imported = stdout_json(
        &curio(profile.path())
            .args(["opml", "import"])
            .arg(&opml)
            .arg("--json")
            .assert()
            .success(),
    );
    assert_eq!(imported["added"], 3);
    assert_eq!(imported["skipped"], 0);

    // Re-import is a no-op: the platform is an inbox, never a fork.
    let again = stdout_json(
        &curio(profile.path())
            .args(["opml", "import"])
            .arg(&opml)
            .arg("--json")
            .assert()
            .success(),
    );
    assert_eq!(again["added"], 0);
    assert_eq!(again["skipped"], 3);

    let out = profile.path().join("subscriptions.opml");
    curio(profile.path())
        .args(["opml", "export"])
        .arg(&out)
        .assert()
        .success();
    let xml = std::fs::read_to_string(&out).unwrap();
    assert!(xml.contains("https://example.com/feed.xml"));
    assert!(xml.contains("https://atom.example.org/feed.atom"));

    // Unsubscribe by unique substring; the negation lands in the stream.
    curio(profile.path())
        .args(["feed", "rm", "atom.example.org"])
        .assert()
        .success();
    let feeds = stdout_json(
        &curio(profile.path())
            .args(["feed", "list", "--json"])
            .assert()
            .success(),
    );
    assert_eq!(feeds.as_array().unwrap().len(), 2);
    let events = events_lines(profile.path());
    let last = events.last().unwrap();
    assert_eq!(last["type"], "feed.removed");
    assert_eq!(
        last["payload"]["feed"],
        "https://atom.example.org/feed.atom"
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn open_emits_article_opened_and_launches_the_browser() {
    use std::os::unix::fs::PermissionsExt as _;

    let server = fixture_server().await;
    let profile = tempfile::tempdir().unwrap();
    seed(profile.path(), &server);
    let curio_id = article_id_by_title(profile.path(), "With a guid");
    let short = &curio_id[curio_id.len() - 8..];

    // A fake $BROWSER that records the URL it was handed.
    let captured = profile.path().join("browser-arg.txt");
    let script = profile.path().join("fake-browser.sh");
    std::fs::write(
        &script,
        "#!/bin/sh\nprintf '%s' \"$1\" > \"$CURIO_TEST_BROWSER_OUT\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    let opened = stdout_json(
        &curio(profile.path())
            .args(["open", short, "--json"])
            .env("BROWSER", &script)
            .env("CURIO_TEST_BROWSER_OUT", &captured)
            .assert()
            .success(),
    );
    assert_eq!(opened["url"], "https://example.com/posts/with-a-guid");

    // The browser is spawned detached — wait for the capture file.
    for _ in 0..50u8 {
        if captured.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(
        std::fs::read_to_string(&captured).unwrap(),
        "https://example.com/posts/with-a-guid"
    );

    // Contract event: article.opened, carrying the article identity.
    let events = events_lines(profile.path());
    let last = events.last().unwrap();
    assert_eq!(last["type"], "article.opened");
    assert_eq!(last["payload"]["curio_id"], json!(curio_id));
}

#[test]
fn failures_are_helpful_and_nonzero() {
    let profile = tempfile::tempdir().unwrap();
    curio(profile.path()).arg("init").assert().success();

    curio(profile.path())
        .args(["show", "deadbeef"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no article matches"));
    curio(profile.path())
        .args(["feed", "rm", "nope"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no feed matches"));

    // The events log is empty but healthy on a fresh profile.
    let tail = stdout_json(
        &curio(profile.path())
            .args(["events", "tail", "--json"])
            .assert()
            .success(),
    );
    assert_eq!(tail.as_array().unwrap().len(), 0);
}

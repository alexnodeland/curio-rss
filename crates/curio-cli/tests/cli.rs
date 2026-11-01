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

#[test]
fn a_garbage_database_is_refused_with_a_clear_message() {
    let profile = tempfile::tempdir().unwrap();
    let db = profile.path().join("curio.db");
    std::fs::write(&db, "not a database at all\n".repeat(64)).unwrap();
    curio(profile.path())
        .arg("doctor")
        .assert()
        .failure()
        .stderr(predicate::str::contains("is not a readable Curio database"));
    // Refused, never "repaired" over.
    assert!(
        std::fs::read_to_string(&db)
            .unwrap()
            .starts_with("not a database")
    );
}

/// #112: a profile inside a synced folder is refused, untouched, until
/// the user opts in with the flag or the environment variable.
#[test]
fn a_profile_in_a_synced_folder_needs_an_explicit_override() {
    let synced = tempfile::tempdir().unwrap();
    std::fs::create_dir(synced.path().join(".stfolder")).unwrap();
    let profile = synced.path().join("curio");
    curio(&profile)
        .arg("init")
        .assert()
        .failure()
        .stderr(predicate::str::contains("is on a Syncthing folder"))
        .stderr(predicate::str::contains("CURIO_ALLOW_NETWORK_FS"));
    assert!(
        !profile.join("curio.db").exists(),
        "refused before creating"
    );

    curio(&profile)
        .args(["init", "--allow-network-fs"])
        .assert()
        .success();
    assert!(profile.join("curio.db").is_file());
    curio(&profile)
        .arg("doctor")
        .assert()
        .failure()
        .stderr(predicate::str::contains("is on a Syncthing folder"));
    curio(&profile)
        .arg("doctor")
        .env("CURIO_ALLOW_NETWORK_FS", "1")
        .assert()
        .success();
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

    // `curio import --from opml` is the same subscribe path as `opml import`.
    let via_generic = stdout_json(
        &curio(profile.path())
            .args(["import", "--from", "opml"])
            .arg(&opml)
            .arg("--json")
            .assert()
            .success(),
    );
    assert_eq!(via_generic["feeds_added"], 0, "already subscribed");
    assert_eq!(via_generic["feeds_skipped"], 3);

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

#[test]
fn pocket_csv_imports_articles_as_read_later_via_the_cli() {
    let profile = tempfile::tempdir().unwrap();
    curio(profile.path()).arg("init").assert().success();

    let csv = repo_root().join("fixtures/import/pocket.csv");
    let imported = stdout_json(
        &curio(profile.path())
            .args(["import", "--from", "pocket"])
            .arg(&csv)
            .arg("--json")
            .assert()
            .success(),
    );
    assert_eq!(imported["feeds_added"], 0, "a Pocket CSV carries no feeds");
    assert_eq!(imported["articles_added"], 5);

    // The saves land in the read-later queue, no feed attached.
    let later = stdout_json(
        &curio(profile.path())
            .args(["list", "--read-later", "--json"])
            .assert()
            .success(),
    );
    assert_eq!(later.as_array().unwrap().len(), 5);

    // The multi-tag row applied its split tags — filterable by one of them.
    let tagged = stdout_json(
        &curio(profile.path())
            .args(["list", "--tag", "distributed-systems", "--json"])
            .assert()
            .success(),
    );
    assert_eq!(tagged.as_array().unwrap().len(), 1);

    // Read-later flags are real events in the stream.
    let events = events_lines(profile.path());
    assert!(
        events
            .iter()
            .any(|event| event["type"] == "article.read_later.added"),
        "importing a save stages a read-later event"
    );
}

#[test]
fn messy_import_files_import_and_report_their_bad_rows() {
    let profile = tempfile::tempdir().unwrap();
    curio(profile.path()).arg("init").assert().success();

    // A UTF-16LE Excel CSV: decoded, and its two unusable URLs reported.
    let csv = repo_root().join("fixtures/import/pocket-utf16le.csv");
    let imported = stdout_json(
        &curio(profile.path())
            .args(["import", "--from", "pocket"])
            .arg(&csv)
            .arg("--json")
            .assert()
            .success(),
    );
    assert_eq!(imported["articles_added"], 3);
    assert_eq!(imported["articles_invalid"], 2);
    assert_eq!(imported["rejected"][0]["kind"], "article");
    assert_eq!(imported["rejected"][0]["reason"], "unsupported_scheme");
    assert_eq!(imported["rejected"][0]["detail"], "javascript:alert(1)");
    assert_eq!(imported["rejected"][1]["reason"], "invalid_url");

    // HTML entities and a broken outline: every usable feed subscribes,
    // the bad ones are counted on stdout and named on stderr.
    let opml = repo_root().join("fixtures/opml/messy-entities.opml");
    curio(profile.path())
        .args(["opml", "import"])
        .arg(&opml)
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "imported 5 feed(s), skipped 0 already subscribed, 2 invalid",
        ))
        .stderr(predicate::str::contains(
            "skipped invalid entry (unsupported scheme): webcal://calendar.example/events.ics",
        ));
    let again = stdout_json(
        &curio(profile.path())
            .args(["opml", "import"])
            .arg(&opml)
            .arg("--json")
            .assert()
            .success(),
    );
    assert_eq!(again["added"], 0);
    assert_eq!(again["skipped"], 5);
    assert_eq!(again["invalid"], 2);
    let rejected = again["rejected"].as_array().unwrap();
    assert_eq!(rejected.len(), 2);
    assert!(rejected.iter().all(|row| row["kind"] == "feed"));
    assert!(rejected.iter().any(|row| {
        row["reason"] == "unsupported_scheme"
            && row["detail"] == "webcal://calendar.example/events.ics"
    }));
    curio(profile.path())
        .args(["import", "--from", "opml"])
        .arg(&opml)
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "imported 0 feed(s) and 0 article(s); skipped 5 already present, 2 invalid",
        ))
        .stderr(predicate::str::contains(
            "skipped invalid entry (unsupported scheme): webcal://calendar.example/events.ics",
        ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bulk_save_exports_the_library_idempotently() {
    let server = fixture_server().await;
    let profile = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    seed(profile.path(), &server);
    curio(profile.path())
        .args(["dest", "add", "vault"])
        .arg(vault.path())
        .assert()
        .success();

    // The fixture feed carries 3 items; --all exports each as a note.
    let first = stdout_json(
        &curio(profile.path())
            .args(["save", "--all", "--dest", "vault", "--json"])
            .assert()
            .success(),
    );
    assert_eq!(first["created"], 3);
    assert_eq!(first["total"], 3);

    let notes = std::fs::read_dir(vault.path().join("curio"))
        .unwrap()
        .count();
    assert_eq!(notes, 3);

    // Re-running rewrites nothing (manifest idempotency per note).
    let again = stdout_json(
        &curio(profile.path())
            .args(["save", "--all", "--dest", "vault", "--json"])
            .assert()
            .success(),
    );
    assert_eq!(again["created"], 0);
    assert_eq!(again["unchanged"], 3);

    // A filtered bulk export walks only the matching set.
    let starred = stdout_json(
        &curio(profile.path())
            .args(["save", "--starred", "--dest", "vault", "--json"])
            .assert()
            .success(),
    );
    assert_eq!(starred["total"], 0, "nothing starred yet");

    // A bare `curio save` selects nothing — refused, not a silent full export.
    curio(profile.path())
        .args(["save", "--dest", "vault"])
        .assert()
        .failure();
}

/// Every CLI invocation re-applies curio.toml's destinations. That must
/// never undo the missing-destination check: a vault that vanished (or an
/// unmounted mount point that now looks empty) is refused, not recreated
/// and fully re-exported — until `curio dest add` registers it afresh.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_vanished_vault_is_refused_not_recreated() {
    let server = fixture_server().await;
    let profile = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let vault = parent.path().join("vault");
    seed(profile.path(), &server);
    curio(profile.path())
        .args(["dest", "add", "vault"])
        .arg(&vault)
        .assert()
        .success();
    let first = stdout_json(
        &curio(profile.path())
            .args(["save", "--all", "--dest", "vault", "--json"])
            .assert()
            .success(),
    );
    assert_eq!(first["created"], 3);
    let saved = |profile: &std::path::Path| {
        events_lines(profile)
            .iter()
            .filter(|event| event["type"] == "article.saved")
            .count()
    };
    assert_eq!(saved(profile.path()), 3);

    // The folder is gone: refused, and nothing is recreated.
    std::fs::remove_dir_all(&vault).unwrap();
    curio(profile.path())
        .args(["save", "--all", "--dest", "vault"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("is missing"));
    assert!(!vault.exists(), "the vault root must not be recreated");

    // An empty folder at the old path (an unmounted mount point): refused
    // too, and left empty.
    std::fs::create_dir(&vault).unwrap();
    curio(profile.path())
        .args(["save", "--all", "--dest", "vault"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("is missing"));
    assert_eq!(std::fs::read_dir(&vault).unwrap().count(), 0);
    assert_eq!(saved(profile.path()), 3, "no duplicate article.saved");

    // Registering it again starts the destination afresh.
    curio(profile.path())
        .args(["dest", "add", "vault"])
        .arg(&vault)
        .assert()
        .success();
    let afresh = stdout_json(
        &curio(profile.path())
            .args(["save", "--all", "--dest", "vault", "--json"])
            .assert()
            .success(),
    );
    assert_eq!(afresh["created"], 3);
    assert!(vault.join(".curio/manifest.json").is_file());
}

/// #114: curio.toml seeds destinations; it does not own them. A
/// destination removed in the desktop app (a second engine handle on the
/// same profile, open the whole time) stays removed after later CLI runs,
/// and one the desktop adds shows up in `curio dest list` without
/// touching curio.toml.
#[test]
fn a_destination_removed_in_the_desktop_stays_removed_after_a_cli_run() {
    let profile = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let vault = parent.path().join("vault");
    curio(profile.path()).arg("init").assert().success();
    let desktop = curio_core::CoreHandle::open(profile.path()).unwrap();
    curio(profile.path())
        .args(["dest", "add", "vault"])
        .arg(&vault)
        .assert()
        .success();
    let listed = |profile: &std::path::Path| -> Vec<String> {
        stdout_json(
            &curio(profile)
                .args(["dest", "list", "--json"])
                .assert()
                .success(),
        )
        .as_array()
        .unwrap()
        .iter()
        .map(|dest| dest["name"].as_str().unwrap().to_owned())
        .collect()
    };
    assert_eq!(listed(profile.path()), ["vault"]);

    let vault_name: curio_types::DestinationName = "vault".parse().unwrap();
    desktop.remove_destination(&vault_name).unwrap();
    desktop
        .add_destination("notes".parse().unwrap(), parent.path().join("notes"))
        .unwrap();
    // Every CLI command reconciles curio.toml, which still declares vault.
    assert_eq!(listed(profile.path()), ["notes"]);
    assert_eq!(listed(profile.path()), ["notes"]);
    let names: Vec<String> = desktop
        .destinations()
        .unwrap()
        .into_iter()
        .map(|dest| dest.name.to_string())
        .collect();
    assert_eq!(names, ["notes"], "the desktop's view agrees");
    let config = std::fs::read_to_string(profile.path().join("curio.toml")).unwrap();
    assert!(config.contains("vault"), "curio.toml itself is untouched");

    // `curio dest add` is the explicit act: it registers the name again.
    curio(profile.path())
        .args(["dest", "add", "vault"])
        .arg(&vault)
        .assert()
        .success();
    assert_eq!(listed(profile.path()), ["notes", "vault"]);
}

#[test]
fn reddit_credentials_have_a_full_cli_surface_and_never_echo_the_secret() {
    let profile = tempfile::tempdir().unwrap();
    curio(profile.path()).arg("init").assert().success();

    // Unconfigured by default — reddit works unauthenticated out of the box.
    let status = stdout_json(
        &curio(profile.path())
            .args(["reddit", "status", "--json"])
            .assert()
            .success(),
    );
    assert_eq!(status["configured"], false);

    // Login stores + activates in one step; the secret comes from stdin
    // (not shell history) and never appears in any output.
    let login = curio(profile.path())
        .args(["reddit", "login", "--client-id", "cid123", "--json"])
        .write_stdin("sekret\n")
        .assert()
        .success();
    let logged_in = stdout_json(&login);
    assert_eq!(logged_in["configured"], true);
    assert_eq!(logged_in["client_id"], "cid123");
    let all_output = format!(
        "{}{}",
        String::from_utf8_lossy(&login.get_output().stdout),
        String::from_utf8_lossy(&login.get_output().stderr),
    );
    assert!(
        !all_output.contains("sekret"),
        "secret leaked: {all_output}"
    );

    // Empty inputs are refused, not stored.
    curio(profile.path())
        .args([
            "reddit",
            "login",
            "--client-id",
            "  ",
            "--client-secret",
            "x",
        ])
        .assert()
        .failure();

    // Logout is idempotent and reports unconfigured.
    let logout = stdout_json(
        &curio(profile.path())
            .args(["reddit", "logout", "--json"])
            .assert()
            .success(),
    );
    assert_eq!(logout["configured"], false);
}

#[test]
fn clip_saves_a_bare_link_read_later_and_reflags_idempotently() {
    let profile = tempfile::tempdir().unwrap();
    curio(profile.path()).arg("init").assert().success();

    // The URL is unreachable (loopback is SSRF-guarded for manual saves),
    // so this exercises the save-the-bare-link guarantee; hydration is
    // covered by the core suite.
    let url = "http://127.0.0.1:9/unreachable-page";
    let clipped = stdout_json(
        &curio(profile.path())
            .args(["clip", url, "--tags", "inbox", "--json"])
            .assert()
            .success(),
    );
    assert_eq!(clipped["created"], true);
    assert_eq!(clipped["hydrated"], false);
    assert_eq!(clipped["title"], url, "the URL stands in for a title");

    // It landed in read-later, feedless, with its tag.
    let later = stdout_json(
        &curio(profile.path())
            .args(["list", "--read-later", "--json"])
            .assert()
            .success(),
    );
    assert_eq!(later.as_array().unwrap().len(), 1);
    assert_eq!(later[0]["tags"][0], "inbox");

    // Re-clipping the same URL reuses the row.
    let again = stdout_json(
        &curio(profile.path())
            .args(["clip", url, "--json"])
            .assert()
            .success(),
    );
    assert_eq!(again["created"], false);
    assert_eq!(again["curio_id"], clipped["curio_id"]);

    // The read-later flag is a real event; a garbage URL is a real error.
    let events = events_lines(profile.path());
    assert!(
        events
            .iter()
            .any(|event| event["type"] == "article.read_later.added")
    );
    curio(profile.path())
        .args(["clip", "not-a-url"])
        .assert()
        .failure();
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

    // A fake $BROWSER that records the URL it was handed. It writes a
    // sibling temp file and renames it into place, so the capture file
    // only ever appears complete: a plain `> file` redirection creates the
    // (empty) file before `printf` runs, and the wait loop below could read
    // that empty file under a slow (instrumented) build (#230).
    let captured = profile.path().join("browser-arg.txt");
    let script = profile.path().join("fake-browser.sh");
    std::fs::write(
        &script,
        "#!/bin/sh\nprintf '%s' \"$1\" > \"$CURIO_TEST_BROWSER_OUT.tmp\" && \
         mv \"$CURIO_TEST_BROWSER_OUT.tmp\" \"$CURIO_TEST_BROWSER_OUT\"\n",
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

#[test]
fn a_missing_profile_is_refused_with_an_init_hint() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("nonexistent");
    curio(&missing)
        .arg("list")
        .assert()
        .failure()
        .stderr(predicate::str::contains("no curio profile at"))
        .stderr(predicate::str::contains("run `curio init`"));
    assert!(
        !missing.exists(),
        "a refused command must not create the profile"
    );

    // An existing directory that holds no database is not a profile either.
    let empty = root.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    curio(&empty)
        .args(["feed", "list"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("run `curio init`"));
    assert!(!empty.join("curio.db").exists());

    // `init` is the one command that creates it; afterwards the rest work.
    curio(&missing).arg("init").assert().success();
    curio(&missing).arg("list").assert().success();
}

#[test]
fn doctor_names_an_explicit_profile_as_such() {
    let profile = tempfile::tempdir().unwrap();
    curio(profile.path()).arg("init").assert().success();
    let doctor = stdout_json(
        &curio(profile.path())
            .args(["doctor", "--json"])
            .assert()
            .success(),
    );
    let checks = doctor["checks"].as_array().unwrap();
    let detail = checks.iter().find(|c| c["name"] == "profile").unwrap()["detail"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(detail.contains(&profile.path().display().to_string()));
    assert!(detail.contains("--profile"), "{detail}");
    // Legacy locations only matter for the default profile.
    assert!(checks.iter().all(|c| c["name"] != "legacy-profile"));
}

/// The default profile (no `--profile` / `CURIO_PROFILE`) is the shared
/// one: `$XDG_DATA_HOME/curio` on Linux, exactly where the desktop app
/// looks. Linux-only because that is the one platform whose data root an
/// environment variable can point at a temp dir (macOS and Windows read
/// the real user folders); the path itself is pinned for every platform by
/// `curio_core::paths`' unit tests.
#[cfg(target_os = "linux")]
#[test]
fn the_default_profile_is_the_shared_data_dir() {
    let home = tempfile::tempdir().unwrap();
    let data = home.path().join("data");
    let default_profile = data.join("curio");
    let bare = || {
        let mut cmd = assert_cmd::Command::cargo_bin("curio").unwrap();
        cmd.env_remove("CURIO_PROFILE")
            .env_remove("BROWSER")
            .env("CURIO_MOCK_KEYRING", "1")
            .env("HOME", home.path())
            .env("XDG_DATA_HOME", &data);
        cmd
    };

    bare()
        .arg("list")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            default_profile.display().to_string(),
        ))
        .stderr(predicate::str::contains("run `curio init`"));

    let init = stdout_json(&bare().args(["init", "--json"]).assert().success());
    assert_eq!(init["profile"], default_profile.display().to_string());
    assert!(default_profile.join("curio.db").is_file());

    let doctor = stdout_json(&bare().args(["doctor", "--json"]).assert().success());
    let checks = doctor["checks"].as_array().unwrap();
    let profile = checks.iter().find(|c| c["name"] == "profile").unwrap();
    assert!(
        profile["detail"]
            .as_str()
            .unwrap()
            .contains("shared with the desktop app")
    );
    let legacy = checks
        .iter()
        .find(|c| c["name"] == "legacy-profile")
        .unwrap();
    assert_eq!(legacy["ok"], true);
}

/// Another process holding the profile database (the desktop app
/// mid-write) is reported as what it is — busy, retry — with its own exit
/// status (75, `EX_TEMPFAIL`), not a generic `sqlite error` exit 1.
#[test]
fn a_busy_profile_says_so_and_exits_75() {
    let profile = tempfile::tempdir().unwrap();
    curio(profile.path()).arg("init").assert().success();

    let other = rusqlite::Connection::open(profile.path().join("curio.db")).unwrap();
    other.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let vault = profile.path().join("vault");
    curio(profile.path())
        .args(["dest", "add", "vault"])
        .arg(&vault)
        .assert()
        .code(75)
        .stderr(predicate::str::contains("busy"))
        .stderr(predicate::str::contains("another Curio process"));
    other.execute_batch("ROLLBACK").unwrap();

    // Once the other writer is done, the same command just works.
    curio(profile.path())
        .args(["dest", "add", "vault"])
        .arg(&vault)
        .assert()
        .success();
}

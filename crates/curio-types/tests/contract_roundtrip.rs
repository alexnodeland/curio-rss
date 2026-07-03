//! Type ↔ JSON ↔ schema round-trips built from Rust values: every contract
//! type serializes to something the published schemas accept, and comes
//! back equal.

#![allow(clippy::unwrap_used)]

mod common;

use std::collections::BTreeMap;

use curio_types::{
    ArticleFrontmatter, ArticleSnapshot, Checksum, CurioId, Destination, DestinationName,
    EventEnvelope, EventId, EventPayload, EventsSchemaV1Marker, FrontmatterSchemaV1Marker,
    ManifestEntry, ManifestSchemaV1Marker, ManifestV1, Timestamp,
};

use common::{EVENTS, FRONTMATTER, MANIFEST, assert_valid};

fn curio_id() -> CurioId {
    "0197b2c4-8f3e-7cc1-a5d2-3e9f10aa4b6d".parse().unwrap()
}

fn event_id() -> EventId {
    "01J1PZ2M3N4P5Q6R7S8T9V0W1X".parse().unwrap()
}

fn checksum() -> Checksum {
    "sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
        .parse()
        .unwrap()
}

fn ts() -> Timestamp {
    "2026-07-03T09:15:00.123Z".parse().unwrap()
}

fn snapshot() -> ArticleSnapshot {
    ArticleSnapshot {
        curio_id: curio_id(),
        title: "Article title".into(),
        source: "https://example.com/article".into(),
        feed: Some("https://example.com/feed.xml".into()),
        feed_title: Some("Example Blog".into()),
        tags: vec!["rust".into(), "databases".into()],
        published: Some("2026-07-01T12:00:00.000Z".parse().unwrap()),
        destination: "vault".parse().unwrap(),
        path: "curio/article-title.md".into(),
        checksum: checksum(),
    }
}

fn all_payloads() -> Vec<EventPayload> {
    vec![
        EventPayload::ArticleSaved(snapshot()),
        EventPayload::ArticleUpdated(snapshot()),
        EventPayload::ArticleStarred {
            curio_id: curio_id(),
            tags: vec!["rust".into()],
        },
        EventPayload::ArticleUnstarred {
            curio_id: curio_id(),
        },
        EventPayload::ReadLaterAdded {
            curio_id: curio_id(),
            tags: vec![],
        },
        EventPayload::ReadLaterRemoved {
            curio_id: curio_id(),
        },
        EventPayload::ArticleArchived {
            curio_id: curio_id(),
        },
        EventPayload::ArticleUnarchived {
            curio_id: curio_id(),
        },
        EventPayload::ArticleOpened {
            curio_id: curio_id(),
            dwell_ms: Some(45_210),
        },
        EventPayload::ArticleTagged {
            curio_id: curio_id(),
            tag: "rust".into(),
        },
        EventPayload::ArticleUntagged {
            curio_id: curio_id(),
            tag: "rust".into(),
        },
        EventPayload::FeedAdded {
            feed: "https://example.com/feed.xml".into(),
            feed_title: None,
            tags: vec!["tech".into()],
        },
        EventPayload::FeedRemoved {
            feed: "https://example.com/feed.xml".into(),
        },
    ]
}

#[test]
fn every_event_type_round_trips_and_validates() {
    let payloads = all_payloads();
    assert_eq!(payloads.len(), 13, "v1 has exactly 13 event types");
    for payload in payloads {
        let envelope = EventEnvelope {
            schema: EventsSchemaV1Marker,
            event_id: event_id(),
            ts: ts(),
            event: payload,
        };
        let value = serde_json::to_value(&envelope).unwrap();
        assert_valid(&EVENTS, &value, envelope.event.event_type());
        // The wire `type` field matches the accessor.
        assert_eq!(value["type"], envelope.event.event_type());
        // And it comes back equal.
        let back: EventEnvelope = serde_json::from_value(value).unwrap();
        assert_eq!(back, envelope);
    }
}

#[test]
fn envelope_wire_shape_matches_the_contract_example() {
    let envelope = EventEnvelope {
        schema: EventsSchemaV1Marker,
        event_id: event_id(),
        ts: ts(),
        event: EventPayload::ArticleArchived {
            curio_id: curio_id(),
        },
    };
    // Direct serialization preserves the envelope's field order, matching
    // the contract example line (key order is cosmetic in JSON, but the
    // producer emits it this way).
    let line = serde_json::to_string(&envelope).unwrap();
    assert!(
        line.starts_with(
            "{\"schema\":\"curio.events.v1\",\
             \"event_id\":\"01J1PZ2M3N4P5Q6R7S8T9V0W1X\",\
             \"ts\":\"2026-07-03T09:15:00.123Z\",\
             \"type\":\"article.archived\",\
             \"payload\":{"
        ),
        "unexpected wire prefix: {line}"
    );
    let value = serde_json::to_value(&envelope).unwrap();
    assert_eq!(value["schema"], "curio.events.v1");
    assert_eq!(value["event_id"], "01J1PZ2M3N4P5Q6R7S8T9V0W1X");
    assert_eq!(value["ts"], "2026-07-03T09:15:00.123Z");
    assert_eq!(value["type"], "article.archived");
    assert_eq!(
        value["payload"]["curio_id"],
        "0197b2c4-8f3e-7cc1-a5d2-3e9f10aa4b6d"
    );
}

#[test]
fn negation_semantics_match_the_contract_table() {
    let negations: Vec<(&str, Option<&str>)> = all_payloads()
        .iter()
        .map(|p| (p.event_type(), p.negates()))
        .collect();
    let expected = [
        ("article.saved", None),
        ("article.updated", None),
        ("article.starred", None),
        ("article.unstarred", Some("article.starred")),
        ("article.read_later.added", None),
        (
            "article.read_later.removed",
            Some("article.read_later.added"),
        ),
        ("article.archived", None),
        ("article.unarchived", Some("article.archived")),
        ("article.opened", None),
        ("article.tagged", None),
        ("article.untagged", Some("article.tagged")),
        ("feed.added", None),
        ("feed.removed", Some("feed.added")),
    ];
    assert_eq!(negations, expected);
}

#[test]
fn article_events_expose_their_curio_id() {
    for payload in all_payloads() {
        let expected = if payload.event_type().starts_with("article.") {
            Some(curio_id())
        } else {
            None
        };
        assert_eq!(payload.curio_id(), expected, "{}", payload.event_type());
    }
}

#[test]
fn opened_without_dwell_omits_the_key() {
    let value = serde_json::to_value(EventPayload::ArticleOpened {
        curio_id: curio_id(),
        dwell_ms: None,
    })
    .unwrap();
    assert!(value["payload"].get("dwell_ms").is_none());
}

fn full_frontmatter() -> ArticleFrontmatter {
    ArticleFrontmatter {
        schema: FrontmatterSchemaV1Marker,
        curio_id: curio_id(),
        title: "Article title".into(),
        source: "https://example.com/article".into(),
        feed: Some("https://example.com/feed.xml".into()),
        feed_title: Some("Example Blog".into()),
        author: Some("Jane Doe".into()),
        published: Some("2026-07-01T12:00:00.000Z".parse().unwrap()),
        saved: ts(),
        tags: vec!["rust".into(), "databases".into()],
        checksum: checksum(),
        lang: Some("en".into()),
        word_count: Some(1234),
        extra: BTreeMap::new(),
    }
}

#[test]
fn frontmatter_round_trips_and_validates() {
    for frontmatter in [
        full_frontmatter(),
        // Manual save: nullable keys null, optional keys absent, a user key.
        ArticleFrontmatter {
            feed: None,
            feed_title: None,
            author: None,
            published: None,
            lang: None,
            word_count: None,
            extra: BTreeMap::from([("kp_enriched".to_owned(), serde_json::json!(true))]),
            ..full_frontmatter()
        },
    ] {
        let value = serde_json::to_value(&frontmatter).unwrap();
        assert_valid(&FRONTMATTER, &value, "frontmatter");
        let back: ArticleFrontmatter = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(back, frontmatter);
    }
}

#[test]
fn frontmatter_nullable_keys_are_always_present_and_optionals_absent() {
    let frontmatter = ArticleFrontmatter {
        feed: None,
        feed_title: None,
        author: None,
        published: None,
        lang: None,
        word_count: None,
        ..full_frontmatter()
    };
    let value = serde_json::to_value(&frontmatter).unwrap();
    let obj = value.as_object().unwrap();
    // Required-but-nullable: present as null.
    assert!(obj["feed"].is_null());
    assert!(obj["published"].is_null());
    // Optional: omitted entirely.
    for absent in ["feed_title", "author", "lang", "word_count"] {
        assert!(!obj.contains_key(absent), "{absent} should be omitted");
    }
}

#[test]
fn manifest_round_trips_and_validates() {
    let mut manifest = ManifestV1 {
        schema: ManifestSchemaV1Marker,
        notes: BTreeMap::new(),
    };
    manifest.notes.insert(
        curio_id(),
        ManifestEntry {
            path: "curio/article-title.md".into(),
            checksum: checksum(),
            exported_at: ts(),
        },
    );
    let value = serde_json::to_value(&manifest).unwrap();
    assert_valid(&MANIFEST, &value, "manifest");
    let back: ManifestV1 = serde_json::from_value(value).unwrap();
    assert_eq!(back, manifest);

    // The canonical on-disk rendering is the same document.
    let canonical: ManifestV1 = serde_json::from_str(&manifest.to_canonical_json()).unwrap();
    assert_eq!(canonical, manifest);
}

#[test]
fn destination_round_trips() {
    let destination = Destination {
        name: "vault".parse::<DestinationName>().unwrap(),
        root: "/home/alex/vault".into(),
    };
    let value = serde_json::to_value(&destination).unwrap();
    assert_eq!(value["name"], "vault");
    let back: Destination = serde_json::from_value(value).unwrap();
    assert_eq!(back, destination);
}

#[test]
fn destination_name_rejects_paths_on_deserialize() {
    assert!(serde_json::from_value::<DestinationName>(serde_json::json!("a/b")).is_err());
    assert!(serde_json::from_value::<DestinationName>(serde_json::json!("..")).is_err());
}

#[test]
fn envelope_new_mints_valid_ids_and_timestamps() {
    let envelope = EventEnvelope::new(EventPayload::ArticleArchived {
        curio_id: curio_id(),
    });
    let value = serde_json::to_value(&envelope).unwrap();
    assert_valid(&EVENTS, &value, "freshly minted envelope");
}

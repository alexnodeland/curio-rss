//! Property: a rendered note parses back to exactly the frontmatter it was
//! rendered from — arbitrary titles, tags and user keys included — so a
//! re-export never drifts a value it wrote itself.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::Path;

use curio_types::{ArticleFrontmatter, FrontmatterSchemaV1Marker};
use proptest::prelude::*;
use serde_json::Value;

use super::note::{parse_note, render_note};
use super::region_checksum;

/// Curio's own keys: a user key by one of these names would be read back
/// as the field, not as an extra.
const KNOWN_KEYS: &[&str] = &[
    "schema",
    "curio_id",
    "title",
    "source",
    "feed",
    "feed_title",
    "author",
    "published",
    "saved",
    "tags",
    "checksum",
    "lang",
    "word_count",
];

/// Characters weighted toward everything YAML treats specially.
fn interesting_char() -> impl Strategy<Value = char> {
    prop_oneof![
        6 => any::<char>(),
        4 => proptest::char::range('a', 'z'),
        3 => Just(' '),
        2 => Just('\n'),
        3 => prop::sample::select(
            "-:#?,[]{}&*!|>'\"%@`.~+0179xe\t\r\0\u{7}\u{1b}\u{7f}\u{85}\u{a0}\u{2028}\u{2029}\u{feff}\u{fffe}\u{200b}\\"
                .chars()
                .collect::<Vec<_>>()
        ),
    ]
}

/// A string. U+2028/U+2029 are kept out of multi-line strings: the
/// emitter writes them raw at a block-scalar line start or line end, a
/// shape a YAML 1.2 reader cannot read back (`yaml::from_str` refuses it
/// instead of misreading it).
fn text() -> impl Strategy<Value = String> {
    let chars = prop::collection::vec(interesting_char(), 0..16)
        .prop_map(|chars| chars.into_iter().collect::<String>());
    let spelled = prop::sample::select(vec![
        "", "null", "~", "true", "False", "yes", "42", "-7", "+5", "0x1F", "007", "1e3", ".inf",
        "-.Inf", ".NaN", "---", "...", "- a", "a: b", "#x", "a #b", "0o7", "1_0", " lead",
        "trail ", "a\nb", "a\n", "\n", " a\nb",
    ])
    .prop_map(str::to_owned);
    prop_oneof![chars, spelled, "[a-z ]{120,140}"].prop_map(|s| {
        if s.contains('\n') {
            s.replace(['\u{2028}', '\u{2029}'], "_")
        } else {
            s
        }
    })
}

fn json_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::from),
        any::<u64>().prop_map(Value::from),
        any::<f64>().prop_map(Value::from),
        text().prop_map(Value::String),
    ];
    leaf.prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
            prop::collection::btree_map(text(), inner, 0..4)
                .prop_map(|map| Value::Object(map.into_iter().collect())),
        ]
    })
}

prop_compose! {
    fn frontmatter()(
        title in text(),
        feed in proptest::option::of(text()),
        feed_title in proptest::option::of(text()),
        author in proptest::option::of(text()),
        tags in prop::collection::vec(text(), 0..4),
        lang in proptest::option::of(text()),
        word_count in proptest::option::of(any::<u64>()),
        extra in prop::collection::btree_map(text(), json_value(), 0..5),
    ) -> ArticleFrontmatter {
        let extra: BTreeMap<String, Value> = extra
            .into_iter()
            .filter(|(key, _)| !KNOWN_KEYS.contains(&key.as_str()))
            .collect();
        ArticleFrontmatter {
            schema: FrontmatterSchemaV1Marker,
            curio_id: "0197b2c4-8f3e-7cc1-a5d2-3e9f10aa4b6d".parse().unwrap(),
            title,
            source: "https://example.com/article".to_owned(),
            feed,
            feed_title,
            author,
            published: None,
            saved: "2026-07-03T09:15:00.123Z".parse().unwrap(),
            tags,
            checksum: region_checksum("body"),
            lang,
            word_count,
            extra,
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn rendered_frontmatter_parses_back_unchanged(fm in frontmatter()) {
        let note = render_note(&fm, "body").unwrap();
        let (_, parsed) = parse_note(&note, Path::new("prop.md")).unwrap();
        prop_assert_eq!(parsed, fm);
    }
}

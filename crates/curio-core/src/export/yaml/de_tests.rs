//! Deserializer unit tests: typing rules, refusals, aliases and limits.

#![allow(clippy::unwrap_used, clippy::float_cmp)]

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Deserialize;
use serde_json::{Value, json};

use super::*;

fn value(input: &str) -> Value {
    from_str(input).unwrap()
}

fn error<T: DeserializeOwned + fmt::Debug>(input: &str) -> String {
    from_str::<T>(input).unwrap_err().to_string()
}

#[test]
fn untyped_values_resolve_by_the_core_schema() {
    assert_eq!(
        value("a: yes\nb: 0x10\nc: ~\nd: 1e3\ne: '1'\nf: |\n  x\ng: !!str 2\nh: !!int '3'\n"),
        json!({"a": "yes", "b": 16, "c": null, "d": 1000.0, "e": "1", "f": "x\n", "g": "2", "h": 3})
    );
    assert_eq!(
        value("i: !!float 2\nj: !!bool true\nk: !!null null\nl: .inf\nm: -9223372036854775808\n"),
        json!({"i": 2.0, "j": true, "k": null, "l": null, "m": i64::MIN})
    );
    assert_eq!(value(""), Value::Null);
    assert_eq!(value("# only a comment\n"), Value::Null);
    // Wider than u64/i64: serde_json refuses it, as it always did.
    assert!(from_str::<Value>("a: 18446744073709551616\n").is_err());
    assert!(from_str::<Value>("a: !!int -9223372036854775809\n").is_err());
}

#[test]
fn mistyped_core_tags_are_refused() {
    for input in [
        "a: !!bool yes\n",
        "a: !!int x\n",
        "a: !!float x\n",
        "a: !!null x\n",
    ] {
        assert!(from_str::<Value>(input).is_err(), "{input:?}");
    }
}

#[test]
fn custom_tags_extra_documents_and_unknown_aliases_are_refused() {
    assert!(error::<Value>("a: !local x\n").contains("custom YAML tag !local"));
    assert!(error::<Value>("a: !local [x]\n").contains("custom YAML tag"));
    assert!(error::<Value>("a: 1\n---\nb: 2\n").contains("more than one YAML document"));
    assert!(error::<Value>("a: *nowhere\n").contains("unknown"));
    assert!(error::<Value>("a: [1\n").contains("line"));
}

#[test]
fn unicode_breaks_fold_in_flow_scalars_and_are_refused_where_shapes_differ() {
    // What the emitter writes for "a<LS>b" and "a<PS>": the blanks after
    // the break are its indentation, not content.
    assert_eq!(
        value("k: 'a\u{2028}  b'\nj: 'a\u{2029}  '\np: x \u{2028}\ty\n"),
        json!({"k": "a\u{2028}b", "j": "a\u{2029}", "p": "x\u{2028}y"})
    );
    assert!(error::<Value>("k: |-\n  a\n  b\u{2028}  c\n").contains("U+2028"));
    assert!(error::<Value>("k: |2+\n\n\u{2028}").contains("U+2028"));
    assert!(error::<Value>("\u{2029}k: v\n").contains("U+2028"));
}

#[derive(Debug, PartialEq, Deserialize)]
struct Wrapper(u8);

#[derive(Debug, PartialEq, Deserialize)]
struct Typed {
    anchor: u8,
    b: bool,
    i: i8,
    wide: i128,
    u: u16,
    uwide: u128,
    f: f32,
    c: char,
    s: String,
    number_text: String,
    unit: (),
    empty_unit: (),
    tuple: (u8, String),
    none: Option<u8>,
    tilde: Option<String>,
    quoted_null: Option<String>,
    tagged_null: Option<String>,
    tagged_text: Option<String>,
    list: Option<Vec<u8>>,
    empty_seq: Vec<u8>,
    empty_map: BTreeMap<String, u8>,
    newtype: Wrapper,
}

const TYPED: &str = "\
b: TRUE
i: -0x10
wide: -170141183460469231731687303715884105728
u: +0o17
uwide: 340282366920938463463374607431768211455
f: -.inf
c: x
s: plain text
number_text: 2024
unit: ~
empty_unit:
tuple: [7, seven]
none: null
tilde: ~
quoted_null: 'null'
tagged_null: !!null Null
tagged_text: !!str ~
list: [1, 2]
empty_seq:
empty_map:
newtype: 9
ignored: {deep: [1, {x: y}], again: *anchored}
";

#[test]
fn typed_fields_read_like_serde_yaml_did() {
    let input = format!("anchor: &anchored 1\n{TYPED}");
    let parsed: Typed = from_str(&input).unwrap();
    assert_eq!(
        parsed,
        Typed {
            anchor: 1,
            b: true,
            i: -16,
            wide: i128::MIN,
            u: 15,
            uwide: u128::MAX,
            f: f32::NEG_INFINITY,
            c: 'x',
            s: "plain text".into(),
            number_text: "2024".into(),
            unit: (),
            empty_unit: (),
            tuple: (7, "seven".into()),
            none: None,
            tilde: None,
            quoted_null: Some("null".into()),
            tagged_null: None,
            tagged_text: Some("~".into()),
            list: Some(vec![1, 2]),
            empty_seq: vec![],
            empty_map: BTreeMap::new(),
            newtype: Wrapper(9),
        }
    );
}

#[derive(Debug, Deserialize)]
struct UnitField {
    #[allow(dead_code)] // only its type matters
    a: (),
}

#[test]
fn typed_mismatches_are_errors_with_a_location() {
    let err = error::<BTreeMap<String, bool>>("a: true\nb: 'true'\n");
    assert!(err.contains("a boolean") && err.contains("line 2"), "{err}");
    assert!(error::<BTreeMap<String, i64>>("a: 1.5\n").contains("an integer"));
    assert!(error::<BTreeMap<String, u64>>("a: -1\n").contains("an integer"));
    assert!(error::<BTreeMap<String, f64>>("a: [1]\n").contains("a float"));
    assert!(error::<BTreeMap<String, String>>("a: [1]\n").contains("sequence"));
    assert!(error::<UnitField>("a: x\n").contains("null"));
    assert!(error::<UnitField>("a: {}\n").contains("map"));
    assert!(error::<BTreeMap<String, Vec<u8>>>("a: x\n").contains("sequence"));
    assert!(error::<BTreeMap<String, BTreeMap<String, u8>>>("a: x\n").contains("map"));
    assert!(error::<BTreeMap<String, Option<String>>>("a: !!null x\n").contains("null"));
    assert!(error::<BTreeMap<String, (u8,)>>("a: [1, 2]\n").contains("1 element"));
    assert!(error::<BTreeMap<String, Wrapper>>("a: x\n").contains("an integer"));
}

#[test]
fn literal_blocks_with_the_matching_core_tag_are_typed() {
    assert_eq!(
        from_str::<BTreeMap<String, u8>>("a: !!int |-\n  7\n").unwrap()["a"],
        7
    );
    assert!(from_str::<BTreeMap<String, u8>>("a: |-\n  7\n").is_err());
}

#[derive(Debug, PartialEq, Deserialize)]
struct Aliased {
    anchors: IgnoredAny,
    b: bool,
    i: i64,
    wide: i128,
    u: u64,
    uwide: u128,
    f: f64,
    s: String,
    unit: (),
    none: Option<u8>,
    some: Option<u8>,
    seq: Vec<u8>,
    map: BTreeMap<String, u8>,
    any: Value,
}

#[test]
fn aliases_replay_their_anchored_node_for_every_type() {
    let input = "\
anchors: [&b true, &i -5, &u 7, &f 1.5, &s text, &n ~, &q [1, 2], &m {k: 1}]
b: *b
i: *i
wide: *i
u: *u
uwide: *u
f: *f
s: *s
unit: *n
none: *n
some: *u
seq: *q
map: *m
any: *m
";
    let parsed: Aliased = from_str(input).unwrap();
    assert_eq!(
        parsed,
        Aliased {
            anchors: IgnoredAny,
            b: true,
            i: -5,
            wide: -5,
            u: 7,
            uwide: 7,
            f: 1.5,
            s: "text".into(),
            unit: (),
            none: None,
            some: Some(7),
            seq: vec![1, 2],
            map: BTreeMap::from([("k".to_owned(), 1)]),
            any: json!({"k": 1}),
        }
    );
}

#[test]
fn alias_bombs_and_deep_nesting_are_bounded() {
    let mut bomb = String::from("a: &a [x, x, x, x, x, x, x, x, x, x]\n");
    let names = ["a", "b", "c", "d", "e", "f", "g", "h"];
    for pair in names.windows(2) {
        let (prev, name) = (pair[0], pair[1]);
        let refs = vec![format!("*{prev}"); 10].join(", ");
        writeln!(bomb, "{name}: &{name} [{refs}]").unwrap();
    }
    assert!(error::<Value>(&bomb).contains("alias expansion limit"));

    let deep = format!("{}{}", "[".repeat(200), "]".repeat(200));
    assert!(error::<Value>(&deep).contains("nests too deeply"));
}

#[test]
fn enums_and_byte_strings_are_unsupported() {
    #[derive(Debug, Deserialize)]
    enum E {
        A,
    }

    #[derive(Debug)]
    struct Bytes;
    impl<'de> Deserialize<'de> for Bytes {
        fn deserialize<D: de::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct V;
            impl Visitor<'_> for V {
                type Value = Bytes;
                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("bytes")
                }
            }
            d.deserialize_byte_buf(V)
        }
    }
    assert!(error::<E>("A\n").contains("enums"));
    assert!(error::<Bytes>("x\n").contains("byte strings"));
}

#[test]
fn collections_with_unread_entries_report_their_length() {
    // A visitor that stops after one entry: the rest is skipped, counted
    // and reported, as serde_yaml did.
    #[derive(Debug)]
    struct First(u8);
    impl<'de> Deserialize<'de> for First {
        fn deserialize<D: de::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct V;
            impl<'de> Visitor<'de> for V {
                type Value = First;
                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("a sequence or map")
                }
                fn visit_seq<A: de::SeqAccess<'de>>(self, mut a: A) -> Result<First, A::Error> {
                    Ok(First(a.next_element()?.unwrap_or(0)))
                }
                fn visit_map<A: de::MapAccess<'de>>(self, mut a: A) -> Result<First, A::Error> {
                    Ok(First(a.next_entry::<String, u8>()?.map_or(0, |(_, v)| v)))
                }
            }
            d.deserialize_any(V)
        }
    }
    assert!(error::<First>("[1, 2, [3]]\n").contains("invalid length 3"));
    assert!(error::<First>("{a: 1, b: {c: 2}}\n").contains("invalid length 2"));
    assert_eq!(from_str::<First>("[5]\n").unwrap().0, 5);
}

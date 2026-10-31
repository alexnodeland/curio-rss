//! A YAML document tree for the workflow lint, loaded with
//! [`saphyr_parser`] (safe Rust, YAML 1.2). It replaces `serde_yaml::Value`
//! (deprecated, over the unmaintained `unsafe-libyaml`; #180) with the same
//! shape and accessors the lint uses. Inputs are trusted repo files, so
//! aliases simply copy their anchored node.

use std::collections::BTreeMap;
use std::fmt;
use std::ops::Index;

use saphyr_parser::{Event, Parser, ScalarStyle, Tag};

/// A YAML node.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `~`, `null`, or an empty plain scalar.
    Null,
    /// `true`/`false` (any case variant YAML 1.2 allows).
    Bool(bool),
    /// An integer or float, as written.
    Number(String),
    /// Any other scalar.
    String(String),
    /// A sequence.
    Sequence(Vec<Value>),
    /// A mapping, in document order.
    Mapping(Mapping),
    /// A node carrying a custom (`!local`) tag.
    Tagged(Box<TaggedValue>),
}

/// A node with a custom tag.
#[derive(Debug, Clone, PartialEq)]
pub struct TaggedValue {
    /// The tag, e.g. `!reference`.
    pub tag: String,
    /// The tagged node.
    pub value: Value,
}

/// A mapping's entries, in document order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Mapping(Vec<(Value, Value)>);

static NULL: Value = Value::Null;

impl Mapping {
    /// The value under the string key `key`.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0
            .iter()
            .find(|(k, _)| k.as_str() == Some(key))
            .map(|(_, v)| v)
    }

    /// The entries, in document order.
    pub fn iter(&self) -> impl Iterator<Item = (&Value, &Value)> {
        self.0.iter().map(|(k, v)| (k, v))
    }

    /// The keys, in document order.
    pub fn keys(&self) -> impl Iterator<Item = &Value> {
        self.0.iter().map(|(k, _)| k)
    }
}

impl<'a> IntoIterator for &'a Mapping {
    type Item = (&'a Value, &'a Value);
    type IntoIter = std::iter::Map<
        std::slice::Iter<'a, (Value, Value)>,
        fn(&'a (Value, Value)) -> (&'a Value, &'a Value),
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter().map(|(k, v)| (k, v))
    }
}

impl Value {
    /// The value under `key` when this is a mapping.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_mapping().and_then(|map| map.get(key))
    }

    /// The text of a string scalar.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(text) => Some(text),
            _ => None,
        }
    }

    /// A boolean scalar.
    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(flag) => Some(*flag),
            _ => None,
        }
    }

    /// The entries of a mapping.
    #[must_use]
    pub fn as_mapping(&self) -> Option<&Mapping> {
        match self {
            Value::Mapping(map) => Some(map),
            _ => None,
        }
    }

    /// The items of a sequence.
    #[must_use]
    pub fn as_sequence(&self) -> Option<&Vec<Value>> {
        match self {
            Value::Sequence(items) => Some(items),
            _ => None,
        }
    }
}

/// Missing keys index to [`Value::Null`], as `serde_yaml` did.
impl Index<&str> for Value {
    type Output = Value;

    fn index(&self, key: &str) -> &Value {
        self.get(key).unwrap_or(&NULL)
    }
}

/// Out-of-range indices index to [`Value::Null`].
impl Index<usize> for Value {
    type Output = Value;

    fn index(&self, index: usize) -> &Value {
        self.as_sequence()
            .and_then(|items| items.get(index))
            .unwrap_or(&NULL)
    }
}

/// A YAML syntax error, or more than one document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// The YAML 1.2 core-schema reading of an untagged plain scalar.
fn resolve_plain(text: &str) -> Value {
    match text {
        "" | "~" | "null" | "Null" | "NULL" => return Value::Null,
        "true" | "True" | "TRUE" => return Value::Bool(true),
        "false" | "False" | "FALSE" => return Value::Bool(false),
        ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" | "-.inf" | "-.Inf" | "-.INF"
        | ".nan" | ".NaN" | ".NAN" => return Value::Number(text.to_owned()),
        _ => {}
    }
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    let radix = [("0x", 16), ("0o", 8)]
        .iter()
        .find_map(|(prefix, radix)| Some((unsigned.strip_prefix(prefix)?, *radix)));
    let is_number = match radix {
        Some((digits, radix)) => !digits.is_empty() && u128::from_str_radix(digits, radix).is_ok(),
        None => {
            unsigned.starts_with(|c: char| c.is_ascii_digit() || c == '.')
                && text.parse::<f64>().is_ok_and(f64::is_finite)
        }
    };
    if is_number {
        Value::Number(text.to_owned())
    } else {
        Value::String(text.to_owned())
    }
}

fn scalar(text: String, style: ScalarStyle, tag: Option<&Tag>) -> Value {
    let untagged = |text: String| {
        if style == ScalarStyle::Plain {
            resolve_plain(&text)
        } else {
            Value::String(text)
        }
    };
    match tag {
        Some(tag) if !tag.is_yaml_core_schema() => Value::Tagged(Box::new(TaggedValue {
            tag: format!("{}{}", tag.handle, tag.suffix),
            value: untagged(text),
        })),
        Some(tag) if tag.suffix == "str" => Value::String(text),
        // `!!int '3'` and friends: the tag decides, whatever the quoting.
        Some(_) => resolve_plain(&text),
        None => untagged(text),
    }
}

/// A collection being built.
enum Open {
    Sequence(Vec<Value>, usize, Option<String>),
    Mapping(Vec<(Value, Value)>, Option<Value>, usize, Option<String>),
}

fn tagged(value: Value, tag: Option<String>) -> Value {
    match tag {
        Some(tag) => Value::Tagged(Box::new(TaggedValue { tag, value })),
        None => value,
    }
}

fn custom_tag(tag: Option<&Tag>) -> Option<String> {
    tag.filter(|tag| !tag.is_yaml_core_schema())
        .map(|tag| format!("{}{}", tag.handle, tag.suffix))
}

/// Parses one YAML document (an empty one is [`Value::Null`]).
///
/// # Errors
///
/// [`Error`] for malformed YAML or more than one document.
pub fn from_str(input: &str) -> Result<Value, Error> {
    let mut stack: Vec<Open> = Vec::new();
    let mut anchors: BTreeMap<usize, Value> = BTreeMap::new();
    let mut root = None;
    let mut documents = 0;
    for next in Parser::new_from_str(input) {
        let (event, _) = next.map_err(|err| Error(err.to_string()))?;
        let (node, anchor) = match event {
            Event::DocumentStart(_) => {
                documents += 1;
                if documents > 1 {
                    return Err(Error("more than one YAML document".to_owned()));
                }
                continue;
            }
            Event::SequenceStart(anchor, tag) => {
                stack.push(Open::Sequence(
                    Vec::new(),
                    anchor,
                    custom_tag(tag.as_deref()),
                ));
                continue;
            }
            Event::MappingStart(anchor, tag) => {
                stack.push(Open::Mapping(
                    Vec::new(),
                    None,
                    anchor,
                    custom_tag(tag.as_deref()),
                ));
                continue;
            }
            Event::SequenceEnd | Event::MappingEnd => match stack.pop() {
                Some(Open::Sequence(items, anchor, tag)) => {
                    (tagged(Value::Sequence(items), tag), anchor)
                }
                Some(Open::Mapping(entries, _, anchor, tag)) => {
                    (tagged(Value::Mapping(Mapping(entries)), tag), anchor)
                }
                None => return Err(Error("unbalanced collection end".to_owned())),
            },
            Event::Scalar(text, style, anchor, tag) => {
                (scalar(text.into_owned(), style, tag.as_deref()), anchor)
            }
            Event::Alias(id) => match anchors.get(&id) {
                Some(node) => (node.clone(), 0),
                None => return Err(Error("unknown alias".to_owned())),
            },
            Event::Nothing | Event::StreamStart | Event::StreamEnd | Event::DocumentEnd => {
                continue;
            }
        };
        if anchor != 0 {
            anchors.insert(anchor, node.clone());
        }
        match stack.last_mut() {
            Some(Open::Sequence(items, ..)) => items.push(node),
            Some(Open::Mapping(entries, key, ..)) => match key.take() {
                Some(key) => entries.push((key, node)),
                None => *key = Some(node),
            },
            None => root = Some(node),
        }
    }
    Ok(root.unwrap_or(Value::Null))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn workflow_shapes_load_with_document_order_and_types() {
        let doc = from_str(
            "on: [push]\njobs:\n  b: {steps: [{run: x}], if: true, timeout-minutes: 30}\n  a:\n    \
             needs: b\n    tag: !reference [x]\n    quoted: '1'\n    nothing: ~\n    \
             anchored: &k v\n    alias: *k\n",
        )
        .unwrap();
        assert_eq!(doc["on"][0].as_str(), Some("push"));
        let jobs = doc["jobs"].as_mapping().unwrap();
        let ids: Vec<_> = jobs.keys().filter_map(Value::as_str).collect();
        assert_eq!(ids, ["b", "a"]);
        assert_eq!(doc["jobs"]["b"]["if"].as_bool(), Some(true));
        assert_eq!(
            doc["jobs"]["b"]["timeout-minutes"],
            Value::Number("30".into())
        );
        assert_eq!(doc["jobs"]["b"]["steps"][0]["run"].as_str(), Some("x"));
        let a = &doc["jobs"]["a"];
        assert_eq!(a["needs"].as_str(), Some("b"));
        assert!(matches!(&a["tag"], Value::Tagged(t) if t.tag == "!reference"));
        assert_eq!(a["quoted"].as_str(), Some("1"));
        assert_eq!(a["nothing"], Value::Null);
        assert_eq!(a["alias"].as_str(), Some("v"));
        assert_eq!(a["missing"]["deeper"][3], Value::Null);
        let mut keys = Vec::new();
        for (key, _) in jobs {
            keys.push(key);
        }
        assert_eq!(keys, jobs.keys().collect::<Vec<_>>());
    }

    #[test]
    fn scalars_resolve_by_the_core_schema() {
        for (text, expected) in [
            ("yes", Value::String("yes".into())),
            ("0x1F", Value::Number("0x1F".into())),
            ("-1.5", Value::Number("-1.5".into())),
            (".inf", Value::Number(".inf".into())),
            ("1e3", Value::Number("1e3".into())),
            ("inf", Value::String("inf".into())),
            ("0x", Value::String("0x".into())),
            ("False", Value::Bool(false)),
        ] {
            assert_eq!(resolve_plain(text), expected, "{text:?}");
        }
        assert_eq!(from_str("!!str 12").unwrap(), Value::String("12".into()));
        assert_eq!(from_str("!!int 12").unwrap(), Value::Number("12".into()));
        assert_eq!(from_str("").unwrap(), Value::Null);
    }

    #[test]
    fn malformed_and_multi_document_input_is_an_error() {
        assert!(from_str("a: [1\n").is_err());
        assert!(from_str("a: 1\n---\nb: 2\n").is_err());
        assert!(from_str("a: *nowhere\n").is_err());
    }
}

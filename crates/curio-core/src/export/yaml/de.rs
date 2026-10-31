//! YAML text → `Deserialize`: [`saphyr_parser`] events replayed through a
//! serde deserializer with `serde_yaml` 0.9's typing rules, so a note
//! that parsed before still parses, into the same values.
//!
//! Untyped values (`serde_json::Value` for user keys) resolve plain
//! scalars by [`resolve_plain`]; typed fields read the scalar text
//! directly (`title: 2024` is the string `"2024"`); `!!bool`/`!!int`/
//! `!!float`/`!!null` tags force a type, other core tags (`!!str`) leave
//! the text alone, and custom `!local` tags are refused.

use std::fmt;

use serde::de::{self, DeserializeOwned, DeserializeSeed, IgnoredAny, Visitor};

use super::Error;
use super::load::{Document, Event, Scalar, Style, load};
use super::resolve::{
    Resolved, parse_bool, parse_f64, parse_null, parse_signed_int, parse_unsigned_int, resolve_int,
    resolve_plain,
};

const TAG_NULL: &str = "tag:yaml.org,2002:null";
const TAG_BOOL: &str = "tag:yaml.org,2002:bool";
const TAG_INT: &str = "tag:yaml.org,2002:int";
const TAG_FLOAT: &str = "tag:yaml.org,2002:float";

/// How deep collections may nest.
const MAX_DEPTH: u8 = 128;

/// Alias replays allowed per event — the billion-laughs bound.
const ALIAS_REPLAYS_PER_EVENT: usize = 100;

pub(super) fn from_str<T: DeserializeOwned>(input: &str) -> Result<T, Error> {
    let document = load(input)?;
    let mut pos = 0;
    let mut replays = 0;
    let mut de = Deserializer {
        document: &document,
        pos: &mut pos,
        replays: &mut replays,
        depth: MAX_DEPTH,
    };
    T::deserialize(&mut de)
}

fn is_custom_tag(tag: Option<&String>) -> bool {
    tag.is_some_and(|tag| tag.starts_with('!'))
}

fn custom_tag_error(tag: Option<&String>) -> Error {
    Error::new(format!(
        "custom YAML tag {} is not supported in frontmatter",
        tag.map_or("", String::as_str)
    ))
}

/// Feeds a resolved plain scalar to `visitor`.
fn visit_resolved<'de, V: Visitor<'de>>(
    visitor: V,
    resolved: Resolved<'_>,
) -> Result<V::Value, Error> {
    match resolved {
        Resolved::Null => visitor.visit_unit(),
        Resolved::Bool(v) => visitor.visit_bool(v),
        Resolved::U64(v) => visitor.visit_u64(v),
        Resolved::I64(v) => visitor.visit_i64(v),
        Resolved::U128(v) => visitor.visit_u128(v),
        Resolved::I128(v) => visitor.visit_i128(v),
        Resolved::F64(v) => visitor.visit_f64(v),
        Resolved::Str(v) => visitor.visit_str(v),
    }
}

/// An untyped scalar: core tags force a type, plain text resolves, any
/// other style is a string.
fn visit_scalar<'de, V: Visitor<'de>>(visitor: V, scalar: &Scalar) -> Result<V::Value, Error> {
    let v = scalar.value.as_str();
    match scalar.tag.as_deref() {
        Some(TAG_BOOL) => match parse_bool(v) {
            Some(b) => visitor.visit_bool(b),
            None => Err(de::Error::invalid_value(
                de::Unexpected::Str(v),
                &"a boolean",
            )),
        },
        Some(TAG_INT) => match resolve_int(v) {
            Some(int) => visit_resolved(visitor, int),
            None => Err(de::Error::invalid_value(
                de::Unexpected::Str(v),
                &"an integer",
            )),
        },
        Some(TAG_FLOAT) => match parse_f64(v) {
            Some(f) => visitor.visit_f64(f),
            None => Err(de::Error::invalid_value(de::Unexpected::Str(v), &"a float")),
        },
        Some(TAG_NULL) => {
            if parse_null(v) {
                visitor.visit_unit()
            } else {
                Err(de::Error::invalid_value(de::Unexpected::Str(v), &"null"))
            }
        }
        None if scalar.style == Style::Plain => visit_resolved(visitor, resolve_plain(v)),
        // Other core tags (`!!str`, `!!timestamp`, …) and quoted or block
        // scalars: the text itself.
        _ => visitor.visit_str(v),
    }
}

/// The error for a node of the wrong shape, naming what it is.
fn invalid_type(event: &Event, expected: &dyn de::Expected) -> Error {
    struct Probe<'a>(&'a dyn de::Expected);
    impl Visitor<'_> for Probe<'_> {
        type Value = ();
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.0.fmt(f)
        }
    }
    match event {
        Event::Scalar(scalar) => match visit_scalar(Probe(expected), scalar) {
            Ok(()) => Error::new(format!("invalid type, expected {expected}")),
            Err(err) => err,
        },
        Event::SequenceStart(_) => de::Error::invalid_type(de::Unexpected::Seq, expected),
        Event::MappingStart(_) => de::Error::invalid_type(de::Unexpected::Map, expected),
        Event::Alias(_) | Event::SequenceEnd | Event::MappingEnd | Event::Void => {
            Error::new(format!("unexpected end of YAML, expected {expected}"))
        }
    }
}

/// Typed scalars accept plain text, or a literal block carrying the
/// matching core tag.
fn typed_text<'a>(scalar: &'a Scalar, tag: &str) -> Option<&'a str> {
    match scalar.style {
        Style::Plain => Some(scalar.value.as_str()),
        Style::Literal if scalar.tag.as_deref() == Some(tag) => Some(scalar.value.as_str()),
        _ => None,
    }
}

#[derive(Debug)]
struct Deserializer<'doc> {
    document: &'doc Document,
    /// The next event to read (shared with the parent collection).
    pos: &'doc mut usize,
    replays: &'doc mut usize,
    /// Nesting budget left.
    depth: u8,
}

impl<'doc> Deserializer<'doc> {
    fn peek(&self) -> Result<(&'doc Event, (usize, usize)), Error> {
        self.document
            .events
            .get(*self.pos)
            .map(|(event, location)| (event, *location))
            .ok_or_else(|| Error::new("unexpected end of YAML"))
    }

    fn next(&mut self) -> Result<(&'doc Event, (usize, usize)), Error> {
        let next = self.peek()?;
        *self.pos += 1;
        Ok(next)
    }

    /// A deserializer replaying the node an alias points at.
    fn replay<'a>(&'a mut self, target: &'a mut usize) -> Result<Deserializer<'a>, Error> {
        *self.replays += 1;
        if *self.replays > self.document.events.len() * ALIAS_REPLAYS_PER_EVENT {
            return Err(Error::new("alias expansion limit exceeded"));
        }
        Ok(Deserializer {
            document: self.document,
            pos: target,
            replays: &mut *self.replays,
            depth: self.depth,
        })
    }

    fn child(&mut self) -> Deserializer<'_> {
        Deserializer {
            document: self.document,
            pos: &mut *self.pos,
            replays: &mut *self.replays,
            depth: self.depth,
        }
    }

    fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, Error>) -> Result<T, Error> {
        let previous = self.depth;
        self.depth = previous
            .checked_sub(1)
            .ok_or_else(|| Error::new("frontmatter nests too deeply"))?;
        let result = f(self);
        self.depth = previous;
        result
    }

    fn visit_sequence<'de, V: Visitor<'de>>(&mut self, visitor: V) -> Result<V::Value, Error> {
        let (value, len) = self.nested(|de| {
            let mut access = SeqAccess {
                de,
                len: 0,
                empty: false,
            };
            let value = visitor.visit_seq(&mut access)?;
            Ok((value, access.len))
        })?;
        let mut rest = SeqAccess {
            de: self,
            len,
            empty: false,
        };
        while de::SeqAccess::next_element::<IgnoredAny>(&mut rest)?.is_some() {}
        let total = rest.len;
        self.next()?; // SequenceEnd
        if total == len {
            Ok(value)
        } else {
            Err(de::Error::invalid_length(
                total,
                &format!("{len} elements").as_str(),
            ))
        }
    }

    fn visit_mapping<'de, V: Visitor<'de>>(&mut self, visitor: V) -> Result<V::Value, Error> {
        let (value, len) = self.nested(|de| {
            let mut access = MapAccess {
                de,
                len: 0,
                empty: false,
            };
            let value = visitor.visit_map(&mut access)?;
            Ok((value, access.len))
        })?;
        let mut rest = MapAccess {
            de: self,
            len,
            empty: false,
        };
        while de::MapAccess::next_entry::<IgnoredAny, IgnoredAny>(&mut rest)?.is_some() {}
        let total = rest.len;
        self.next()?; // MappingEnd
        if total == len {
            Ok(value)
        } else {
            Err(de::Error::invalid_length(
                total,
                &format!("{len} entries").as_str(),
            ))
        }
    }

    fn ignore_any(&mut self) -> Result<(), Error> {
        let mut depth = 0_usize;
        loop {
            match self.next()?.0 {
                Event::SequenceStart(_) | Event::MappingStart(_) => depth += 1,
                Event::SequenceEnd | Event::MappingEnd => depth = depth.saturating_sub(1),
                Event::Alias(_) | Event::Scalar(_) | Event::Void => {}
            }
            if depth == 0 {
                return Ok(());
            }
        }
    }

    /// A typed scalar read: `parse` on the accepted text, else a type
    /// error naming the node.
    fn typed<'de, V: Visitor<'de>>(
        &mut self,
        visitor: V,
        (tag, expected): (&str, &str),
        parse: impl FnOnce(&str, V) -> Option<Result<V::Value, Error>>,
        again: impl FnOnce(&mut Deserializer<'_>, V) -> Result<V::Value, Error>,
    ) -> Result<V::Value, Error> {
        let (event, location) = self.next()?;
        let parsed = match event {
            Event::Alias(target) => {
                let mut target = *target;
                return again(&mut self.replay(&mut target)?, visitor)
                    .map_err(|err| err.at(location));
            }
            Event::Scalar(scalar) => typed_text(scalar, tag).and_then(|text| parse(text, visitor)),
            _ => None,
        };
        parsed
            .unwrap_or_else(|| Err(invalid_type(event, &expected)))
            .map_err(|err| err.at(location))
    }
}

struct SeqAccess<'a, 'doc> {
    de: &'a mut Deserializer<'doc>,
    len: usize,
    empty: bool,
}

impl<'de> de::SeqAccess<'de> for SeqAccess<'_, '_> {
    type Error = Error;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Error> {
        if self.empty {
            return Ok(None);
        }
        match self.de.peek()?.0 {
            Event::SequenceEnd | Event::Void => Ok(None),
            _ => {
                self.len += 1;
                seed.deserialize(&mut self.de.child()).map(Some)
            }
        }
    }
}

struct MapAccess<'a, 'doc> {
    de: &'a mut Deserializer<'doc>,
    len: usize,
    empty: bool,
}

impl<'de> de::MapAccess<'de> for MapAccess<'_, '_> {
    type Error = Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Error> {
        if self.empty {
            return Ok(None);
        }
        match self.de.peek()?.0 {
            Event::MappingEnd | Event::Void => Ok(None),
            _ => {
                self.len += 1;
                seed.deserialize(&mut *self.de).map(Some)
            }
        }
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Error> {
        seed.deserialize(&mut self.de.child())
    }
}

macro_rules! forward_signed {
    ($($method:ident),*) => {$(
        fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
            self.deserialize_i64(visitor)
        }
    )*};
}

macro_rules! forward_unsigned {
    ($($method:ident),*) => {$(
        fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
            self.deserialize_u64(visitor)
        }
    )*};
}

impl<'de> de::Deserializer<'de> for &mut Deserializer<'_> {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        let (event, location) = self.next()?;
        match event {
            Event::Alias(target) => {
                let mut target = *target;
                self.replay(&mut target)?.deserialize_any(visitor)
            }
            Event::Scalar(scalar) if is_custom_tag(scalar.tag.as_ref()) => {
                Err(custom_tag_error(scalar.tag.as_ref()))
            }
            Event::Scalar(scalar) => visit_scalar(visitor, scalar),
            Event::SequenceStart(tag) | Event::MappingStart(tag) if is_custom_tag(tag.as_ref()) => {
                Err(custom_tag_error(tag.as_ref()))
            }
            Event::SequenceStart(_) => self.visit_sequence(visitor),
            Event::MappingStart(_) => self.visit_mapping(visitor),
            Event::Void => visitor.visit_none(),
            Event::SequenceEnd | Event::MappingEnd => {
                Err(Error::new("unexpected end of a collection"))
            }
        }
        .map_err(|err| err.at(location))
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.typed(
            visitor,
            (TAG_BOOL, "a boolean"),
            |text, visitor| parse_bool(text).map(|b| visitor.visit_bool(b)),
            |de, visitor| de.deserialize_bool(visitor),
        )
    }

    forward_signed!(deserialize_i8, deserialize_i16, deserialize_i32);

    fn deserialize_i64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.typed(
            visitor,
            (TAG_INT, "an integer"),
            |text, visitor| {
                parse_signed_int(text, i64::from_str_radix).map(|i| visitor.visit_i64(i))
            },
            |de, visitor| de.deserialize_i64(visitor),
        )
    }

    fn deserialize_i128<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.typed(
            visitor,
            (TAG_INT, "an integer"),
            |text, visitor| {
                parse_signed_int(text, i128::from_str_radix).map(|i| visitor.visit_i128(i))
            },
            |de, visitor| de.deserialize_i128(visitor),
        )
    }

    forward_unsigned!(deserialize_u8, deserialize_u16, deserialize_u32);

    fn deserialize_u64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.typed(
            visitor,
            (TAG_INT, "an integer"),
            |text, visitor| {
                parse_unsigned_int(text, u64::from_str_radix).map(|u| visitor.visit_u64(u))
            },
            |de, visitor| de.deserialize_u64(visitor),
        )
    }

    fn deserialize_u128<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.typed(
            visitor,
            (TAG_INT, "an integer"),
            |text, visitor| {
                parse_unsigned_int(text, u128::from_str_radix).map(|u| visitor.visit_u128(u))
            },
            |de, visitor| de.deserialize_u128(visitor),
        )
    }

    fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_f64(visitor)
    }

    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.typed(
            visitor,
            (TAG_FLOAT, "a float"),
            |text, visitor| parse_f64(text).map(|f| visitor.visit_f64(f)),
            |de, visitor| de.deserialize_f64(visitor),
        )
    }

    fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_str(visitor)
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        let (event, location) = self.next()?;
        match event {
            Event::Scalar(scalar) => visitor.visit_str(&scalar.value),
            Event::Alias(target) => {
                let mut target = *target;
                self.replay(&mut target)?.deserialize_str(visitor)
            }
            other => Err(invalid_type(other, &visitor)),
        }
        .map_err(|err| err.at(location))
    }

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_str(visitor)
    }

    fn deserialize_bytes<V: Visitor<'de>>(self, _visitor: V) -> Result<V::Value, Error> {
        Err(Error::new("byte strings are not supported in frontmatter"))
    }

    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_bytes(visitor)
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        let (event, location) = self.peek()?;
        let is_some = match event {
            Event::Alias(target) => {
                *self.pos += 1;
                let mut target = *target;
                return self.replay(&mut target)?.deserialize_option(visitor);
            }
            Event::Scalar(scalar) if scalar.style != Style::Plain => true,
            Event::Scalar(Scalar {
                value,
                tag: Some(tag),
                ..
            }) if tag == TAG_NULL => {
                if parse_null(value) {
                    false
                } else {
                    return Err(de::Error::invalid_value(
                        de::Unexpected::Str(value),
                        &"null",
                    ))
                    .map_err(|err: Error| err.at(location));
                }
            }
            Event::Scalar(Scalar { tag: Some(_), .. })
            | Event::SequenceStart(_)
            | Event::MappingStart(_) => true,
            Event::Scalar(scalar) => !scalar.value.is_empty() && !parse_null(&scalar.value),
            Event::Void => false,
            Event::SequenceEnd | Event::MappingEnd => {
                return Err(Error::new("unexpected end of a collection").at(location));
            }
        };
        if is_some {
            visitor.visit_some(self)
        } else {
            *self.pos += 1;
            visitor.visit_none()
        }
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        let (event, location) = self.next()?;
        match event {
            Event::Scalar(scalar) => {
                let is_null = scalar.style == Style::Plain
                    && match scalar.tag.as_deref() {
                        Some(tag) => tag == TAG_NULL && parse_null(&scalar.value),
                        None => scalar.value.is_empty() || parse_null(&scalar.value),
                    };
                if is_null {
                    visitor.visit_unit()
                } else {
                    Err(de::Error::invalid_value(
                        de::Unexpected::Str(&scalar.value),
                        &"null",
                    ))
                }
            }
            Event::Alias(target) => {
                let mut target = *target;
                self.replay(&mut target)?.deserialize_unit(visitor)
            }
            Event::Void => visitor.visit_unit(),
            other => Err(invalid_type(other, &visitor)),
        }
        .map_err(|err| err.at(location))
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.deserialize_unit(visitor)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.nested(|de| visitor.visit_newtype_struct(de))
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        let (event, location) = self.next()?;
        match event {
            Event::Alias(target) => {
                let mut target = *target;
                self.replay(&mut target)?.deserialize_seq(visitor)
            }
            Event::SequenceStart(_) => self.visit_sequence(visitor),
            Event::Void => visitor.visit_seq(SeqAccess {
                de: self,
                len: 0,
                empty: true,
            }),
            Event::Scalar(scalar) if scalar.value.is_empty() && scalar.style == Style::Plain => {
                visitor.visit_seq(SeqAccess {
                    de: self,
                    len: 0,
                    empty: true,
                })
            }
            other => Err(invalid_type(other, &visitor)),
        }
        .map_err(|err| err.at(location))
    }

    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        let (event, location) = self.next()?;
        match event {
            Event::Alias(target) => {
                let mut target = *target;
                self.replay(&mut target)?.deserialize_map(visitor)
            }
            Event::MappingStart(_) => self.visit_mapping(visitor),
            Event::Void => visitor.visit_map(MapAccess {
                de: self,
                len: 0,
                empty: true,
            }),
            Event::Scalar(scalar) if scalar.value.is_empty() && scalar.style == Style::Plain => {
                visitor.visit_map(MapAccess {
                    de: self,
                    len: 0,
                    empty: true,
                })
            }
            other => Err(invalid_type(other, &visitor)),
        }
        .map_err(|err| err.at(location))
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.deserialize_map(visitor)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, Error> {
        Err(Error::new("enums are not supported in frontmatter"))
    }

    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_str(visitor)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.ignore_any()?;
        visitor.visit_unit()
    }
}

#[cfg(test)]
#[path = "de_tests.rs"]
mod tests;

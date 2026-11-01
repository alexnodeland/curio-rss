//! `Serialize` → [`Node`] tree, choosing each scalar's requested style the
//! way `serde_yaml` 0.9 did (the emitter may still escalate it).

use serde::ser::{self, Serialize};

use super::Error;
use super::resolve::needs_quotes;

/// The style a scalar asks the emitter for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Style {
    /// Plain if the content allows it, else quoted.
    Any,
    /// Numbers, booleans and null: plain unless the content forbids it.
    Plain,
    /// Strings that would read back as something else.
    SingleQuoted,
    /// Strings containing `\n`.
    Literal,
}

/// A serialized YAML node.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Node {
    Scalar(String, Style),
    Seq(Vec<Node>),
    Map(Vec<(Node, Node)>),
}

fn plain(value: impl Into<String>) -> Node {
    Node::Scalar(value.into(), Style::Plain)
}

fn string(value: &str) -> Node {
    let style = if value.contains('\n') {
        Style::Literal
    } else if needs_quotes(value) {
        Style::SingleQuoted
    } else {
        Style::Any
    };
    Node::Scalar(value.to_owned(), style)
}

fn float(value: f64, finite: impl FnOnce(&mut ryu::Buffer) -> String) -> Node {
    if value.is_nan() {
        plain(".nan")
    } else if value.is_infinite() {
        plain(if value.is_sign_positive() {
            ".inf"
        } else {
            "-.inf"
        })
    } else {
        plain(finite(&mut ryu::Buffer::new()))
    }
}

fn unsupported(what: &str) -> Error {
    Error::new(format!("{what} cannot be serialized as frontmatter YAML"))
}

/// The serializer: every value becomes a [`Node`].
#[derive(Debug, Clone, Copy)]
pub(super) struct NodeSerializer;

/// Collects sequence elements.
#[derive(Debug)]
pub(super) struct SeqCollector(Vec<Node>);

/// Collects mapping entries (and struct fields).
#[derive(Debug)]
pub(super) struct MapCollector {
    entries: Vec<(Node, Node)>,
    key: Option<Node>,
}

impl ser::Serializer for NodeSerializer {
    type Ok = Node;
    type Error = Error;
    type SerializeSeq = SeqCollector;
    type SerializeTuple = SeqCollector;
    type SerializeTupleStruct = SeqCollector;
    type SerializeTupleVariant = ser::Impossible<Node, Error>;
    type SerializeMap = MapCollector;
    type SerializeStruct = MapCollector;
    type SerializeStructVariant = ser::Impossible<Node, Error>;

    fn serialize_bool(self, v: bool) -> Result<Node, Error> {
        Ok(plain(if v { "true" } else { "false" }))
    }

    fn serialize_i8(self, v: i8) -> Result<Node, Error> {
        Ok(plain(v.to_string()))
    }

    fn serialize_i16(self, v: i16) -> Result<Node, Error> {
        Ok(plain(v.to_string()))
    }

    fn serialize_i32(self, v: i32) -> Result<Node, Error> {
        Ok(plain(v.to_string()))
    }

    fn serialize_i64(self, v: i64) -> Result<Node, Error> {
        Ok(plain(v.to_string()))
    }

    fn serialize_i128(self, v: i128) -> Result<Node, Error> {
        Ok(plain(v.to_string()))
    }

    fn serialize_u8(self, v: u8) -> Result<Node, Error> {
        Ok(plain(v.to_string()))
    }

    fn serialize_u16(self, v: u16) -> Result<Node, Error> {
        Ok(plain(v.to_string()))
    }

    fn serialize_u32(self, v: u32) -> Result<Node, Error> {
        Ok(plain(v.to_string()))
    }

    fn serialize_u64(self, v: u64) -> Result<Node, Error> {
        Ok(plain(v.to_string()))
    }

    fn serialize_u128(self, v: u128) -> Result<Node, Error> {
        Ok(plain(v.to_string()))
    }

    fn serialize_f32(self, v: f32) -> Result<Node, Error> {
        Ok(float(f64::from(v), |buffer| {
            buffer.format_finite(v).to_owned()
        }))
    }

    fn serialize_f64(self, v: f64) -> Result<Node, Error> {
        Ok(float(v, |buffer| buffer.format_finite(v).to_owned()))
    }

    fn serialize_char(self, v: char) -> Result<Node, Error> {
        Ok(Node::Scalar(v.to_string(), Style::SingleQuoted))
    }

    fn serialize_str(self, v: &str) -> Result<Node, Error> {
        Ok(string(v))
    }

    fn serialize_bytes(self, _v: &[u8]) -> Result<Node, Error> {
        Err(unsupported("a byte string"))
    }

    fn serialize_none(self) -> Result<Node, Error> {
        Ok(plain("null"))
    }

    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<Node, Error> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Node, Error> {
        Ok(plain("null"))
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Node, Error> {
        Ok(plain("null"))
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
    ) -> Result<Node, Error> {
        Ok(string(variant))
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Node, Error> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<Node, Error> {
        Err(unsupported("an enum variant with data"))
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<SeqCollector, Error> {
        Ok(SeqCollector(Vec::with_capacity(len.unwrap_or(0))))
    }

    fn serialize_tuple(self, len: usize) -> Result<SeqCollector, Error> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<SeqCollector, Error> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Error> {
        Err(unsupported("an enum variant with data"))
    }

    fn serialize_map(self, len: Option<usize>) -> Result<MapCollector, Error> {
        Ok(MapCollector {
            entries: Vec::with_capacity(len.unwrap_or(0)),
            key: None,
        })
    }

    fn serialize_struct(self, _name: &'static str, len: usize) -> Result<MapCollector, Error> {
        self.serialize_map(Some(len))
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Error> {
        Err(unsupported("an enum variant with data"))
    }
}

impl ser::SerializeSeq for SeqCollector {
    type Ok = Node;
    type Error = Error;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Error> {
        self.0.push(value.serialize(NodeSerializer)?);
        Ok(())
    }

    fn end(self) -> Result<Node, Error> {
        Ok(Node::Seq(self.0))
    }
}

impl ser::SerializeTuple for SeqCollector {
    type Ok = Node;
    type Error = Error;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Error> {
        ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Node, Error> {
        ser::SerializeSeq::end(self)
    }
}

impl ser::SerializeTupleStruct for SeqCollector {
    type Ok = Node;
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Error> {
        ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Node, Error> {
        ser::SerializeSeq::end(self)
    }
}

impl ser::SerializeMap for MapCollector {
    type Ok = Node;
    type Error = Error;

    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), Error> {
        self.key = Some(key.serialize(NodeSerializer)?);
        Ok(())
    }

    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Error> {
        let key = self
            .key
            .take()
            .ok_or_else(|| Error::new("map value serialized before its key"))?;
        self.entries.push((key, value.serialize(NodeSerializer)?));
        Ok(())
    }

    fn end(self) -> Result<Node, Error> {
        Ok(Node::Map(self.entries))
    }
}

impl ser::SerializeStruct for MapCollector {
    type Ok = Node;
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        self.entries
            .push((string(key), value.serialize(NodeSerializer)?));
        Ok(())
    }

    fn end(self) -> Result<Node, Error> {
        Ok(Node::Map(self.entries))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn scalars_request_the_serde_yaml_styles() {
        let node = |v: &dyn erased::Ser| v.ser();
        assert_eq!(node(&true), Node::Scalar("true".into(), Style::Plain));
        assert_eq!(node(&-3_i8), Node::Scalar("-3".into(), Style::Plain));
        assert_eq!(node(&1.5_f32), Node::Scalar("1.5".into(), Style::Plain));
        assert_eq!(node(&f64::NAN), Node::Scalar(".nan".into(), Style::Plain));
        assert_eq!(
            node(&f64::NEG_INFINITY),
            Node::Scalar("-.inf".into(), Style::Plain)
        );
        assert_eq!(node(&'x'), Node::Scalar("x".into(), Style::SingleQuoted));
        assert_eq!(node(&"a\nb"), Node::Scalar("a\nb".into(), Style::Literal));
        assert_eq!(node(&"42"), Node::Scalar("42".into(), Style::SingleQuoted));
        assert_eq!(node(&"text"), Node::Scalar("text".into(), Style::Any));
        assert_eq!(node(&()), Node::Scalar("null".into(), Style::Plain));
        assert_eq!(
            node(&(1_u8, 2_u8)),
            Node::Seq(vec![node(&1_u8), node(&2_u8)])
        );
    }

    #[test]
    fn data_carrying_enums_and_bytes_are_refused() {
        #[derive(serde::Serialize)]
        enum E {
            Unit,
            New(u8),
            Tuple(u8, u8),
            Struct { a: u8 },
        }
        assert_eq!(
            E::Unit.serialize(NodeSerializer).unwrap(),
            Node::Scalar("Unit".into(), Style::Any)
        );
        assert!(E::New(1).serialize(NodeSerializer).is_err());
        assert!(E::Tuple(1, 2).serialize(NodeSerializer).is_err());
        assert!(E::Struct { a: 1 }.serialize(NodeSerializer).is_err());
        let bytes = serde_bytes_like(&[1, 2]);
        assert!(bytes.is_err());
    }

    fn serde_bytes_like(bytes: &[u8]) -> Result<Node, Error> {
        ser::Serializer::serialize_bytes(NodeSerializer, bytes)
    }

    /// Object-safe shim so the table above can mix value types.
    mod erased {
        use super::{Node, NodeSerializer, Serialize};

        pub(super) trait Ser {
            fn ser(&self) -> Node;
        }

        impl<T: Serialize> Ser for T {
            #[allow(clippy::unwrap_used)] // test helper
            fn ser(&self) -> Node {
                self.serialize(NodeSerializer).unwrap()
            }
        }
    }
}

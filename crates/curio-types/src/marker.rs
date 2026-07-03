//! Schema discriminator markers.
//!
//! Every published contract document carries an in-band `schema` field whose
//! value is a fixed literal. These zero-sized markers serialize as exactly
//! that literal and refuse to deserialize anything else, so a document with
//! the wrong (or missing) `schema` value fails at the type level.

macro_rules! schema_marker {
    ($(#[$meta:meta])* $name:ident => $lit:expr) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
        pub struct $name;

        impl $name {
            /// The literal `schema` value this marker stands for.
            pub const LITERAL: &'static str = $lit;
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str($lit)
            }
        }

        impl ::serde::Serialize for $name {
            fn serialize<S: ::serde::Serializer>(
                &self,
                serializer: S,
            ) -> Result<S::Ok, S::Error> {
                serializer.serialize_str($lit)
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(
                deserializer: D,
            ) -> Result<Self, D::Error> {
                let s = <::std::string::String as ::serde::Deserialize>::deserialize(
                    deserializer,
                )?;
                if s == $lit {
                    Ok(Self)
                } else {
                    Err(<D::Error as ::serde::de::Error>::invalid_value(
                        ::serde::de::Unexpected::Str(&s),
                        &concat!("the literal \"", $lit, "\""),
                    ))
                }
            }
        }
    };
}

schema_marker!(
    /// The literal `curio.frontmatter.v1` — the `schema` discriminator of
    /// exported-note frontmatter.
    FrontmatterSchemaV1Marker => "curio.frontmatter.v1"
);

schema_marker!(
    /// The literal `curio.events.v1` — the `schema` discriminator of every
    /// event envelope.
    EventsSchemaV1Marker => "curio.events.v1"
);

schema_marker!(
    /// The literal `curio.manifest.v1` — the `schema` discriminator of the
    /// per-destination export manifest.
    ManifestSchemaV1Marker => "curio.manifest.v1"
);

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn markers_serialize_as_their_literals() {
        assert_eq!(
            serde_json::to_value(FrontmatterSchemaV1Marker).unwrap(),
            serde_json::json!("curio.frontmatter.v1")
        );
        assert_eq!(
            serde_json::to_value(EventsSchemaV1Marker).unwrap(),
            serde_json::json!("curio.events.v1")
        );
        assert_eq!(
            serde_json::to_value(ManifestSchemaV1Marker).unwrap(),
            serde_json::json!("curio.manifest.v1")
        );
    }

    #[test]
    fn markers_reject_wrong_literals() {
        assert!(
            serde_json::from_value::<EventsSchemaV1Marker>(serde_json::json!("curio.events.v2"))
                .is_err()
        );
        assert!(
            serde_json::from_value::<FrontmatterSchemaV1Marker>(serde_json::json!(42)).is_err()
        );
    }
}

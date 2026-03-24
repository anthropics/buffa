//! Linked descriptor types for runtime reflection.
//!
//! These are the processed, feature-resolved form of the raw
//! [`FileDescriptorProto`](crate::generated::descriptor::FileDescriptorProto)
//! tree.  Where the raw protos use string `type_name` references and
//! unresolved `FeatureSet` options, these types use pool indices
//! ([`MessageIndex`], [`EnumIndex`]) and pre-resolved edition features
//! ([`FieldPresence`](buffa::editions::FieldPresence), `packed`, `delimited`).
//!
//! [`FieldKind`] flattens protobuf's orthogonal type × label × map-entry axes
//! into a single discriminant that maps 1:1 to runtime representation — the
//! same approach protobuf-es takes with its `fieldKind` union.
//!
//! These types are constructed by `DescriptorPool` (forthcoming) from a
//! `FileDescriptorSet` and are immutable thereafter.  The `#[non_exhaustive]`
//! attribute prevents construction outside this crate; read the `pub` fields
//! directly.  Mutation through `&mut` is unsupported — the pool hands out
//! shared references only.
//!
//! # Limits
//!
//! Field indices within a message are stored as `u16`, capping the number of
//! fields per message at 65 535.  `DescriptorPool` enforces this at
//! construction time.  Field *numbers* remain `u32` per the protobuf spec.

use alloc::string::String;
use alloc::vec::Vec;

use crate::generated::descriptor::field_descriptor_proto::Type as ProtoType;
use buffa::editions::{EnumType, FieldPresence};

/// Index of a [`MessageDescriptor`] within its owning pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MessageIndex(pub(crate) u32);

/// Index of an [`EnumDescriptor`] within its owning pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EnumIndex(pub(crate) u32);

/// Protobuf scalar field types.
///
/// This is [`field_descriptor_proto::Type`](ProtoType) minus
/// `TYPE_MESSAGE`, `TYPE_GROUP`, and `TYPE_ENUM` — those get dedicated
/// [`SingularKind`] variants instead of being lumped in with scalars.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScalarType {
    Double,
    Float,
    Int64,
    Uint64,
    Int32,
    Fixed64,
    Fixed32,
    Bool,
    String,
    Bytes,
    Uint32,
    Sfixed32,
    Sfixed64,
    Sint32,
    Sint64,
}

impl ScalarType {
    /// Convert a raw proto `Type` to a `ScalarType`.
    ///
    /// Returns `None` for `TYPE_MESSAGE`, `TYPE_GROUP`, and `TYPE_ENUM`,
    /// which are not scalar.
    pub fn from_proto(ty: ProtoType) -> Option<Self> {
        Some(match ty {
            ProtoType::TYPE_DOUBLE => Self::Double,
            ProtoType::TYPE_FLOAT => Self::Float,
            ProtoType::TYPE_INT64 => Self::Int64,
            ProtoType::TYPE_UINT64 => Self::Uint64,
            ProtoType::TYPE_INT32 => Self::Int32,
            ProtoType::TYPE_FIXED64 => Self::Fixed64,
            ProtoType::TYPE_FIXED32 => Self::Fixed32,
            ProtoType::TYPE_BOOL => Self::Bool,
            ProtoType::TYPE_STRING => Self::String,
            ProtoType::TYPE_BYTES => Self::Bytes,
            ProtoType::TYPE_UINT32 => Self::Uint32,
            ProtoType::TYPE_SFIXED32 => Self::Sfixed32,
            ProtoType::TYPE_SFIXED64 => Self::Sfixed64,
            ProtoType::TYPE_SINT32 => Self::Sint32,
            ProtoType::TYPE_SINT64 => Self::Sint64,
            ProtoType::TYPE_MESSAGE | ProtoType::TYPE_GROUP | ProtoType::TYPE_ENUM => return None,
        })
    }

    /// Whether this scalar is valid as a protobuf map key.
    ///
    /// Per the protobuf spec: integral types, bool, and string. Not floats,
    /// not bytes.
    pub fn is_valid_map_key(self) -> bool {
        !matches!(self, Self::Double | Self::Float | Self::Bytes)
    }
}

/// The element kind of a singular field, list element, or map value.
///
/// Separating this from [`FieldKind`] makes `List(List(...))` and
/// `Map { value: Map {..} }` unrepresentable — protobuf does not allow
/// nested repeated or map-of-map.  It also keeps [`FieldKind`] `Copy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SingularKind {
    /// A scalar value.
    Scalar(ScalarType),
    /// An enum value, referencing an enum in the pool.
    Enum(EnumIndex),
    /// A message value, referencing a message in the pool.
    Message(MessageIndex),
}

/// The kind of a protobuf field, flattening type × cardinality × map-entry.
///
/// This discriminant maps 1:1 to the field's runtime representation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FieldKind {
    /// A singular (non-repeated, non-map) field.
    Singular(SingularKind),
    /// A `repeated` field.
    List(SingularKind),
    /// A `map<K, V>` field.
    Map {
        /// Key type. Always integral, bool, or string per the protobuf spec.
        key: ScalarType,
        /// Value kind.
        value: SingularKind,
    },
}

/// A linked, feature-resolved field descriptor.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct FieldDescriptor {
    /// Proto field name (as written in the `.proto` file).
    pub name: String,
    /// JSON name — lowerCamelCase unless overridden by `[json_name = ...]`.
    pub json_name: String,
    /// Field number.
    pub number: u32,
    /// Resolved kind (scalar/enum/message/list/map).
    pub kind: FieldKind,
    /// Resolved presence discipline.  For `List`/`Map` kinds this is
    /// always [`Implicit`](FieldPresence::Implicit) (repeated fields have
    /// no presence).
    pub presence: FieldPresence,
    /// Whether a `List` of packable scalars uses packed wire encoding.
    /// Meaningless for non-list or non-packable kinds.
    pub packed: bool,
    /// Whether a `Message` kind uses delimited (group-style) wire encoding.
    /// Meaningless for non-message kinds.
    pub delimited: bool,
    /// Index into the parent message's `oneofs`, if this field belongs to
    /// a oneof (including proto3 synthetic oneofs for `optional`).
    pub oneof_index: Option<u16>,
}

/// A linked message descriptor.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct MessageDescriptor {
    /// Fully-qualified proto name without leading dot, e.g.
    /// `google.protobuf.Timestamp`.
    pub full_name: String,
    /// Fields in source (declaration) order.
    pub fields: Vec<FieldDescriptor>,
    /// `(field_number, index_into_fields)` pairs, sorted by field number
    /// for binary-search lookup.
    pub field_by_number: Vec<(u32, u16)>,
    /// Oneof declarations, including proto3 synthetic oneofs.
    pub oneofs: Vec<OneofDescriptor>,
    /// Extension ranges `[start, end)`.
    pub extension_ranges: Vec<(u32, u32)>,
}

impl MessageDescriptor {
    /// Look up a field by its proto field number.
    pub fn field(&self, number: u32) -> Option<&FieldDescriptor> {
        let i = self
            .field_by_number
            .binary_search_by_key(&number, |&(n, _)| n)
            .ok()?;
        let (_, idx) = self.field_by_number[i];
        debug_assert!(
            (idx as usize) < self.fields.len(),
            "field_by_number index {idx} out of bounds for {} fields",
            self.fields.len()
        );
        self.fields.get(idx as usize)
    }

    /// Whether `number` falls within any declared extension range.
    pub fn in_extension_range(&self, number: u32) -> bool {
        self.extension_ranges
            .iter()
            .any(|&(start, end)| start <= number && number < end)
    }
}

/// A oneof declaration within a message.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct OneofDescriptor {
    /// Proto oneof name.
    pub name: String,
    /// Indices into the parent message's `fields` for members of this oneof.
    pub field_indices: Vec<u16>,
    /// Whether this is a synthetic oneof generated for a proto3 `optional`
    /// field (exactly one member, not user-declared).
    pub synthetic: bool,
}

/// A linked enum descriptor.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct EnumDescriptor {
    /// Fully-qualified proto name without leading dot.
    pub full_name: String,
    /// Declared values in source order.
    pub values: Vec<EnumValueDescriptor>,
    /// Whether unknown numeric values are preserved
    /// ([`Open`](EnumType::Open)) or treated as unknown fields
    /// ([`Closed`](EnumType::Closed)). Resolved from edition features.
    pub enum_type: EnumType,
}

impl EnumDescriptor {
    /// Look up a value by its numeric value.
    ///
    /// If the enum has aliases (`allow_alias = true`), returns the first
    /// declared value with that number.
    pub fn value(&self, number: i32) -> Option<&EnumValueDescriptor> {
        self.values.iter().find(|v| v.number == number)
    }

    /// Look up a value by its proto name.
    pub fn value_by_name(&self, name: &str) -> Option<&EnumValueDescriptor> {
        self.values.iter().find(|v| v.name == name)
    }
}

/// A single value within an enum.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct EnumValueDescriptor {
    /// Proto value name, e.g. `FOO_BAR`.
    pub name: String,
    /// Numeric value.
    pub number: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_type_from_proto_scalars() {
        assert_eq!(
            ScalarType::from_proto(ProtoType::TYPE_INT32),
            Some(ScalarType::Int32)
        );
        assert_eq!(
            ScalarType::from_proto(ProtoType::TYPE_STRING),
            Some(ScalarType::String)
        );
        assert_eq!(
            ScalarType::from_proto(ProtoType::TYPE_SINT64),
            Some(ScalarType::Sint64)
        );
    }

    #[test]
    fn scalar_type_from_proto_rejects_composites() {
        assert_eq!(ScalarType::from_proto(ProtoType::TYPE_MESSAGE), None);
        assert_eq!(ScalarType::from_proto(ProtoType::TYPE_GROUP), None);
        assert_eq!(ScalarType::from_proto(ProtoType::TYPE_ENUM), None);
    }

    #[test]
    fn scalar_type_map_key_validity() {
        assert!(ScalarType::Int32.is_valid_map_key());
        assert!(ScalarType::String.is_valid_map_key());
        assert!(ScalarType::Bool.is_valid_map_key());
        assert!(ScalarType::Sfixed64.is_valid_map_key());
        assert!(!ScalarType::Double.is_valid_map_key());
        assert!(!ScalarType::Float.is_valid_map_key());
        assert!(!ScalarType::Bytes.is_valid_map_key());
    }

    fn scalar_field(name: &str, number: u32, ty: ScalarType) -> FieldDescriptor {
        FieldDescriptor {
            name: name.into(),
            json_name: name.into(),
            number,
            kind: FieldKind::Singular(SingularKind::Scalar(ty)),
            presence: FieldPresence::Implicit,
            packed: false,
            delimited: false,
            oneof_index: None,
        }
    }

    fn sample_message() -> MessageDescriptor {
        MessageDescriptor {
            full_name: "test.Foo".into(),
            fields: alloc::vec![
                scalar_field("a", 1, ScalarType::Int32),
                scalar_field("b", 5, ScalarType::String),
            ],
            field_by_number: alloc::vec![(1, 0), (5, 1)],
            oneofs: Vec::new(),
            extension_ranges: alloc::vec![(100, 200), (1000, 2000)],
        }
    }

    #[test]
    fn message_field_lookup_by_number() {
        let m = sample_message();
        assert_eq!(m.field(1).unwrap().name, "a");
        assert_eq!(m.field(5).unwrap().name, "b");
        assert!(m.field(2).is_none());
        assert!(m.field(99).is_none());
    }

    #[test]
    fn empty_message_field_lookup() {
        let m = MessageDescriptor {
            full_name: "test.Empty".into(),
            fields: Vec::new(),
            field_by_number: Vec::new(),
            oneofs: Vec::new(),
            extension_ranges: Vec::new(),
        };
        assert!(m.field(1).is_none());
        assert!(!m.in_extension_range(1));
    }

    #[test]
    fn message_extension_range_check() {
        let m = sample_message();
        assert!(m.in_extension_range(100));
        assert!(m.in_extension_range(150));
        assert!(m.in_extension_range(199));
        assert!(!m.in_extension_range(200)); // end is exclusive
        assert!(m.in_extension_range(1500));
        assert!(!m.in_extension_range(50));
        assert!(!m.in_extension_range(500));
    }

    #[test]
    fn enum_value_lookup() {
        let e = EnumDescriptor {
            full_name: "test.Color".into(),
            values: alloc::vec![
                EnumValueDescriptor {
                    name: "RED".into(),
                    number: 0
                },
                EnumValueDescriptor {
                    name: "GREEN".into(),
                    number: 1
                },
                EnumValueDescriptor {
                    name: "ALIAS_RED".into(),
                    number: 0
                },
            ],
            enum_type: EnumType::Open,
        };
        assert_eq!(e.value(1).unwrap().name, "GREEN");
        assert_eq!(e.value(0).unwrap().name, "RED"); // first wins on alias
        assert!(e.value(99).is_none());
        assert_eq!(e.value_by_name("GREEN").unwrap().number, 1);
        assert!(e.value_by_name("BLUE").is_none());
    }

    #[test]
    fn field_kind_is_copy() {
        let list = FieldKind::List(SingularKind::Message(MessageIndex(3)));
        let copied = list;
        assert_eq!(list, copied);

        let map = FieldKind::Map {
            key: ScalarType::String,
            value: SingularKind::Enum(EnumIndex(1)),
        };
        match map {
            FieldKind::Map { key, value } => {
                assert_eq!(key, ScalarType::String);
                assert_eq!(value, SingularKind::Enum(EnumIndex(1)));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn scalar_type_from_proto_exhaustive() {
        use ProtoType::*;
        let all = [
            (TYPE_DOUBLE, ScalarType::Double),
            (TYPE_FLOAT, ScalarType::Float),
            (TYPE_INT64, ScalarType::Int64),
            (TYPE_UINT64, ScalarType::Uint64),
            (TYPE_INT32, ScalarType::Int32),
            (TYPE_FIXED64, ScalarType::Fixed64),
            (TYPE_FIXED32, ScalarType::Fixed32),
            (TYPE_BOOL, ScalarType::Bool),
            (TYPE_STRING, ScalarType::String),
            (TYPE_BYTES, ScalarType::Bytes),
            (TYPE_UINT32, ScalarType::Uint32),
            (TYPE_SFIXED32, ScalarType::Sfixed32),
            (TYPE_SFIXED64, ScalarType::Sfixed64),
            (TYPE_SINT32, ScalarType::Sint32),
            (TYPE_SINT64, ScalarType::Sint64),
        ];
        for (proto, scalar) in all {
            assert_eq!(ScalarType::from_proto(proto), Some(scalar));
        }
    }
}

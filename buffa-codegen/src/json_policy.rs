//! Code fragments for `json_policy_types`: custom `bytes` and `string`
//! representations whose JSON goes through `buffa::json_helpers::ProtoBytesJson`
//! or `ProtoStringJson`.

use proc_macro2::TokenStream;
use quote::quote;

use crate::context::CodeGenContext;
use crate::generated::descriptor::field_descriptor_proto::Type;
use crate::CodeGenError;

/// Which policy trait a field's custom representation implements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JsonPolicyKind {
    /// A custom `bytes` type implementing `ProtoBytesJson`.
    Bytes,
    /// A custom `string` type implementing `ProtoStringJson`.
    String,
}

impl JsonPolicyKind {
    /// The policy kind of a `bytes` or `string` field whose resolved
    /// representations are `bytes_repr` and `string_repr`, or `None` when the
    /// field's type is not listed in `json_policy_types`.
    pub(crate) fn of_field(
        ctx: &CodeGenContext<'_>,
        field_type: Type,
        bytes_repr: &crate::BytesRepr,
        string_repr: &crate::StringRepr,
    ) -> Option<Self> {
        match (field_type, bytes_repr, string_repr) {
            (Type::TYPE_BYTES, crate::BytesRepr::Custom(path), _)
                if ctx.is_json_policy_type(path) =>
            {
                Some(Self::Bytes)
            }
            (Type::TYPE_STRING, _, crate::StringRepr::Custom(path))
                if ctx.is_json_policy_type(path) =>
            {
                Some(Self::String)
            }
            _ => None,
        }
    }

    /// The borrowed type the policy serializes: `[u8]` or `str`.
    pub(crate) fn borrowed(self) -> TokenStream {
        match self {
            Self::Bytes => quote! { [u8] },
            Self::String => quote! { str },
        }
    }

    /// An expression serializing the borrowed `value` (of type
    /// `&[u8]` / `&str`) under the policy of `rust_type`.
    pub(crate) fn serialize_borrowed(
        self,
        rust_type: &TokenStream,
        value: &TokenStream,
    ) -> TokenStream {
        match self {
            Self::Bytes => {
                quote! { &::buffa::json_helpers::BytesPolicyJson::<#rust_type>::new(#value) }
            }
            Self::String => {
                quote! { &::buffa::json_helpers::StringPolicyJson::<#rust_type>::new(#value) }
            }
        }
    }

    /// An expression serializing the owned `value` (a reference to
    /// `rust_type`) under its policy.
    pub(crate) fn serialize_owned(
        self,
        rust_type: &TokenStream,
        value: &TokenStream,
    ) -> TokenStream {
        let borrowed = self.borrowed();
        self.serialize_borrowed(
            rust_type,
            &quote! { ::core::convert::AsRef::<#borrowed>::as_ref(#value) },
        )
    }

    /// A `DeserializeSeed` producing `rust_type` through its policy.
    pub(crate) fn seed(self, rust_type: &TokenStream) -> TokenStream {
        match self {
            Self::Bytes => quote! { ::buffa::json_helpers::BytesPolicySeed::<#rust_type>::new() },
            Self::String => quote! { ::buffa::json_helpers::StringPolicySeed::<#rust_type>::new() },
        }
    }
}

/// The policy of a non-map field, with its owned type, for code that only
/// has the borrowed value: views and lazy views. `field_type` is the field's
/// effective type.
pub(crate) fn of_view_field(
    ctx: &CodeGenContext<'_>,
    proto_fqn: &str,
    field_name: &str,
    field_type: Type,
) -> Result<Option<(JsonPolicyKind, TokenStream)>, CodeGenError> {
    let path = match field_type {
        Type::TYPE_BYTES => match crate::impl_message::field_bytes_repr(ctx, proto_fqn, field_name)
        {
            crate::BytesRepr::Custom(path) => (JsonPolicyKind::Bytes, path),
            _ => return Ok(None),
        },
        Type::TYPE_STRING => {
            match crate::impl_message::field_string_repr(ctx, proto_fqn, field_name) {
                crate::StringRepr::Custom(path) => (JsonPolicyKind::String, path),
                _ => return Ok(None),
            }
        }
        _ => return Ok(None),
    };
    let (kind, path) = path;
    if !ctx.is_json_policy_type(&path) {
        return Ok(None);
    }
    Ok(Some((kind, crate::parse_custom_type_path(&path)?)))
}

/// The owned type of a map slot whose representation `repr_path` is a
/// listed policy type, or `None`.
pub(crate) fn of_map_slot(
    ctx: &CodeGenContext<'_>,
    kind: JsonPolicyKind,
    repr_path: Option<&str>,
) -> Result<Option<(JsonPolicyKind, TokenStream)>, CodeGenError> {
    match repr_path {
        Some(path) if ctx.is_json_policy_type(path) => {
            Ok(Some((kind, crate::parse_custom_type_path(path)?)))
        }
        _ => Ok(None),
    }
}

impl JsonPolicyKind {
    /// An expression serializing borrowed repeated elements
    /// (`&[&[u8]]` / `&[&str]`) under the policy of `rust_type`.
    pub(crate) fn serialize_seq(self, rust_type: &TokenStream, value: &TokenStream) -> TokenStream {
        match self {
            Self::Bytes => {
                quote! { &::buffa::json_helpers::BytesPolicySeqJson::<#rust_type>::new(#value) }
            }
            Self::String => {
                quote! { &::buffa::json_helpers::StringPolicySeqJson::<#rust_type>::new(#value) }
            }
        }
    }
}

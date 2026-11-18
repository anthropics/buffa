//! Validate the complete request before returning any native declarations.

use std::collections::{BTreeMap, BTreeSet};

use proc_macro2::{Ident, TokenStream};
use quote::quote;

use crate::generated::descriptor::field_descriptor_proto::{Label, Type};
use crate::generated::descriptor::{
    DescriptorProto, EnumDescriptorProto, FieldDescriptorProto, FileDescriptorProto,
};
use crate::{GeneratedFile, GeneratedFileKind};

use super::*;

const MAX_FIELD_DEPTH: usize = 64;

struct Schema<'a> {
    enums: BTreeMap<String, (&'a EnumDescriptorProto, &'a str)>,
    messages: BTreeMap<String, (&'a DescriptorProto, &'a str)>,
    native_selectors: BTreeMap<String, BTreeSet<String>>,
}

impl<'a> Schema<'a> {
    fn new(files: &'a [FileDescriptorProto]) -> Result<Self, AdapterError> {
        let mut schema = Self {
            enums: BTreeMap::new(),
            messages: BTreeMap::new(),
            native_selectors: BTreeMap::new(),
        };
        for file in files {
            let file_name = file.name.as_deref().ok_or(ErrorReason::InvalidDescriptor)?;
            let package = file.package.as_deref().unwrap_or_default();
            let prefix = if package.is_empty() {
                String::new()
            } else {
                format!(".{package}")
            };
            schema.add_enums(&prefix, &file.enum_type, file_name)?;
            // An explicit stack also bounds traversal of adversarial hand-built
            // descriptors without relying on the thread's recursion limit.
            let mut pending: Vec<_> = file
                .message_type
                .iter()
                .map(|m| (prefix.clone(), m))
                .collect();
            while let Some((parent, message)) = pending.pop() {
                let name = message
                    .name
                    .as_deref()
                    .ok_or(ErrorReason::InvalidDescriptor)?;
                let fqn = format!("{parent}.{name}");
                if schema
                    .messages
                    .insert(fqn.clone(), (message, file_name))
                    .is_some()
                {
                    return Err(ErrorReason::InvalidDescriptor.into());
                }
                schema.add_enums(&fqn, &message.enum_type, file_name)?;
                pending.extend(message.nested_type.iter().map(|m| (fqn.clone(), m)));
            }
        }
        if schema
            .enums
            .keys()
            .any(|name| schema.messages.contains_key(name))
        {
            return Err(ErrorReason::InvalidDescriptor.into());
        }
        Ok(schema)
    }

    fn add_enums(
        &mut self,
        parent: &str,
        enums: &'a [EnumDescriptorProto],
        file: &'a str,
    ) -> Result<(), AdapterError> {
        for descriptor in enums {
            let name = descriptor
                .name
                .as_deref()
                .ok_or(ErrorReason::InvalidDescriptor)?;
            if self
                .enums
                .insert(format!("{parent}.{name}"), (descriptor, file))
                .is_some()
            {
                return Err(ErrorReason::InvalidDescriptor.into());
            }
        }
        Ok(())
    }

    fn enumeration(&self, name: &str) -> Result<&'a EnumDescriptorProto, AdapterError> {
        self.enums
            .get(name)
            .map(|(e, _)| *e)
            .ok_or_else(|| ErrorReason::MissingDescriptor.into())
    }

    fn message(&self, name: &str) -> Result<&'a DescriptorProto, AdapterError> {
        self.messages
            .get(name)
            .map(|(m, _)| *m)
            .ok_or_else(|| ErrorReason::MissingDescriptor.into())
    }

    fn in_scope(&self, name: &str, requested: &[String]) -> Result<(), AdapterError> {
        let file = self
            .enums
            .get(name)
            .map(|(_, f)| f)
            .or_else(|| self.messages.get(name).map(|(_, f)| f))
            .ok_or(ErrorReason::MissingDescriptor)?;
        if !requested.iter().any(|f| f == file) {
            return Err(ErrorReason::GenerationScope.into());
        }
        Ok(())
    }

    fn field(&self, source: &FieldSource) -> Result<&'a FieldDescriptorProto, AdapterError> {
        if source.path.is_empty() || source.path.len() > MAX_FIELD_DEPTH {
            return Err(ErrorReason::InvalidPayload.into());
        }
        let mut message = self.message(&source.message)?;
        let mut parts = source.path.iter().peekable();
        while let Some(part) = parts.next() {
            let field = unique_field(message, part)?;
            if field.label == Some(Label::LABEL_REPEATED) {
                return Err(ErrorReason::OwnershipMismatch.into());
            }
            if parts.peek().is_none() {
                return Ok(field);
            }
            if field.r#type != Some(Type::TYPE_MESSAGE) || field.proto3_optional == Some(true) {
                return Err(ErrorReason::InvalidPayload.into());
            }
            message = self.message(
                field
                    .type_name
                    .as_deref()
                    .ok_or(ErrorReason::InvalidDescriptor)?,
            )?;
        }
        Err(ErrorReason::InvalidPayload.into())
    }
}

fn unique_field<'a>(
    message: &'a DescriptorProto,
    name: &str,
) -> Result<&'a FieldDescriptorProto, AdapterError> {
    let mut fields = message
        .field
        .iter()
        .filter(|f| f.name.as_deref() == Some(name));
    let field = fields.next().ok_or(ErrorReason::MissingDescriptor)?;
    if fields.next().is_some() {
        return Err(ErrorReason::InvalidDescriptor.into());
    }
    Ok(field)
}

fn ident(name: &str) -> Result<Ident, AdapterError> {
    // Reject structural syntax before tokenization: a deeply nested token
    // group must never reach a general Rust parser through a name string.
    if !name
        .bytes()
        .all(|b| !b.is_ascii() || b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(ErrorReason::InvalidIdentifier.into());
    }
    // syn's identifier parser does not track every newer edition keyword.
    // Native declarations must remain usable by consumers on those editions.
    if name.starts_with("r#") || crate::idents::is_rust_keyword(name) {
        return Err(ErrorReason::InvalidIdentifier.into());
    }
    let value: Ident = syn::parse_str(name).map_err(|_| ErrorReason::InvalidIdentifier)?;
    if matches!(
        name,
        "self" | "Self" | "super" | "crate" | "_" | "r#self" | "r#Self" | "r#super" | "r#crate"
    ) {
        return Err(ErrorReason::InvalidIdentifier.into());
    }
    Ok(value)
}

fn path(name: &str) -> Result<syn::Path, AdapterError> {
    // Only module-path syntax reaches syn. In particular, nested generics or
    // token groups cannot exhaust the parser's stack before being rejected.
    if !name.bytes().all(|b| {
        !b.is_ascii()
            || b.is_ascii_alphanumeric()
            || b.is_ascii_whitespace()
            || matches!(b, b'_' | b':' | b'#')
    }) {
        return Err(ErrorReason::InvalidRustPath.into());
    }
    let value: syn::Path = syn::parse_str(name).map_err(|_| ErrorReason::InvalidRustPath)?;
    if value.segments.is_empty()
        || value.segments.iter().any(|s| {
            let name = s.ident.to_string();
            !matches!(s.arguments, syn::PathArguments::None)
                || (crate::idents::is_rust_keyword(&name)
                    && !matches!(name.as_str(), "self" | "Self" | "super" | "crate"))
        })
    {
        return Err(ErrorReason::InvalidRustPath.into());
    }
    Ok(value)
}

fn error(mapping: &ErrorMapping) -> Result<(syn::Path, syn::Path), AdapterError> {
    Ok((path(&mapping.rust_type)?, path(&mapping.value)?))
}

/// Generate standalone native adapter files, validating complete coverage first.
///
/// Outputs have [`GeneratedFileKind::Adapter`] and are intentionally excluded
/// from the portable module tree. Include each in its native API module. This
/// function neither reads files nor changes descriptors or portable generation.
///
/// # Errors
/// Returns a fixed [`ErrorReason`] for invalid metadata, incomplete coverage,
/// incompatible ownership, ambiguous aliases or sources outside the request.
pub fn generate_adapters(
    files: &[FileDescriptorProto],
    requested: &[String],
    config: &AdapterConfig,
) -> Result<Vec<GeneratedFile>, AdapterError> {
    if config.version != 1 {
        return Err(ErrorReason::UnsupportedVersion.into());
    }
    let mut schema = Schema::new(files)?;
    for selector in &config.selectors {
        schema
            .native_selectors
            .entry(selector.wire.source.clone())
            .or_default()
            .insert(selector.name.clone());
    }
    let mut names = BTreeSet::new();
    let mut ports = BTreeMap::new();
    for port in &config.ports {
        ident(&port.name)?;
        if ports
            .insert(port.name.as_str(), path(&port.rust_trait)?)
            .is_some()
        {
            return Err(ErrorReason::DuplicateName.into());
        }
    }
    let mut output = BTreeMap::new();
    let mut emit = |file: &str, name: &str, tokens: TokenStream| -> Result<(), AdapterError> {
        ident(name)?;
        if !names.insert(name.to_owned()) {
            return Err(ErrorReason::DuplicateName.into());
        }
        let stem = file.strip_suffix(".rs").ok_or(ErrorReason::InvalidOutput)?;
        if stem.is_empty()
            || stem.eq_ignore_ascii_case("mod")
            || !stem.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(ErrorReason::InvalidOutput.into());
        }
        let syntax = syn::parse2::<syn::File>(tokens).map_err(|_| ErrorReason::InvalidSyntax)?;
        let content = format!(
            "{}{}",
            "// @generated by buffa-codegen. DO NOT EDIT.\n",
            prettyplease::unparse(&syntax)
        );
        let generated = GeneratedFile {
            name: file.to_owned(),
            package: String::new(),
            content,
            kind: GeneratedFileKind::Adapter,
        };
        // Files may be written on a case-insensitive filesystem even when
        // generation runs on a case-sensitive build host.
        if output
            .insert(file.to_ascii_lowercase(), generated)
            .is_some()
        {
            return Err(ErrorReason::DuplicateOutput.into());
        }
        Ok(())
    };
    for selector in &config.selectors {
        schema.in_scope(&selector.wire.source, requested)?;
        emit(&selector.file, &selector.name, closed(&schema, selector)?)?;
    }
    for selector in &config.payloads {
        schema.in_scope(&selector.source, requested)?;
        let cases = enum_cases(schema.enumeration(&selector.source)?)?;
        let tokens = payload_enum(
            &schema,
            &ports,
            &selector.name,
            &selector.variants,
            &cases,
            false,
            None,
        )?;
        emit(&selector.file, &selector.name, tokens)?;
    }
    for adapter in &config.borrowed {
        let (cases, oneof) = match &adapter.source {
            BorrowedSource::Enum { source } => {
                schema.in_scope(source, requested)?;
                (enum_cases(schema.enumeration(source)?)?, None)
            }
            BorrowedSource::Oneof { message, name } => {
                schema.in_scope(message, requested)?;
                let descriptor = schema.message(message)?;
                let mut declarations = descriptor
                    .oneof_decl
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| o.name.as_deref() == Some(name));
                let (index, _) = declarations.next().ok_or(ErrorReason::MissingDescriptor)?;
                if declarations.next().is_some() {
                    return Err(ErrorReason::InvalidDescriptor.into());
                }
                let index = i32::try_from(index).map_err(|_| ErrorReason::InvalidDescriptor)?;
                let mut cases = BTreeMap::new();
                for field in &descriptor.field {
                    if field.oneof_index != Some(index) || field.proto3_optional == Some(true) {
                        return Err(ErrorReason::UncoveredField.into());
                    }
                    let name = field
                        .name
                        .as_deref()
                        .ok_or(ErrorReason::InvalidDescriptor)?;
                    if cases
                        .insert(
                            name.to_owned(),
                            field.number.ok_or(ErrorReason::InvalidDescriptor)?,
                        )
                        .is_some()
                    {
                        return Err(ErrorReason::InvalidDescriptor.into());
                    }
                }
                if cases.is_empty() {
                    return Err(ErrorReason::InvalidDescriptor.into());
                }
                (cases, Some(message.as_str()))
            }
        };
        let tokens = payload_enum(
            &schema,
            &ports,
            &adapter.name,
            &adapter.variants,
            &cases,
            true,
            oneof,
        )?;
        emit(&adapter.file, &adapter.name, tokens)?;
    }
    Ok(output.into_values().collect())
}

fn enum_values(descriptor: &EnumDescriptorProto) -> Result<BTreeMap<String, i32>, AdapterError> {
    let mut values = BTreeMap::new();
    for value in &descriptor.value {
        let name = value
            .name
            .as_deref()
            .ok_or(ErrorReason::InvalidDescriptor)?;
        let number = value.number.ok_or(ErrorReason::InvalidDescriptor)?;
        let mut bytes = name.bytes();
        if !bytes
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
            || !bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(ErrorReason::InvalidDescriptor.into());
        }
        if values.insert(name.to_owned(), number).is_some() {
            return Err(ErrorReason::InvalidDescriptor.into());
        }
    }
    // Unspecified is a numeric boundary, not a unique spelling: protobuf
    // enums may legally declare several names for zero with allow_alias.
    if !values.values().any(|&number| number == 0) {
        return Err(ErrorReason::InvalidDescriptor.into());
    }
    Ok(values)
}

fn enum_cases(descriptor: &EnumDescriptorProto) -> Result<BTreeMap<String, i32>, AdapterError> {
    let mut cases = enum_values(descriptor)?;
    cases.retain(|_, n| *n != 0);
    if cases.is_empty() {
        return Err(ErrorReason::InvalidDescriptor.into());
    }
    Ok(cases)
}

fn claim<'a>(
    values: &'a BTreeMap<String, i32>,
    claimed: &mut BTreeSet<String>,
    source: &str,
) -> Result<&'a i32, AdapterError> {
    let number = values.get(source).ok_or(ErrorReason::UnknownCase)?;
    if *number == 0 {
        return Err(ErrorReason::UnspecifiedCase.into());
    }
    if !claimed.insert(source.to_owned()) {
        return Err(ErrorReason::DuplicateCase.into());
    }
    Ok(number)
}

struct InputMapping<'a> {
    wire: &'a WireEnum,
    error: &'a ErrorMapping,
    canonical: &'a [(String, String)],
    normalized: &'a [Normalization],
    rejected: &'a [String],
}

fn conversion(
    schema: &Schema<'_>,
    variants: &BTreeMap<String, Ident>,
    native: &Ident,
    mapping: InputMapping<'_>,
) -> Result<TokenStream, AdapterError> {
    let InputMapping {
        wire,
        error: error_mapping,
        canonical,
        normalized,
        rejected,
    } = mapping;
    let values = enum_values(schema.enumeration(&wire.source)?)?;
    let wire_type = path(&wire.rust_type)?;
    let (error_type, error_value) = error(error_mapping)?;
    let mut claimed = BTreeSet::new();
    let mut accepted = BTreeMap::new();
    for (source, target) in canonical
        .iter()
        .map(|(s, t)| (s, t))
        .chain(normalized.iter().map(|n| (&n.source, &n.target)))
    {
        let number = *claim(&values, &mut claimed, source)?;
        let variant = variants
            .get(target)
            .ok_or(ErrorReason::InvalidNormalization)?;
        if let Some(previous) = accepted.insert(number, variant) {
            if previous != variant {
                return Err(ErrorReason::InvalidNormalization.into());
            }
        }
    }
    for source in rejected {
        let number = claim(&values, &mut claimed, source)?;
        if accepted.contains_key(number) {
            return Err(ErrorReason::InvalidNormalization.into());
        }
    }
    if values
        .iter()
        .any(|(name, &n)| n != 0 && !claimed.contains(name))
    {
        return Err(ErrorReason::UncoveredCase.into());
    }
    let arms = accepted
        .iter()
        .map(|(number, variant)| quote! { #number => ::core::result::Result::Ok(Self::#variant), });
    // The public Unknown variant can be constructed with a recognized number.
    // Preserve the known-value boundary before applying numeric normalizations.
    Ok(quote! {
        impl ::core::convert::TryFrom<::buffa::EnumValue<#wire_type>> for #native {
            type Error = #error_type;
            fn try_from(value: ::buffa::EnumValue<#wire_type>) -> ::core::result::Result<Self, #error_type> {
                if value.is_unknown() {
                    return ::core::result::Result::Err(::core::convert::Into::into(#error_value));
                }
                match value.to_i32() {
                    #(#arms)*
                    _ => ::core::result::Result::Err(::core::convert::Into::into(#error_value)),
                }
            }
        }
    })
}

fn closed(schema: &Schema<'_>, selector: &ClosedSelector) -> Result<TokenStream, AdapterError> {
    let name = ident(&selector.name)?;
    if selector.variants.is_empty() {
        return Err(ErrorReason::UncoveredCase.into());
    }
    let mut variants = BTreeMap::new();
    let mut occupied = BTreeSet::from([
        String::from("ALL"),
        String::from("identifier"),
        String::from("from_identifier"),
    ]);
    let mut declarations = Vec::new();
    let mut aliases = Vec::new();
    let mut canonical = Vec::new();
    let mut reverse = Vec::new();
    let mut ordered = Vec::new();
    let wire = path(&selector.wire.rust_type)?;
    let mut canonical_numbers = BTreeSet::new();
    let values = enum_values(schema.enumeration(&selector.wire.source)?)?;
    for variant in &selector.variants {
        let native = ident(&variant.name)?;
        if !occupied.insert(variant.name.clone()) {
            return Err(ErrorReason::DuplicateName.into());
        }
        let number = values
            .get(&variant.source)
            .ok_or(ErrorReason::UnknownCase)?;
        if !canonical_numbers.insert(number) {
            return Err(ErrorReason::InvalidNormalization.into());
        }
        // Wire values follow the portable generator's keyword escaping; the
        // explicitly named native API uses the stricter identifier validator.
        let source = crate::idents::make_field_ident(&variant.source);
        variants.insert(variant.name.clone(), native.clone());
        canonical.push((variant.source.clone(), variant.name.clone()));
        ordered.push(native.clone());
        declarations.push(quote! { #[doc = "Canonical descriptor selector."] #native, });
        reverse.push(quote! { #name::#native => Self::#source, });
        for alias in &variant.aliases {
            let alias_ident = ident(alias)?;
            if !occupied.insert(alias.clone()) {
                return Err(ErrorReason::DuplicateName.into());
            }
            aliases.push(quote! {
                #[doc = "Compatibility spelling of the canonical selector."]
                #[allow(non_upper_case_globals)]
                pub const #alias_ident: Self = Self::#native;
            });
        }
    }
    let conversion = conversion(
        schema,
        &variants,
        &name,
        InputMapping {
            wire: &selector.wire,
            error: &selector.error,
            canonical: &canonical,
            normalized: &selector.normalizations,
            rejected: &selector.rejected,
        },
    )?;
    let mut legacy = Vec::new();
    let mut legacy_types = BTreeSet::from([quote!(#wire).to_string()]);
    for source in &selector.legacy {
        // Distinct paths prevent duplicate impls within one declaration. Rust
        // additionally checks whether application aliases name the same type.
        let legacy_path = path(&source.wire.rust_type)?;
        if !legacy_types.insert(quote!(#legacy_path).to_string()) {
            return Err(ErrorReason::InvalidNormalization.into());
        }
        legacy.push(conversion_for_legacy(
            schema,
            source,
            &selector.error,
            &variants,
            &name,
        )?);
    }
    let identities = identities(selector, &ordered)?;
    let non_exhaustive = selector
        .non_exhaustive
        .then(|| quote! { #[non_exhaustive] });
    Ok(quote! {
        #[doc = "Closed native selector generated from authoritative descriptors."]
        #non_exhaustive
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub enum #name { #(#declarations)* }
        #conversion
        #(#legacy)*
        #[allow(deprecated)]
        impl ::core::convert::From<#name> for #wire {
            fn from(value: #name) -> Self { match value { #(#reverse)* } }
        }
        impl #name {
            #[doc = "Complete canonical selector set in manifest order."]
            pub const ALL: &'static [Self] = &[#(Self::#ordered),*];
            #(#aliases)*
        }
        #identities
    })
}

fn conversion_for_legacy(
    schema: &Schema<'_>,
    legacy: &LegacySelector,
    error: &ErrorMapping,
    variants: &BTreeMap<String, Ident>,
    name: &Ident,
) -> Result<TokenStream, AdapterError> {
    conversion(
        schema,
        variants,
        name,
        InputMapping {
            wire: &legacy.wire,
            error,
            canonical: &[],
            normalized: &legacy.normalizations,
            rejected: &legacy.rejected,
        },
    )
}

fn identities(selector: &ClosedSelector, variants: &[Ident]) -> Result<TokenStream, AdapterError> {
    let count = selector
        .variants
        .iter()
        .filter(|v| v.identifier.is_some())
        .count();
    let Some(mapping) = &selector.identifier_error else {
        if count != 0 {
            return Err(ErrorReason::InvalidIdentifiers.into());
        }
        return Ok(TokenStream::new());
    };
    if count != variants.len() {
        return Err(ErrorReason::InvalidIdentifiers.into());
    }
    let (error_type, error_value) = error(mapping)?;
    let name = ident(&selector.name)?;
    let numbers: Vec<_> = selector
        .variants
        .iter()
        .filter_map(|v| v.identifier)
        .collect();
    if numbers.iter().collect::<BTreeSet<_>>().len() != numbers.len() {
        return Err(ErrorReason::InvalidIdentifiers.into());
    }
    Ok(quote! {
        impl #name {
            #[doc = "Exact external protocol identity, distinct from the protobuf number."]
            #[must_use]
            pub const fn identifier(self) -> ::core::primitive::i64 { match self { #(Self::#variants => #numbers,)* } }
            #[doc = "Resolve an exact external identity without policy substitution."]
            #[doc = ""]
            #[doc = "# Errors"]
            #[doc = "Returns the configured fixed domain error for an unsupported identity."]
            pub fn from_identifier(value: ::core::primitive::i64) -> ::core::result::Result<Self, #error_type> {
                match value {
                    #(#numbers => ::core::result::Result::Ok(Self::#variants),)*
                    _ => ::core::result::Result::Err(::core::convert::Into::into(#error_value)),
                }
            }
        }
    })
}

fn payload_type(
    schema: &Schema<'_>,
    field: &PayloadField,
    borrowed: bool,
) -> Result<TokenStream, AdapterError> {
    ident(&field.name)?;
    let descriptor = schema.field(&field.source)?;
    match &field.representation {
        PayloadType::Bytes => {
            if !borrowed {
                return Err(ErrorReason::OwnershipMismatch.into());
            }
            if descriptor.r#type != Some(Type::TYPE_BYTES) {
                return Err(ErrorReason::InvalidPayload.into());
            }
            Ok(quote! { &'a [::core::primitive::u8] })
        }
        PayloadType::Value { source, rust_type } => {
            if !matches!(
                descriptor.r#type,
                Some(Type::TYPE_ENUM | Type::TYPE_MESSAGE)
            ) || descriptor.type_name.as_deref() != Some(source)
            {
                return Err(ErrorReason::InvalidPayload.into());
            }
            match descriptor.r#type {
                Some(Type::TYPE_ENUM) => {
                    schema.enumeration(source)?;
                }
                Some(Type::TYPE_MESSAGE) => {
                    // A Copy compatibility value can aggregate closed selectors,
                    // but cannot smuggle byte owners or collections into a borrow.
                    let message = schema.message(source)?;
                    if message.field.is_empty()
                        || message.field.iter().any(|field| {
                            field.r#type != Some(Type::TYPE_ENUM)
                                || field.label == Some(Label::LABEL_REPEATED)
                                || !field
                                    .type_name
                                    .as_ref()
                                    .is_some_and(|name| schema.native_selectors.contains_key(name))
                        })
                    {
                        return Err(ErrorReason::OwnershipMismatch.into());
                    }
                }
                _ => return Err(ErrorReason::InvalidPayload.into()),
            }
            let value = path(rust_type)?;
            if descriptor.r#type == Some(Type::TYPE_ENUM) {
                let native = value
                    .segments
                    .last()
                    .ok_or(ErrorReason::InvalidRustPath)?
                    .ident
                    .to_string();
                if !schema
                    .native_selectors
                    .get(source)
                    .is_some_and(|names| names.contains(&native))
                {
                    return Err(ErrorReason::InvalidPayload.into());
                }
            }
            Ok(quote! { #value })
        }
    }
}

fn payload_fields(payload: &Payload) -> &[PayloadField] {
    match payload {
        Payload::Tuple { field } => std::slice::from_ref(field),
        Payload::Struct { fields } => fields,
        Payload::Unit | Payload::Port { .. } => &[],
    }
}

fn payload_enum(
    schema: &Schema<'_>,
    ports: &BTreeMap<&str, syn::Path>,
    name: &str,
    variants: &[PayloadVariant],
    cases: &BTreeMap<String, i32>,
    borrowed: bool,
    oneof: Option<&str>,
) -> Result<TokenStream, AdapterError> {
    let name = ident(name)?;
    let mut claimed = BTreeSet::new();
    let mut names = BTreeSet::new();
    let mut declarations = Vec::new();
    let mut has_borrow = false;
    let mut numeric_targets = BTreeMap::new();
    let mut copy_checks = Vec::new();
    for variant in variants {
        let native = ident(&variant.name)?;
        if !names.insert(variant.name.clone()) {
            return Err(ErrorReason::DuplicateName.into());
        }
        if variant.cases.is_empty() {
            return Err(ErrorReason::UncoveredCase.into());
        }
        for case in &variant.cases {
            let number = claim(cases, &mut claimed, case)?;
            if let Some(previous) = numeric_targets.insert(number, &variant.name) {
                if previous != &variant.name {
                    return Err(ErrorReason::InvalidNormalization.into());
                }
            }
        }
        let fields = payload_fields(&variant.payload);
        let mut field_names = BTreeSet::new();
        let mut sources = BTreeSet::new();
        for field in fields {
            if !field_names.insert(&field.name) {
                return Err(ErrorReason::DuplicateName.into());
            }
            let key = (&field.source.message, &field.source.path);
            if !sources.insert(key) {
                return Err(ErrorReason::DuplicateCase.into());
            }
            if fields.iter().any(|other| {
                other.source.message == field.source.message
                    && other.source.path != field.source.path
                    && other.source.path.starts_with(&field.source.path)
            }) {
                return Err(ErrorReason::DuplicateCase.into());
            }
            if matches!(field.representation, PayloadType::Value { .. }) {
                let ty = payload_type(schema, field, borrowed)?;
                copy_checks.push(quote! { __assert_copy::<#ty>(); });
            }
            if let Some(root) = oneof {
                if field.source.message != root
                    || !field
                        .source
                        .path
                        .first()
                        .is_some_and(|p| variant.cases.contains(p))
                {
                    return Err(ErrorReason::UnexpectedField.into());
                }
            }
        }
        if let Some(root) = oneof {
            if variant.cases.len() != 1 {
                return Err(ErrorReason::InvalidPayload.into());
            }
            for case in &variant.cases {
                complete_payload(schema, root, std::slice::from_ref(case), fields)?;
            }
        }
        let payload = match &variant.payload {
            Payload::Unit => TokenStream::new(),
            Payload::Tuple { field } => {
                has_borrow |= matches!(field.representation, PayloadType::Bytes);
                let ty = payload_type(schema, field, borrowed)?;
                quote! { (#ty) }
            }
            Payload::Struct { fields } => {
                if fields.is_empty() {
                    return Err(ErrorReason::InvalidPayload.into());
                }
                let mut members = Vec::new();
                for field in fields {
                    has_borrow |= matches!(field.representation, PayloadType::Bytes);
                    let field_name = ident(&field.name)?;
                    let ty = payload_type(schema, field, borrowed)?;
                    members.push(quote! { #[doc = "Projected canonical descriptor field."] #field_name: #ty, });
                }
                quote! { { #(#members)* } }
            }
            Payload::Port { name } => {
                if !borrowed || oneof.is_some() {
                    return Err(ErrorReason::OwnershipMismatch.into());
                }
                let port = ports
                    .get(name.as_str())
                    .ok_or(ErrorReason::InvalidPayload)?;
                has_borrow = true;
                quote! { (&'a dyn #port) }
            }
        };
        declarations
            .push(quote! { #[doc = "Descriptor-defined native alternative."] #native #payload, });
    }
    if claimed.len() != cases.len() {
        return Err(ErrorReason::UncoveredCase.into());
    }
    if borrowed && !has_borrow {
        return Err(ErrorReason::OwnershipMismatch.into());
    }
    let lifetime = borrowed.then(|| quote! { <'a> });
    // Borrowed bytes and capabilities must not inherit diagnostics or an owning
    // cleanup implementation. Copy payload selectors cannot hide an owner.
    let derives =
        (!borrowed).then(|| quote! { #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)] });
    let copy_guard = (!copy_checks.is_empty()).then(|| {
        quote! {
            const _: fn() = || {
                fn __assert_copy<T: ::core::marker::Copy>() {}
                #(#copy_checks)*
            };
        }
    });
    Ok(quote! {
        #[doc = "Process-local adapter generated from authoritative descriptors."]
        #derives
        pub enum #name #lifetime { #(#declarations)* }
        #copy_guard
    })
}

fn complete_payload(
    schema: &Schema<'_>,
    root: &str,
    prefix: &[String],
    fields: &[PayloadField],
) -> Result<(), AdapterError> {
    let mut pending = vec![prefix.to_vec()];
    while let Some(current) = pending.pop() {
        if current.len() > MAX_FIELD_DEPTH {
            return Err(ErrorReason::InvalidPayload.into());
        }
        if fields
            .iter()
            .any(|f| f.source.message == root && f.source.path == current)
        {
            continue;
        }
        let field = schema.field(&FieldSource {
            message: root.to_owned(),
            path: current.clone(),
        })?;
        let descendants = fields
            .iter()
            .any(|f| f.source.message == root && f.source.path.starts_with(&current));
        if field.r#type != Some(Type::TYPE_MESSAGE) || !descendants {
            return Err(ErrorReason::UncoveredField.into());
        }
        let child = schema.message(
            field
                .type_name
                .as_deref()
                .ok_or(ErrorReason::InvalidDescriptor)?,
        )?;
        if child.field.is_empty() {
            return Err(ErrorReason::UncoveredField.into());
        }
        for member in &child.field {
            let mut next = current.clone();
            next.push(member.name.clone().ok_or(ErrorReason::InvalidDescriptor)?);
            pending.push(next);
        }
    }
    // A parent and its descendant cannot both be claimed as distinct payloads.
    for field in fields {
        if fields.iter().any(|other| {
            other.source.path != field.source.path
                && other.source.path.starts_with(&field.source.path)
        }) {
            return Err(ErrorReason::DuplicateCase.into());
        }
    }
    Ok(())
}

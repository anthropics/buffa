//! Rust names that a schema sets with `(buffa.ext.field).name`.
//!
//! [`field_name`] reads the option, and
//! [`CodeGenContext::field_rust_name`] and
//! [`oneof_variant_ident`](crate::oneof::oneof_variant_ident) use the value
//! as written, in place of the name they derive from the proto name.
//!
//! [`validate_file`] runs before any code is generated for a file. It rejects
//! a value that is unusable as the identifier it asks for, and a value that
//! gives two members of one struct, or two variants of one oneof, the same
//! Rust name. So the emission code can build an identifier from the value
//! without checking it.

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fmt;

use buffa::ExtensionSet as _;

use crate::context::CodeGenContext;
use crate::generated::descriptor::{DescriptorProto, FieldDescriptorProto, FileDescriptorProto};
use crate::impl_message::is_real_oneof_member;
use crate::CodeGenError;

/// The option that [`field_name`] reads, as a schema writes it.
const FIELD_NAME_OPTION: &str = "(buffa.ext.field).name";

/// Why code generation rejected the value of a `name` setting, such as
/// `(buffa.ext.field).name`.
///
/// [`CodeGenError::InvalidNameOption`] has one as its `problem`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum NameOptionProblem {
    /// The value is not an ASCII identifier: letters, digits and `_`, not
    /// starting with a digit, and not `_` alone.
    NotAnIdentifier,
    /// The value is a Rust keyword. buffa escapes a proto name that is a
    /// keyword, and does not escape a `name` value.
    Keyword,
    /// The value starts with `__buffa_`, the prefix of the identifiers that
    /// buffa adds to generated code, such as the fields it adds to a struct.
    ReservedPrefix,
    /// The option is on an extension. buffa generates a constant for an
    /// extension, and the option does not rename the constant.
    OnExtension,
}

impl fmt::Display for NameOptionProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAnIdentifier => f.write_str(
                "the value is not an ASCII Rust identifier (letters, digits and `_`, \
                 not starting with a digit, and not `_` alone)",
            ),
            Self::Keyword => {
                f.write_str("the value is a Rust keyword, and buffa does not escape a `name` value")
            }
            Self::ReservedPrefix => f.write_str(
                "names that start with `__buffa_` are reserved for buffa's own identifiers",
            ),
            Self::OnExtension => f.write_str("the option does not rename an extension"),
        }
    }
}

/// The `(buffa.ext.field).name` value of `field`, or `None` if the schema
/// does not set one.
pub(crate) fn field_name(field: &FieldDescriptorProto) -> Option<String> {
    field
        .options
        .as_option()?
        .extension(&buffa_proto_options::FIELD)?
        .name
}

/// Check every `(buffa.ext.field).name` in `file` before code is generated
/// for it.
///
/// # Errors
///
/// - [`CodeGenError::InvalidNameOption`] if a value cannot be the Rust name
///   it asks for. [`NameOptionProblem`] lists the cases.
/// - [`CodeGenError::NameOptionConflict`] if a value gives two fields of one
///   struct, or two variants of one oneof, the same Rust name.
pub(crate) fn validate_file(
    ctx: &CodeGenContext,
    file: &FileDescriptorProto,
) -> Result<(), CodeGenError> {
    let package = file.package.as_deref().unwrap_or_default();
    reject_on_extensions(&file.extension, package)?;
    for msg in &file.message_type {
        validate_message(ctx, msg, package)?;
    }
    Ok(())
}

fn validate_message(
    ctx: &CodeGenContext,
    msg: &DescriptorProto,
    scope: &str,
) -> Result<(), CodeGenError> {
    let fqn = join_fqn(scope, msg.name.as_deref().unwrap_or_default());
    reject_on_extensions(&msg.extension, &fqn)?;
    for nested in &msg.nested_type {
        validate_message(ctx, nested, &fqn)?;
    }

    let names: Vec<_> = msg.field.iter().map(field_name).collect();
    if names.iter().all(Option::is_none) {
        return Ok(());
    }

    // The struct's fields share one namespace, and the variants of each
    // oneof share another.
    let mut struct_fields = Namespace::default();
    let mut variants: HashMap<i32, Namespace> = HashMap::new();
    for (field, name) in msg.field.iter().zip(names) {
        let element = join_fqn(&fqn, field.name.as_deref().unwrap_or_default());
        let oneof = field.oneof_index.filter(|_| is_real_oneof_member(field));
        let source = match name {
            Some(name) => {
                if let Err(problem) = check_identifier(&name) {
                    return Err(CodeGenError::InvalidNameOption {
                        option: FIELD_NAME_OPTION,
                        element,
                        name,
                        problem,
                    });
                }
                NameSource::Schema
            }
            None => NameSource::Derived,
        };
        let (namespace, rust_name) = match oneof {
            Some(index) => (
                variants.entry(index).or_default(),
                crate::oneof::oneof_variant_ident(field),
            ),
            None => (&mut struct_fields, ctx.field_ident(field)),
        };
        namespace.claim(rust_name.to_string(), element, source)?;
    }
    // A oneof is one field of the struct, so a field's `name` can collide
    // with it.
    for (index, oneof) in msg.oneof_decl.iter().enumerate() {
        let is_real = i32::try_from(index).is_ok_and(|index| variants.contains_key(&index));
        if let (true, Some(proto_name)) = (is_real, oneof.name.as_deref()) {
            let rust_name = ctx.oneof_ident(proto_name).to_string();
            struct_fields.claim(rust_name, join_fqn(&fqn, proto_name), NameSource::Derived)?;
        }
    }
    Ok(())
}

/// Where the Rust name of a member comes from.
#[derive(Clone, Copy)]
enum NameSource {
    /// The schema set it with a `name` option.
    Schema,
    /// buffa derived it from the proto name.
    Derived,
}

/// The member that has taken a Rust name in a [`Namespace`].
struct Taken {
    /// Fully-qualified proto name of the member.
    element: String,
    source: NameSource,
}

/// The Rust names taken in one scope of a message: the fields of its struct,
/// or the variants of one of its oneofs.
#[derive(Default)]
struct Namespace {
    taken: HashMap<String, Taken>,
}

impl Namespace {
    /// Record that the member `element` has the Rust name `rust_name`.
    ///
    /// Two members with one Rust name are an error when a `name` option set
    /// either name. Two derived names that collide are outside this check:
    /// the `idiomatic_field_names` plan adjusts them, or rustc reports them.
    fn claim(
        &mut self,
        rust_name: String,
        element: String,
        source: NameSource,
    ) -> Result<(), CodeGenError> {
        let earlier = match self.taken.entry(rust_name) {
            Entry::Vacant(slot) => {
                slot.insert(Taken { element, source });
                return Ok(());
            }
            Entry::Occupied(earlier) => earlier,
        };
        let earlier_element = earlier.get().element.clone();
        // The error names the member whose option to change. When both
        // members set the option, that is the one declared second.
        let (element, other) = match (source, earlier.get().source) {
            (NameSource::Schema, NameSource::Schema | NameSource::Derived) => {
                (element, earlier_element)
            }
            (NameSource::Derived, NameSource::Schema) => (earlier_element, element),
            (NameSource::Derived, NameSource::Derived) => return Ok(()),
        };
        Err(CodeGenError::NameOptionConflict {
            option: FIELD_NAME_OPTION,
            element,
            other,
            rust_name: earlier.key().clone(),
        })
    }
}

/// Check that `name` can be the name of a struct field or of a oneof variant
/// exactly as written.
fn check_identifier(name: &str) -> Result<(), NameOptionProblem> {
    let mut chars = name.chars();
    let starts_as_identifier = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    if !starts_as_identifier || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') || name == "_"
    {
        Err(NameOptionProblem::NotAnIdentifier)
    } else if crate::idents::is_rust_keyword(name) {
        Err(NameOptionProblem::Keyword)
    } else if name.starts_with("__buffa_") {
        Err(NameOptionProblem::ReservedPrefix)
    } else {
        Ok(())
    }
}

/// Reject a `name` option on an extension. buffa generates a constant for an
/// extension, and derives the constant's name from the proto name.
fn reject_on_extensions(
    extensions: &[FieldDescriptorProto],
    scope: &str,
) -> Result<(), CodeGenError> {
    for extension in extensions {
        if let Some(name) = field_name(extension) {
            return Err(CodeGenError::InvalidNameOption {
                option: FIELD_NAME_OPTION,
                element: join_fqn(scope, extension.name.as_deref().unwrap_or_default()),
                name,
                problem: NameOptionProblem::OnExtension,
            });
        }
    }
    Ok(())
}

/// `scope.name`, or `name` alone when `scope` is the empty package.
fn join_fqn(scope: &str, name: &str) -> String {
    if scope.is_empty() {
        name.to_string()
    } else {
        format!("{scope}.{name}")
    }
}

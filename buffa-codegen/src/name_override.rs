//! Rust names that a schema sets with the `name` setting of a `buffa.ext`
//! option: `(buffa.ext.field).name`, `(buffa.ext.message).name` and
//! `(buffa.ext.enum).name`.
//!
//! [`field_name`] reads the field option, and
//! [`CodeGenContext::field_rust_name`] and
//! [`oneof_variant_ident`](crate::oneof::oneof_variant_ident) use the value
//! as written, in place of the name they derive from the proto name.
//! [`TypeDecl`] does the same for the name of a message or an enum.
//!
//! [`validate_file`] runs before any code is generated for a file. It rejects
//! a value that is unusable as the identifier it asks for, and a value that
//! gives two members of one struct, or two variants of one oneof, the same
//! Rust name. [`check_type_names`] does both for the types of one scope. So
//! the emission code can build an identifier from the value without
//! checking it.

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fmt;

use buffa::ExtensionSet as _;

use crate::context::CodeGenContext;
use crate::generated::descriptor::{
    DescriptorProto, EnumDescriptorProto, FieldDescriptorProto, FileDescriptorProto,
};
use crate::impl_message::is_real_oneof_member;
use crate::CodeGenError;

/// The option that [`field_name`] reads, as a schema writes it.
const FIELD_NAME_OPTION: &str = "(buffa.ext.field).name";
/// The option that [`TypeDecl::message`] reads, as a schema writes it.
const MESSAGE_NAME_OPTION: &str = "(buffa.ext.message).name";
/// The option that [`TypeDecl::enumeration`] reads, as a schema writes it.
const ENUM_NAME_OPTION: &str = "(buffa.ext.enum).name";

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
    /// On a message or an enum, the value can also be `__buffa`, the module
    /// that holds buffa's ancillary types.
    ReservedPrefix,
    /// The option is on a message or an enum, and the value is the name of a
    /// primitive type that generated code uses: `bool`, `str`, `u8`,
    /// `usize`, `i32`, `i64`, `u32`, `u64`, `f32` or `f64`. A type with that
    /// name would shadow the primitive for the generated code in its module.
    PrimitiveType,
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
                "`__buffa` and names that start with `__buffa_` are reserved for buffa's own \
                 identifiers",
            ),
            Self::PrimitiveType => f.write_str(
                "the value is a primitive type name that generated code uses (`bool`, `str`, \
                 `u8`, `usize`, `i32`, `i64`, `u32`, `u64`, `f32`, `f64`), and a type with \
                 that name would shadow it; choose another name, such as `Bool`",
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

/// A message or an enum as its descriptor declares it: the proto name, and
/// the `name` setting of its `buffa.ext` option.
pub(crate) struct TypeDecl<'a> {
    /// The simple proto name. Empty if the descriptor has none.
    proto_name: &'a str,
    /// The value of the `name` setting, unchecked.
    option: Option<String>,
    /// The setting as a schema writes it.
    option_path: &'static str,
}

impl<'a> TypeDecl<'a> {
    /// `msg` with its `(buffa.ext.message).name`.
    pub(crate) fn message(msg: &'a DescriptorProto) -> Self {
        let option = msg
            .options
            .as_option()
            .and_then(|options| options.extension(&buffa_proto_options::MESSAGE))
            .and_then(|settings| settings.name);
        Self {
            proto_name: msg.name.as_deref().unwrap_or_default(),
            option,
            option_path: MESSAGE_NAME_OPTION,
        }
    }

    /// `enum_type` with its `(buffa.ext.enum).name`.
    pub(crate) fn enumeration(enum_type: &'a EnumDescriptorProto) -> Self {
        let option = enum_type
            .options
            .as_option()
            .and_then(|options| options.extension(&buffa_proto_options::ENUM))
            .and_then(|settings| settings.name);
        Self {
            proto_name: enum_type.name.as_deref().unwrap_or_default(),
            option,
            option_path: ENUM_NAME_OPTION,
        }
    }

    /// The simple proto name, or an empty string if the descriptor has
    /// none.
    pub(crate) fn proto_name(&self) -> &'a str {
        self.proto_name
    }

    /// The value of the `name` setting, if it can be the name of a type.
    ///
    /// [`check_type_names`] rejects any other value in a file that code is
    /// generated for. In an imported file no error is reported, and a value
    /// that cannot be the name is ignored: the crate that owns the file
    /// cannot have generated a type with that name.
    pub(crate) fn name_option(&self) -> Option<&str> {
        self.option
            .as_deref()
            .filter(|name| check_type_identifier(name).is_ok())
    }

    /// The Rust name of the type in the crate that generates it: the `name`
    /// setting as written, or without one the proto name with
    /// `type_name_prefix` and the escaping of
    /// [`local_type_name`](crate::idents::local_type_name).
    pub(crate) fn rust_name(&self, type_name_prefix: &str) -> String {
        match self.name_option() {
            Some(name) => name.to_string(),
            None => crate::idents::local_type_name(type_name_prefix, self.proto_name),
        }
    }

    /// Reject a `name` setting that cannot be the name of a type.
    fn check_option(&self, scope: &str) -> Result<(), CodeGenError> {
        let Some(name) = &self.option else {
            return Ok(());
        };
        check_type_identifier(name).map_err(|problem| CodeGenError::InvalidNameOption {
            option: self.option_path,
            element: join_fqn(scope, self.proto_name),
            name: name.clone(),
            problem,
        })
    }
}

/// The messages and enums declared in one scope: a file's top level, or the
/// body of a message.
pub(crate) fn type_decls<'a>(
    messages: &'a [DescriptorProto],
    enums: &'a [EnumDescriptorProto],
) -> impl Iterator<Item = TypeDecl<'a>> {
    messages
        .iter()
        .map(TypeDecl::message)
        .chain(enums.iter().map(TypeDecl::enumeration))
}

/// Checks the `name` settings of the types of one scope, and rejects two
/// types that have the same Rust name. A type without a proto name is
/// skipped; `validate_file` in the crate root reports it.
///
/// `scope` is the proto package, or the fully-qualified name of the message
/// that the types are nested in.
///
/// # Errors
///
/// - [`CodeGenError::InvalidNameOption`] if a `name` setting cannot be the
///   name of a type. [`NameOptionProblem`] lists the cases.
/// - [`CodeGenError::NameOptionConflict`] if two types have one Rust name
///   and a `name` setting set either name.
/// - [`CodeGenError::TypeNameConflict`] if buffa derived both names.
pub(crate) fn check_type_names<'a>(
    scope: &str,
    decls: impl Iterator<Item = TypeDecl<'a>>,
    type_name_prefix: &str,
) -> Result<(), CodeGenError> {
    let mut types = Namespace::default();
    for decl in decls.filter(|decl| !decl.proto_name.is_empty()) {
        // Before the names are compared: an unusable value falls back to
        // the derived name, and a clash on that name would hide the cause.
        decl.check_option(scope)?;
        let source = match decl.name_option() {
            Some(_) => NameSource::Schema(decl.option_path),
            None => NameSource::Derived,
        };
        let rust_name = decl.rust_name(type_name_prefix);
        let element = join_fqn(scope, decl.proto_name);
        match types.claim(rust_name, element, source) {
            Ok(()) => {}
            Err(Clash::Schema(conflict)) => return Err(conflict),
            Err(Clash::Derived { earlier, rust_name }) => {
                return Err(CodeGenError::TypeNameConflict {
                    scope: scope.to_string(),
                    first_type: simple_name(scope, &earlier).to_string(),
                    second_type: decl.proto_name.to_string(),
                    rust_name,
                });
            }
        }
    }
    Ok(())
}

/// Checks every `(buffa.ext.field).name` in `file` before code is generated
/// for it. [`check_type_names`] checks the settings of messages and enums.
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
                NameSource::Schema(FIELD_NAME_OPTION)
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
        claim_member(namespace, rust_name.to_string(), element, source)?;
    }
    // A oneof is one field of the struct, so a field's `name` can collide
    // with it.
    for (index, oneof) in msg.oneof_decl.iter().enumerate() {
        let is_real = i32::try_from(index).is_ok_and(|index| variants.contains_key(&index));
        if let (true, Some(proto_name)) = (is_real, oneof.name.as_deref()) {
            let rust_name = ctx.oneof_ident(proto_name).to_string();
            let element = join_fqn(&fqn, proto_name);
            claim_member(&mut struct_fields, rust_name, element, NameSource::Derived)?;
        }
    }
    Ok(())
}

/// Where the Rust name of a member comes from.
#[derive(Clone, Copy)]
enum NameSource {
    /// The schema set it with the `name` setting that the string gives, as
    /// a schema writes it.
    Schema(&'static str),
    /// buffa derived it from the proto name.
    Derived,
}

/// The member that has taken a Rust name in a [`Namespace`].
struct Taken {
    /// Fully-qualified proto name of the member.
    element: String,
    source: NameSource,
}

/// Two members of one [`Namespace`] with the same Rust name.
enum Clash {
    /// A `name` setting set one of the two names. The error is a
    /// [`CodeGenError::NameOptionConflict`].
    Schema(CodeGenError),
    /// buffa derived both names. `earlier` is the fully-qualified proto name
    /// of the member that claimed `rust_name` first.
    Derived { earlier: String, rust_name: String },
}

/// The Rust names taken in one scope: the fields of a struct, the variants
/// of a oneof, or the types of a package or of a message.
#[derive(Default)]
struct Namespace {
    taken: HashMap<String, Taken>,
}

impl Namespace {
    /// Record that the member `element` has the Rust name `rust_name`.
    ///
    /// When another member already has the name, the first one keeps it and
    /// the result is the [`Clash`].
    fn claim(
        &mut self,
        rust_name: String,
        element: String,
        source: NameSource,
    ) -> Result<(), Clash> {
        let earlier = match self.taken.entry(rust_name) {
            Entry::Vacant(slot) => {
                slot.insert(Taken { element, source });
                return Ok(());
            }
            Entry::Occupied(earlier) => earlier,
        };
        let earlier_element = earlier.get().element.clone();
        // The error names the member whose option to change. When both
        // members set an option, that is the one that claims second.
        let (option, element, other) = match (source, earlier.get().source) {
            (NameSource::Schema(option), NameSource::Schema(_) | NameSource::Derived) => {
                (option, element, earlier_element)
            }
            (NameSource::Derived, NameSource::Schema(option)) => (option, earlier_element, element),
            (NameSource::Derived, NameSource::Derived) => {
                return Err(Clash::Derived {
                    earlier: earlier_element,
                    rust_name: earlier.key().clone(),
                })
            }
        };
        Err(Clash::Schema(CodeGenError::NameOptionConflict {
            option,
            element,
            other,
            rust_name: earlier.key().clone(),
        }))
    }
}

/// Claim the name of a struct field, a oneof or a oneof variant.
///
/// Two derived names that collide are outside this check: the
/// `idiomatic_field_names` plan adjusts them, or rustc reports them.
fn claim_member(
    namespace: &mut Namespace,
    rust_name: String,
    element: String,
    source: NameSource,
) -> Result<(), CodeGenError> {
    match namespace.claim(rust_name, element, source) {
        Ok(()) | Err(Clash::Derived { .. }) => Ok(()),
        Err(Clash::Schema(conflict)) => Err(conflict),
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

/// Check that `name` can be the name of a struct or an enum exactly as
/// written.
fn check_type_identifier(name: &str) -> Result<(), NameOptionProblem> {
    check_identifier(name)?;
    if name == crate::context::SENTINEL_MOD {
        // A type with this name at a package root would sit beside the
        // `__buffa` module.
        Err(NameOptionProblem::ReservedPrefix)
    } else if crate::idents::is_generated_primitive(name) {
        Err(NameOptionProblem::PrimitiveType)
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

/// The part of `fqn` after `scope`: the inverse of [`join_fqn`].
fn simple_name<'a>(scope: &str, fqn: &'a str) -> &'a str {
    fqn.strip_prefix(scope)
        .map_or(fqn, |rest| rest.strip_prefix('.').unwrap_or(rest))
}

/// `scope.name`, or `name` alone when `scope` is the empty package.
fn join_fqn(scope: &str, name: &str) -> String {
    if scope.is_empty() {
        name.to_string()
    } else {
        format!("{scope}.{name}")
    }
}

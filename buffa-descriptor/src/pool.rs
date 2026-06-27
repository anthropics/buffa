//! Runtime descriptor pool.
//!
//! [`DescriptorPool`] takes one or more `FileDescriptorProto`s and produces a
//! flat, linked, feature-resolved set of [`MessageDescriptor`]s and
//! [`EnumDescriptor`]s. Cross-references (a field of message type, an enum
//! value type) are resolved to pool-local [`MessageIndex`] / [`EnumIndex`]
//! handles. Edition features (presence, packed, delimited, enum openness) are
//! resolved at build time, so every [`FieldDescriptor`] carries final values
//! and consumers never need to walk a `FeatureSet` chain.
//!
//! Construction is two-pass:
//!
//! 1. **Register**: walk every file, recording the fully-qualified name of
//!    every message and enum (including nested ones) and assigning each a
//!    pool index. This makes forward references and cross-file references
//!    resolvable in the second pass.
//! 2. **Link**: walk every file again, building the linked [`MessageDescriptor`]
//!    for each message: resolving `type_name` strings to indices, classifying
//!    fields as singular / list / map, resolving features down the
//!    file → message → field chain, and validating `u16` field-count limits.
//!
//! The pool retains the original `FileDescriptorProto`s after linking — gRPC
//! server reflection needs the raw bytes, and they're cheap to keep relative
//! to the linked structures.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::desc::{
    EnumDescriptor, EnumIndex, EnumValueDescriptor, ExtensionDescriptor, ExtensionIndex,
    FieldDescriptor, FieldKind, MessageDescriptor, MessageIndex, MethodDescriptor, OneofDescriptor,
    ScalarType, ServiceDescriptor, ServiceIndex, SingularKind,
};
use crate::features::{self, ResolvedFeatures};
use crate::generated::descriptor::field_descriptor_proto::{Label, Type as ProtoType};
use crate::generated::descriptor::{
    DescriptorProto, EnumDescriptorProto, FieldDescriptorProto, FileDescriptorProto,
    FileDescriptorSet, ServiceDescriptorProto,
};
use buffa::editions::{EnumType, FieldPresence, MessageEncoding, RepeatedFieldEncoding};
use buffa::MessageField;

/// Clone a descriptor's raw `*Options` into a boxed `Option`, the form the
/// linked descriptors store. `None` for the common no-options case; one
/// allocation only when options are present. Generic over the source field's
/// pointer (`Inline` for non-recursive fields under the default, `Box` for the
/// recursive `FieldOptions.features` chain).
fn clone_options<T: Clone + Default, P: buffa::ProtoBox<T>>(
    opts: &MessageField<T, P>,
) -> Option<Box<T>> {
    opts.as_option().cloned().map(Box::new)
}

/// Errors that can occur while building a [`DescriptorPool`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum PoolError {
    /// The `FileDescriptorSet` bytes did not decode. Carries the underlying
    /// wire-format error.
    Decode(buffa::DecodeError),
    /// A field had no `type_name` for a `TYPE_MESSAGE`/`TYPE_GROUP`/`TYPE_ENUM`.
    MissingTypeName { field: String },
    /// A field's `type_name` did not resolve to any registered message or
    /// enum. Carries the dangling name and the field's fully-qualified name.
    UnresolvedTypeName { type_name: String, field: String },
    /// A field's `type_name` resolved to the wrong kind (e.g. a `TYPE_ENUM`
    /// field referencing a message). Carries the name and the field.
    WrongTypeKind { type_name: String, field: String },
    /// Two messages or enums declared the same fully-qualified name.
    DuplicateName(String),
    /// A message has more than 65 535 fields, exceeding the `u16` index
    /// limit of the internal field-number lookup table behind
    /// [`MessageDescriptor::field`].
    TooManyFields { message: String, count: usize },
    /// A field number is outside the valid range
    /// `[1, MAX_FIELD_NUMBER]` (`(1 << 29) - 1`), or an extension range has
    /// a negative bound.
    InvalidFieldNumber { field: String, number: i32 },
    /// A map entry message did not have exactly fields 1 (key) and 2 (value),
    /// or the key type is not a valid map key per the protobuf spec.
    MalformedMapEntry { message: String },
    /// Two extensions claim the same field number on the same message.
    /// protoc rejects this within one compilation unit, but it can arise
    /// when merging independently-compiled `FileDescriptorSet`s.
    DuplicateExtensionNumber { extendee: String, number: u32 },
}

impl core::fmt::Display for PoolError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Decode(e) => write!(f, "FileDescriptorSet decode failed: {e}"),
            Self::MissingTypeName { field } => write!(f, "field {field} has no type_name"),
            Self::UnresolvedTypeName { type_name, field } => {
                write!(f, "unresolved type name {type_name:?} on field {field}")
            }
            Self::WrongTypeKind { type_name, field } => {
                write!(
                    f,
                    "type name {type_name:?} on field {field} resolves to the wrong kind"
                )
            }
            Self::DuplicateName(name) => write!(f, "duplicate type name {name:?}"),
            Self::TooManyFields { message, count } => {
                write!(
                    f,
                    "message {message} has {count} fields, exceeding the u16 limit"
                )
            }
            Self::InvalidFieldNumber { field, number } => {
                write!(f, "field {field} has invalid field number {number}")
            }
            Self::MalformedMapEntry { message } => {
                write!(f, "malformed map entry message {message}")
            }
            Self::DuplicateExtensionNumber { extendee, number } => {
                write!(
                    f,
                    "more than one extension claims field number {number} on {extendee}"
                )
            }
        }
    }
}

impl From<buffa::DecodeError> for PoolError {
    fn from(e: buffa::DecodeError) -> Self {
        Self::Decode(e)
    }
}

#[cfg(feature = "std")]
impl std::error::Error for PoolError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Decode(e) => Some(e),
            _ => None,
        }
    }
}

/// What a fully-qualified name resolves to within the pool.
#[derive(Clone, Copy, Debug)]
enum Definition {
    Message(MessageIndex),
    Enum(EnumIndex),
}

/// A pool of linked, feature-resolved protobuf descriptors.
///
/// Built from one or more `FileDescriptorProto`s via [`DescriptorPool::new`]
/// or accumulated via [`DescriptorPool::add_file_descriptor_set`]. Once built,
/// the pool is immutable — descriptor handles are pool indices and all data
/// is stored in flat `Vec`s.
#[derive(Debug, Default)]
pub struct DescriptorPool {
    /// Original file descriptors, retained for raw access.
    files: Vec<FileDescriptorProto>,
    /// Linked message descriptors, indexed by [`MessageIndex`].
    messages: Vec<MessageDescriptor>,
    /// Linked enum descriptors, indexed by [`EnumIndex`].
    enums: Vec<EnumDescriptor>,
    /// Linked service descriptors, indexed by [`ServiceIndex`].
    services: Vec<ServiceDescriptor>,
    /// Linked extension descriptors, indexed by [`ExtensionIndex`].
    extensions: Vec<ExtensionDescriptor>,
    /// FQN (no leading dot) → definition lookup.
    by_name: BTreeMap<String, Definition>,
    /// Service FQN (no leading dot) → index. Separate from `by_name`
    /// because `Definition` is `MessageIndex`-or-`EnumIndex` and services
    /// are linked in a single pass after types resolve.
    service_by_name: BTreeMap<String, ServiceIndex>,
    /// Extension FQN (no leading dot) → index. The JSON-parse lookup for
    /// `"[pkg.ext]"` keys.
    extension_by_name: BTreeMap<String, ExtensionIndex>,
    /// `(extendee, field number)` → index. The wire-decode and
    /// JSON-serialize lookup ("this number on this message is which
    /// extension?"), and the backing store for
    /// [`extensions_of`](Self::extensions_of) via a range scan.
    extension_by_extendee: BTreeMap<(MessageIndex, u32), ExtensionIndex>,
    /// Filename → index into `files`.
    file_by_name: BTreeMap<String, usize>,
    /// Declared symbol FQN → index into `files`. Covers messages (including
    /// nested), enums (including nested), services, methods, and extensions
    /// — the symbols gRPC server reflection's `FindFileContainingSymbol`
    /// resolves.
    symbol_file: BTreeMap<String, usize>,
}

impl DescriptorPool {
    /// Build a pool from a single `FileDescriptorSet`.
    ///
    /// # Errors
    ///
    /// Returns a [`PoolError`] if any type name fails to resolve, a name is
    /// declared twice, a message exceeds 65 535 fields, or a map entry is
    /// malformed.
    pub fn new(set: FileDescriptorSet) -> Result<Self, PoolError> {
        let mut pool = Self::default();
        pool.add_file_descriptor_set(set)?;
        Ok(pool)
    }

    /// Build a pool from raw `FileDescriptorSet` bytes.
    ///
    /// `bytes` is treated as untrusted input — consumers loading a
    /// `FileDescriptorSet` from a schema registry, gRPC server reflection
    /// peer, or on-disk policy bundle should call this rather than decoding
    /// and calling [`DescriptorPool::new`] separately.
    ///
    /// # Errors
    ///
    /// Returns [`PoolError::Decode`] if the bytes are not a well-formed
    /// `FileDescriptorSet`, or any other [`PoolError`] on a structural
    /// validation failure (dangling type names, out-of-range field numbers,
    /// duplicate types, malformed map entries).
    pub fn decode(bytes: &[u8]) -> Result<Self, PoolError> {
        use buffa::Message;
        let set = FileDescriptorSet::decode_from_slice(bytes)?;
        Self::new(set)
    }

    /// Add the files in a `FileDescriptorSet` to the pool, registering and
    /// linking new types. Files already in the pool (by filename) are skipped.
    ///
    /// # Errors
    ///
    /// Returns a [`PoolError`] on resolution failure.
    pub fn add_file_descriptor_set(&mut self, set: FileDescriptorSet) -> Result<(), PoolError> {
        // Filter out files already present (idempotent re-add).
        let new_files: Vec<FileDescriptorProto> = set
            .file
            .into_iter()
            .filter(|f| {
                f.name
                    .as_deref()
                    // MSRV: `Option::is_none_or` requires 1.82.
                    .map_or(true, |n| !self.file_by_name.contains_key(n))
            })
            .collect();
        if new_files.is_empty() {
            return Ok(());
        }

        // Pass 1: register all message/enum FQNs and assign indices.
        // This walk is over the new files only; existing names are already in
        // `by_name`.
        let first_new_message = self.messages.len();
        for file in &new_files {
            let pkg = file.package.as_deref().unwrap_or("");
            for msg in &file.message_type {
                self.register_message(pkg, msg)?;
            }
            for e in &file.enum_type {
                self.register_enum(pkg, e)?;
            }
        }

        // Pass 2: link. We need to iterate the new files again to fill in
        // the placeholder `MessageDescriptor`s. Walk in the same order.
        let mut linked = first_new_message;
        for file in &new_files {
            let pkg = file.package.as_deref().unwrap_or("");
            let file_features = features::for_file(file);
            for msg in &file.message_type {
                linked = self.link_message(pkg, msg, &file_features, linked)?;
            }
            for e in &file.enum_type {
                self.link_enum(pkg, e, &file_features)?;
            }
        }
        debug_assert_eq!(linked, self.messages.len());

        // Pass 3: link services and extensions. Both reference message types
        // by name (a service's method input/output, an extension's extendee
        // and value type), so they link after the type passes. There's no
        // register/link split because neither has forward references to its
        // own kind.
        for file in &new_files {
            let pkg = file.package.as_deref().unwrap_or("");
            let file_features = features::for_file(file);
            for svc in &file.service {
                self.link_service(pkg, svc)?;
            }
            // File-level extensions: `extend Foo { ... }` at the top level.
            for ext in &file.extension {
                self.link_extension(pkg, ext, &file_features)?;
            }
            // Message-scoped extensions: `message Scope { extend Foo {...} }`,
            // registered under `pkg.Scope.ext_name`. Recurses into nested
            // messages.
            for msg in &file.message_type {
                self.link_nested_extensions(pkg, msg, &file_features)?;
            }
        }

        // Record filenames (for idempotent re-add) and the symbol → file
        // index (for `FindFileContainingSymbol`).
        let base = self.files.len();
        for (i, f) in new_files.iter().enumerate() {
            let file_idx = base + i;
            if let Some(n) = f.name.as_deref() {
                self.file_by_name.insert(n.to_string(), file_idx);
            }
            self.index_file_symbols(f, file_idx);
        }
        self.files.extend(new_files);

        Ok(())
    }

    /// Record every symbol declared in `file` into `symbol_file`. Indexes the
    /// full set of named descriptors gRPC server reflection's
    /// `FindFileContainingSymbol` accepts — messages, fields, oneofs,
    /// enums, enum values, services, methods, and extensions — each at its
    /// fully-qualified name, all mapping to the declaring file.
    fn index_file_symbols(&mut self, file: &FileDescriptorProto, file_idx: usize) {
        let pkg = file.package.as_deref().unwrap_or("");
        let join = |scope: &str, name: &str| {
            if scope.is_empty() {
                name.to_string()
            } else {
                format!("{scope}.{name}")
            }
        };
        for msg in &file.message_type {
            self.index_message_symbols(pkg, msg, file_idx);
        }
        for e in &file.enum_type {
            self.index_enum_symbols(pkg, e, file_idx);
        }
        for svc in &file.service {
            let svc_fqn = join(pkg, svc.name.as_deref().unwrap_or(""));
            for m in &svc.method {
                self.symbol_file.insert(
                    format!("{svc_fqn}.{}", m.name.as_deref().unwrap_or("")),
                    file_idx,
                );
            }
            self.symbol_file.insert(svc_fqn, file_idx);
        }
        for ext in &file.extension {
            self.symbol_file
                .insert(join(pkg, ext.name.as_deref().unwrap_or("")), file_idx);
        }
    }

    /// Recursive helper for [`index_file_symbols`](Self::index_file_symbols):
    /// records `msg` and everything declared inside it (fields, oneofs,
    /// nested messages, nested enums, message-scoped extensions).
    fn index_message_symbols(&mut self, scope: &str, msg: &DescriptorProto, file_idx: usize) {
        let name = msg.name.as_deref().unwrap_or("");
        let fqn = if scope.is_empty() {
            name.to_string()
        } else {
            format!("{scope}.{name}")
        };
        for field in &msg.field {
            self.symbol_file.insert(
                format!("{fqn}.{}", field.name.as_deref().unwrap_or("")),
                file_idx,
            );
        }
        for oneof in &msg.oneof_decl {
            self.symbol_file.insert(
                format!("{fqn}.{}", oneof.name.as_deref().unwrap_or("")),
                file_idx,
            );
        }
        for nested in &msg.nested_type {
            self.index_message_symbols(&fqn, nested, file_idx);
        }
        for e in &msg.enum_type {
            self.index_enum_symbols(&fqn, e, file_idx);
        }
        for ext in &msg.extension {
            self.symbol_file.insert(
                format!("{fqn}.{}", ext.name.as_deref().unwrap_or("")),
                file_idx,
            );
        }
        self.symbol_file.insert(fqn, file_idx);
    }

    /// Record an enum and its values. Enum values live in the enum's
    /// *parent* scope per protobuf naming (`pkg.VALUE`, not
    /// `pkg.Enum.VALUE`), matching how gRPC reflection resolves them.
    fn index_enum_symbols(&mut self, scope: &str, e: &EnumDescriptorProto, file_idx: usize) {
        let fqn = if scope.is_empty() {
            e.name.clone().unwrap_or_default()
        } else {
            format!("{scope}.{}", e.name.as_deref().unwrap_or(""))
        };
        for v in &e.value {
            self.symbol_file.insert(
                format!("{scope}.{}", v.name.as_deref().unwrap_or("")),
                file_idx,
            );
        }
        self.symbol_file.insert(fqn, file_idx);
    }

    // ── Public lookup API ──────────────────────────────────────────────────

    /// Look up a message by fully-qualified name (no leading dot).
    #[must_use]
    pub fn message_by_name(&self, full_name: &str) -> Option<&MessageDescriptor> {
        let name = full_name.strip_prefix('.').unwrap_or(full_name);
        match self.by_name.get(name)? {
            Definition::Message(idx) => Some(&self.messages[idx.0 as usize]),
            Definition::Enum(_) => None,
        }
    }

    /// Look up an enum by fully-qualified name (no leading dot).
    #[must_use]
    pub fn enum_by_name(&self, full_name: &str) -> Option<&EnumDescriptor> {
        let name = full_name.strip_prefix('.').unwrap_or(full_name);
        match self.by_name.get(name)? {
            Definition::Enum(idx) => Some(&self.enums[idx.0 as usize]),
            Definition::Message(_) => None,
        }
    }

    /// Look up a message by its [`MessageIndex`].
    ///
    /// Indices are stable for the lifetime of the pool — adding files via
    /// [`add_file_descriptor_set`](Self::add_file_descriptor_set) only appends
    /// new entries.
    ///
    /// # Panics
    ///
    /// Panics if `idx` was issued by a *different* pool whose message count
    /// is smaller than this one's. `MessageIndex` carries no pool identity;
    /// passing an index across pools is a logic error and may also silently
    /// return the wrong descriptor without panicking. Hold one pool per
    /// schema and don't mix indices.
    #[must_use]
    pub fn message(&self, idx: MessageIndex) -> &MessageDescriptor {
        &self.messages[idx.0 as usize]
    }

    /// Look up an enum by its [`EnumIndex`].
    ///
    /// # Panics
    ///
    /// Same cross-pool hazard as [`Self::message`].
    #[must_use]
    pub fn enumeration(&self, idx: EnumIndex) -> &EnumDescriptor {
        &self.enums[idx.0 as usize]
    }

    /// The [`MessageIndex`] for a fully-qualified name, if present.
    #[must_use]
    pub fn message_index(&self, full_name: &str) -> Option<MessageIndex> {
        let name = full_name.strip_prefix('.').unwrap_or(full_name);
        match self.by_name.get(name)? {
            Definition::Message(idx) => Some(*idx),
            Definition::Enum(_) => None,
        }
    }

    /// The [`EnumIndex`] for a fully-qualified name, if present.
    #[must_use]
    pub fn enum_index(&self, full_name: &str) -> Option<EnumIndex> {
        let name = full_name.strip_prefix('.').unwrap_or(full_name);
        match self.by_name.get(name)? {
            Definition::Enum(idx) => Some(*idx),
            Definition::Message(_) => None,
        }
    }

    /// All linked messages, in pool index order.
    #[must_use]
    pub fn messages(&self) -> &[MessageDescriptor] {
        &self.messages
    }

    /// All linked enums, in pool index order.
    #[must_use]
    pub fn enums(&self) -> &[EnumDescriptor] {
        &self.enums
    }

    /// All linked services, in pool index order.
    #[must_use]
    pub fn services(&self) -> &[ServiceDescriptor] {
        &self.services
    }

    /// Look up a service by its fully-qualified proto name.
    #[must_use]
    pub fn service_by_name(&self, full_name: &str) -> Option<&ServiceDescriptor> {
        let name = full_name.strip_prefix('.').unwrap_or(full_name);
        let idx = self.service_by_name.get(name)?;
        self.services.get(idx.0 as usize)
    }

    /// Look up a service by its [`ServiceIndex`].
    ///
    /// # Panics
    ///
    /// Same cross-pool hazard as [`Self::message`].
    #[must_use]
    pub fn service(&self, idx: ServiceIndex) -> &ServiceDescriptor {
        &self.services[idx.0 as usize]
    }

    /// The [`ServiceIndex`] for a fully-qualified name, if present.
    #[must_use]
    pub fn service_index(&self, full_name: &str) -> Option<ServiceIndex> {
        let name = full_name.strip_prefix('.').unwrap_or(full_name);
        self.service_by_name.get(name).copied()
    }

    /// All linked extensions, in pool index order.
    #[must_use]
    pub fn extensions(&self) -> &[ExtensionDescriptor] {
        &self.extensions
    }

    /// Look up an extension by its fully-qualified registration name
    /// (`pkg.ext_name` for file-level, `pkg.Scope.ext_name` for one declared
    /// inside a message). This is the JSON `"[...]"` key without the
    /// brackets.
    #[must_use]
    pub fn extension_by_name(&self, full_name: &str) -> Option<&ExtensionDescriptor> {
        let name = full_name.strip_prefix('.').unwrap_or(full_name);
        let idx = self.extension_by_name.get(name)?;
        self.extensions.get(idx.0 as usize)
    }

    /// Look up the extension that occupies field number `number` on
    /// `extendee`, if one is registered.
    ///
    /// This is the wire-decode and JSON-serialize lookup: "this field number
    /// is in `extendee`'s extension range — which extension is it?"
    #[must_use]
    pub fn extension_for(
        &self,
        extendee: MessageIndex,
        number: u32,
    ) -> Option<&ExtensionDescriptor> {
        let idx = self.extension_by_extendee.get(&(extendee, number))?;
        self.extensions.get(idx.0 as usize)
    }

    /// All registered extensions of `extendee`, in field-number order.
    pub fn extensions_of(
        &self,
        extendee: MessageIndex,
    ) -> impl Iterator<Item = &ExtensionDescriptor> {
        self.extension_by_extendee
            .range((extendee, 0)..=(extendee, u32::MAX))
            .filter_map(|(_, idx)| self.extensions.get(idx.0 as usize))
    }

    /// The [`ExtensionIndex`] for a fully-qualified registration name, if
    /// present.
    #[must_use]
    pub fn extension_index(&self, full_name: &str) -> Option<ExtensionIndex> {
        let name = full_name.strip_prefix('.').unwrap_or(full_name);
        self.extension_by_name.get(name).copied()
    }

    /// Look up an extension by its [`ExtensionIndex`].
    ///
    /// # Panics
    ///
    /// Same cross-pool hazard as [`Self::message`].
    #[must_use]
    pub fn extension(&self, idx: ExtensionIndex) -> &ExtensionDescriptor {
        &self.extensions[idx.0 as usize]
    }

    /// The original `FileDescriptorProto`s the pool was built from.
    #[must_use]
    pub fn files(&self) -> &[FileDescriptorProto] {
        &self.files
    }

    /// Look up a `FileDescriptorProto` by filename.
    #[must_use]
    pub fn file_by_name(&self, name: &str) -> Option<&FileDescriptorProto> {
        let idx = *self.file_by_name.get(name)?;
        Some(&self.files[idx])
    }

    /// The `FileDescriptorProto` that declares a fully-qualified symbol, the
    /// way gRPC server reflection's `FindFileContainingSymbol` resolves it.
    ///
    /// Resolves messages (including nested), enums (including nested),
    /// services, methods (`pkg.Service.Method`), and extensions — every
    /// symbol kind a reflection client queries. `O(log n)` over the symbol
    /// index.
    #[must_use]
    pub fn file_containing_symbol(&self, full_name: &str) -> Option<&FileDescriptorProto> {
        let name = full_name.strip_prefix('.').unwrap_or(full_name);
        let idx = *self.symbol_file.get(name)?;
        Some(&self.files[idx])
    }

    // ── Pass 1: register names ──────────────────────────────────────────────

    fn register_message(
        &mut self,
        parent_fqn: &str,
        msg: &DescriptorProto,
    ) -> Result<(), PoolError> {
        let name = msg.name.as_deref().unwrap_or("");
        let fqn = if parent_fqn.is_empty() {
            name.to_string()
        } else {
            format!("{parent_fqn}.{name}")
        };
        let idx = MessageIndex(
            u32::try_from(self.messages.len()).expect("pool message count fits in u32"),
        );
        if self
            .by_name
            .insert(fqn.clone(), Definition::Message(idx))
            .is_some()
        {
            return Err(PoolError::DuplicateName(fqn));
        }
        // Push a placeholder; pass 2 fills it in.
        self.messages.push(MessageDescriptor {
            full_name: fqn.clone(),
            fields: Vec::new(),
            field_by_number: Vec::new(),
            field_by_name: Vec::new(),
            oneofs: Vec::new(),
            extension_ranges: Vec::new(),
            options: None,
        });
        for nested in &msg.nested_type {
            self.register_message(&fqn, nested)?;
        }
        for nested_enum in &msg.enum_type {
            self.register_enum(&fqn, nested_enum)?;
        }
        Ok(())
    }

    fn register_enum(
        &mut self,
        parent_fqn: &str,
        e: &EnumDescriptorProto,
    ) -> Result<(), PoolError> {
        let name = e.name.as_deref().unwrap_or("");
        let fqn = if parent_fqn.is_empty() {
            name.to_string()
        } else {
            format!("{parent_fqn}.{name}")
        };
        let idx = EnumIndex(u32::try_from(self.enums.len()).expect("pool enum count fits in u32"));
        if self
            .by_name
            .insert(fqn.clone(), Definition::Enum(idx))
            .is_some()
        {
            return Err(PoolError::DuplicateName(fqn));
        }
        // Enums don't need a second pass — they have no cross-references —
        // so we can't fully link them here either, because feature resolution
        // walks the message hierarchy. Push a placeholder.
        self.enums.push(EnumDescriptor {
            full_name: fqn,
            values: Vec::new(),
            enum_type: EnumType::Open,
            options: None,
        });
        Ok(())
    }

    // ── Pass 2: link ────────────────────────────────────────────────────────

    /// Link a message and its nested messages/enums. Returns the index after
    /// the last message linked (used to walk in registration order).
    fn link_message(
        &mut self,
        parent_fqn: &str,
        msg: &DescriptorProto,
        parent_features: &ResolvedFeatures,
        next_index: usize,
    ) -> Result<usize, PoolError> {
        let name = msg.name.as_deref().unwrap_or("");
        let fqn = if parent_fqn.is_empty() {
            name.to_string()
        } else {
            format!("{parent_fqn}.{name}")
        };
        let msg_features =
            features::resolve_child(parent_features, features::message_features(msg));

        // u16 field index cap.
        let field_count = msg.field.len();
        if field_count > u16::MAX as usize {
            return Err(PoolError::TooManyFields {
                message: fqn,
                count: field_count,
            });
        }

        // Build oneof descriptors. Track member field indices as we go.
        let mut oneofs: Vec<OneofDescriptor> = msg
            .oneof_decl
            .iter()
            .map(|o| OneofDescriptor {
                name: o.name.clone().unwrap_or_default(),
                field_indices: Vec::new(),
                synthetic: false,
                options: clone_options(&o.options),
            })
            .collect();

        // Build field descriptors.
        let mut fields = Vec::with_capacity(field_count);
        let mut field_by_number: Vec<(u32, u16)> = Vec::with_capacity(field_count);
        let mut field_by_name: Vec<(String, u16)> = Vec::with_capacity(field_count * 2);
        for (i, f) in msg.field.iter().enumerate() {
            let fd = self.link_field(&fqn, f, &msg_features, Some(msg))?;
            let i16 = i as u16;
            // Wire up oneof membership.
            if let Some(oneof_idx) = fd.oneof_index {
                let oi = oneof_idx as usize;
                if let Some(o) = oneofs.get_mut(oi) {
                    o.field_indices.push(i16);
                }
            }
            field_by_number.push((fd.number, i16));
            // Index both the proto name and the JSON name so a single
            // `field_by_name` resolves either — JSON parsers must accept
            // both per the proto3 JSON spec, and CEL evaluators look up by
            // the proto name.
            field_by_name.push((fd.name.clone(), i16));
            if fd.json_name != fd.name {
                field_by_name.push((fd.json_name.clone(), i16));
            }
            fields.push(fd);
        }
        field_by_number.sort_unstable_by_key(|&(n, _)| n);
        field_by_name.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));

        // Mark synthetic oneofs (proto3 optional). Per protobuf semantics,
        // a synthetic oneof has exactly one member field and that field has
        // `proto3_optional = true`.
        for o in &mut oneofs {
            if o.field_indices.len() == 1 {
                let fidx = o.field_indices[0] as usize;
                if msg.field[fidx].proto3_optional == Some(true) {
                    o.synthetic = true;
                }
            }
        }

        // Negative bounds are spec-illegal; reject rather than letting the
        // `i32 → u32` reinterpretation roll over to a giant range.
        let mut extension_ranges: Vec<(u32, u32)> = Vec::with_capacity(msg.extension_range.len());
        for r in &msg.extension_range {
            let (Some(start), Some(end)) = (r.start, r.end) else {
                continue;
            };
            let (Ok(start), Ok(end)) = (u32::try_from(start), u32::try_from(end)) else {
                return Err(PoolError::InvalidFieldNumber {
                    field: format!("{fqn} (extension range)"),
                    number: start.min(end),
                });
            };
            extension_ranges.push((start, end));
        }

        // Replace the placeholder. Pass 1 registered messages depth-first
        // (this message before its nested messages), and pass 2 walks in the
        // same order, so `next_index` is exactly this message's slot.
        //
        // This invariant is load-bearing for cross-reference correctness —
        // a desync would silently write a `MessageDescriptor` into the wrong
        // slot, corrupting every cross-reference in the pool. Assert it in
        // release builds: a panic on bad input is preferable to a pool that
        // returns wrong descriptors at runtime.
        assert_eq!(
            self.messages[next_index].full_name, fqn,
            "pass-1/pass-2 walk order desync (this is a bug in DescriptorPool)"
        );
        self.messages[next_index] = MessageDescriptor {
            full_name: fqn.clone(),
            fields,
            field_by_number,
            field_by_name,
            oneofs,
            extension_ranges,
            options: clone_options(&msg.options),
        };
        let mut after = next_index + 1;

        // Recurse into nested messages. The nested message indices follow
        // this one in registration order.
        for nested in &msg.nested_type {
            after = self.link_message(&fqn, nested, &msg_features, after)?;
        }
        // Link nested enums.
        for nested_enum in &msg.enum_type {
            self.link_enum(&fqn, nested_enum, &msg_features)?;
        }
        Ok(after)
    }

    fn link_enum(
        &mut self,
        parent_fqn: &str,
        e: &EnumDescriptorProto,
        parent_features: &ResolvedFeatures,
    ) -> Result<(), PoolError> {
        let name = e.name.as_deref().unwrap_or("");
        let fqn = if parent_fqn.is_empty() {
            name.to_string()
        } else {
            format!("{parent_fqn}.{name}")
        };
        let enum_features = features::resolve_child(parent_features, features::enum_features(e));
        let idx = self.enum_index(&fqn).expect("enum registered in pass 1");
        let values: Vec<EnumValueDescriptor> = e
            .value
            .iter()
            .map(|v| EnumValueDescriptor {
                name: v.name.clone().unwrap_or_default(),
                number: v.number.unwrap_or(0),
                options: clone_options(&v.options),
            })
            .collect();
        self.enums[idx.0 as usize] = EnumDescriptor {
            full_name: fqn,
            values,
            enum_type: enum_features.enum_type,
            options: clone_options(&e.options),
        };
        Ok(())
    }

    fn link_service(
        &mut self,
        parent_fqn: &str,
        svc: &ServiceDescriptorProto,
    ) -> Result<(), PoolError> {
        let name = svc.name.as_deref().unwrap_or("");
        let fqn = if parent_fqn.is_empty() {
            name.to_string()
        } else {
            format!("{parent_fqn}.{name}")
        };
        if self.service_by_name.contains_key(&fqn) {
            return Err(PoolError::DuplicateName(fqn));
        }
        let methods = svc
            .method
            .iter()
            .map(|m| {
                let mname = m.name.as_deref().unwrap_or("");
                let method_fqn = format!("{fqn}.{mname}");
                let input = self.resolve_message_type_name(m.input_type.as_deref(), &method_fqn)?;
                let output =
                    self.resolve_message_type_name(m.output_type.as_deref(), &method_fqn)?;
                Ok(MethodDescriptor {
                    name: mname.to_string(),
                    input,
                    output,
                    client_streaming: m.client_streaming.unwrap_or(false),
                    server_streaming: m.server_streaming.unwrap_or(false),
                    options: clone_options(&m.options),
                })
            })
            .collect::<Result<Vec<_>, PoolError>>()?;
        let idx = ServiceIndex(
            u32::try_from(self.services.len()).expect("pool service count fits in u32"),
        );
        self.service_by_name.insert(fqn.clone(), idx);
        self.services.push(ServiceDescriptor {
            full_name: fqn,
            methods,
            options: clone_options(&svc.options),
        });
        Ok(())
    }

    /// Link one extension declaration scoped under `scope_fqn` (the package
    /// for file-level extensions, the declaring message's FQN for nested
    /// ones).
    fn link_extension(
        &mut self,
        scope_fqn: &str,
        ext: &FieldDescriptorProto,
        parent_features: &ResolvedFeatures,
    ) -> Result<(), PoolError> {
        let name = ext.name.as_deref().unwrap_or("");
        let fqn = if scope_fqn.is_empty() {
            name.to_string()
        } else {
            format!("{scope_fqn}.{name}")
        };
        // Protobuf has a single symbol space per scope: an extension cannot
        // share an FQN with another extension, a message, an enum, or a
        // service. A spec-compliant protoc enforces this; the input is no
        // longer trusted to come from protoc.
        if self.extension_by_name.contains_key(&fqn)
            || self.by_name.contains_key(&fqn)
            || self.service_by_name.contains_key(&fqn)
        {
            return Err(PoolError::DuplicateName(fqn));
        }
        let extendee = self.resolve_message_type_name(ext.extendee.as_deref(), &fqn)?;
        // The field links exactly like a declared field. `containing_msg` is
        // `None` because extensions cannot be map fields (a map requires a
        // synthetic MapEntry message nested in the declaring message, which
        // an `extend` block cannot contain).
        let mut field = self.link_field(scope_fqn, ext, parent_features, None)?;
        // Extensions cannot be oneof members. A malformed FieldDescriptorProto
        // carrying `oneof_index` would otherwise make `set()` clear the
        // *extendee's* declared oneof members (the index would be interpreted
        // against the extendee's oneof table). Scrub rather than reject —
        // there is exactly one valid interpretation of an extension's oneof
        // membership, and it is "none".
        field.oneof_index = None;
        // Validate the number falls inside one of the extendee's declared
        // extension ranges.
        if !self.messages[extendee.0 as usize].in_extension_range(field.number) {
            return Err(PoolError::InvalidFieldNumber {
                field: fqn,
                // `link_field` bounds the number to `MAX_FIELD_NUMBER`
                // (2^29 - 1), which fits `i32`; saturate defensively anyway.
                number: i32::try_from(field.number).unwrap_or(i32::MAX),
            });
        }
        // Two extensions claiming the same field number on the same message
        // is a conflict protoc rejects within one compilation unit but which
        // can arise when merging independently-compiled FileDescriptorSets.
        // Registering both would make one a phantom: resolvable by name but
        // never used by the wire or JSON codecs.
        if self
            .extension_by_extendee
            .contains_key(&(extendee, field.number))
        {
            return Err(PoolError::DuplicateExtensionNumber {
                extendee: self.messages[extendee.0 as usize].full_name.clone(),
                number: field.number,
            });
        }
        let json_key = format!("[{fqn}]");
        let idx = ExtensionIndex(
            u32::try_from(self.extensions.len()).expect("pool extension count fits in u32"),
        );
        self.extension_by_name.insert(fqn.clone(), idx);
        self.extension_by_extendee
            .insert((extendee, field.number), idx);
        self.extensions.push(ExtensionDescriptor {
            field,
            full_name: fqn,
            json_key,
            extendee,
        });
        Ok(())
    }

    /// Recursively link extensions declared inside `msg` and its nested
    /// messages. A nested extension's registration name is scoped under the
    /// declaring message: `pkg.Scope.ext_name`.
    fn link_nested_extensions(
        &mut self,
        parent_fqn: &str,
        msg: &DescriptorProto,
        parent_features: &ResolvedFeatures,
    ) -> Result<(), PoolError> {
        let name = msg.name.as_deref().unwrap_or("");
        let fqn = if parent_fqn.is_empty() {
            name.to_string()
        } else {
            format!("{parent_fqn}.{name}")
        };
        let msg_features =
            features::resolve_child(parent_features, features::message_features(msg));
        for ext in &msg.extension {
            self.link_extension(&fqn, ext, &msg_features)?;
        }
        for nested in &msg.nested_type {
            self.link_nested_extensions(&fqn, nested, &msg_features)?;
        }
        Ok(())
    }

    /// Resolve a method's `input_type`/`output_type` (a leading-dot FQN like
    /// `.my.pkg.Request`) to a [`MessageIndex`].
    fn resolve_message_type_name(
        &self,
        type_name: Option<&str>,
        method_fqn: &str,
    ) -> Result<MessageIndex, PoolError> {
        let tn = type_name.ok_or_else(|| PoolError::MissingTypeName {
            field: method_fqn.to_string(),
        })?;
        let lookup = tn.strip_prefix('.').unwrap_or(tn);
        match self.by_name.get(lookup) {
            Some(Definition::Message(midx)) => Ok(*midx),
            Some(Definition::Enum(_)) => Err(PoolError::WrongTypeKind {
                type_name: tn.to_string(),
                field: method_fqn.to_string(),
            }),
            None => Err(PoolError::UnresolvedTypeName {
                type_name: tn.to_string(),
                field: method_fqn.to_string(),
            }),
        }
    }

    fn link_field(
        &self,
        msg_fqn: &str,
        f: &FieldDescriptorProto,
        parent_features: &ResolvedFeatures,
        containing_msg: Option<&DescriptorProto>,
    ) -> Result<FieldDescriptor, PoolError> {
        let name = f.name.clone().unwrap_or_default();
        let field_fqn = format!("{msg_fqn}.{name}");
        let resolved = features::resolve_child(parent_features, features::field_features(f));

        let label = f.label.unwrap_or_default();
        let proto_ty = f.r#type.unwrap_or_default();
        let is_repeated = label == Label::LABEL_REPEATED;

        // Resolve the singular kind (element type).
        let element = self.resolve_singular(proto_ty, f.type_name.as_deref(), &field_fqn)?;

        // Note: enum closedness is *not* overlaid onto `FieldDescriptor`
        // (unlike `buffa-codegen::features::resolve_field`). The runtime
        // consumer reads `pool.enumeration(eidx).enum_type` directly when it
        // matters; the field descriptor only carries the index.

        // Detect map fields: repeated + message type + the message is a
        // map_entry. `containing_msg` is `None` for extensions, which cannot
        // be map fields — the lookup is skipped entirely.
        let kind = if is_repeated {
            if let SingularKind::Message(midx) = element {
                if let Some(entry) = containing_msg.and_then(|m| self.find_map_entry(m, f)) {
                    let (key_ty, value_kind) = self.resolve_map_entry(entry, &field_fqn)?;
                    // Map entry messages are synthetic — they're not real
                    // pool members for reflection purposes, but we leave
                    // them registered (consumers can ignore them).
                    let _ = midx;
                    FieldKind::Map {
                        key: key_ty,
                        value: value_kind,
                    }
                } else {
                    FieldKind::List(element)
                }
            } else {
                FieldKind::List(element)
            }
        } else {
            FieldKind::Singular(element)
        };

        // Resolve presence.
        let presence = if is_repeated {
            // Repeated/map fields have no presence.
            FieldPresence::Implicit
        } else if label == Label::LABEL_REQUIRED {
            FieldPresence::LegacyRequired
        } else if f.proto3_optional == Some(true) || f.oneof_index.is_some() {
            // proto3 `optional` and any oneof member always have explicit
            // presence regardless of edition features. A oneof field set
            // to its type's default value is still "present" — the oneof
            // discriminant carries that information on the wire.
            FieldPresence::Explicit
        } else if matches!(element, SingularKind::Message(_))
            && !matches!(kind, FieldKind::Map { .. })
        {
            // Singular message fields always have explicit presence (you can
            // distinguish absent from default).
            FieldPresence::Explicit
        } else {
            resolved.field_presence
        };

        // Resolve packed encoding.
        // Per the spec, only repeated scalar/enum fields are packable.
        let packable = matches!(
            kind,
            FieldKind::List(SingularKind::Scalar(s)) if !matches!(s, ScalarType::String | ScalarType::Bytes)
        ) || matches!(kind, FieldKind::List(SingularKind::Enum(_)));
        let packed = if packable {
            // An explicit [packed = ...] option wins over feature resolution.
            match f.options.as_option().and_then(|o| o.packed) {
                Some(p) => p,
                None => resolved.repeated_field_encoding == RepeatedFieldEncoding::Packed,
            }
        } else {
            false
        };

        // Resolve delimited (group) encoding.
        // proto2/proto3: TYPE_GROUP is delimited; TYPE_MESSAGE is length-prefixed.
        // editions: message_encoding feature controls it.
        let delimited = if proto_ty == ProtoType::TYPE_GROUP {
            true
        } else if matches!(element, SingularKind::Message(_)) {
            resolved.message_encoding == MessageEncoding::Delimited
        } else {
            false
        };

        let oneof_index = f.oneof_index.and_then(|i| u16::try_from(i).ok());

        let json_name = f
            .json_name
            .clone()
            .unwrap_or_else(|| derive_json_name(&name));

        // Validate the field number. The wire format reserves 0; the upper
        // bound is `(1 << 29) - 1`. Spec-compliant `protoc` never emits an
        // out-of-range number, but the input is no longer trusted to come
        // from `protoc` once consumers feed network-loaded descriptors.
        let raw_number = f.number.unwrap_or(0);
        let number = u32::try_from(raw_number)
            .ok()
            .filter(|&n| (1..=buffa::encoding::MAX_FIELD_NUMBER).contains(&n))
            .ok_or(PoolError::InvalidFieldNumber {
                field: field_fqn,
                number: raw_number,
            })?;

        Ok(FieldDescriptor {
            name,
            json_name,
            number,
            kind,
            presence,
            packed,
            delimited,
            oneof_index,
            options: clone_options(&f.options),
        })
    }

    fn resolve_singular(
        &self,
        ty: ProtoType,
        type_name: Option<&str>,
        field_fqn: &str,
    ) -> Result<SingularKind, PoolError> {
        if let Some(scalar) = ScalarType::from_proto(ty) {
            return Ok(SingularKind::Scalar(scalar));
        }
        // ENUM, MESSAGE, GROUP — resolve type_name.
        let tn = type_name.ok_or_else(|| PoolError::MissingTypeName {
            field: field_fqn.to_string(),
        })?;
        let lookup = tn.strip_prefix('.').unwrap_or(tn);
        match self.by_name.get(lookup) {
            Some(Definition::Message(midx))
                if matches!(ty, ProtoType::TYPE_MESSAGE | ProtoType::TYPE_GROUP) =>
            {
                Ok(SingularKind::Message(*midx))
            }
            Some(Definition::Enum(eidx)) if ty == ProtoType::TYPE_ENUM => {
                Ok(SingularKind::Enum(*eidx))
            }
            Some(_) => Err(PoolError::WrongTypeKind {
                type_name: tn.to_string(),
                field: field_fqn.to_string(),
            }),
            None => Err(PoolError::UnresolvedTypeName {
                type_name: tn.to_string(),
                field: field_fqn.to_string(),
            }),
        }
    }

    /// Find the nested map-entry message for a repeated message field.
    fn find_map_entry<'a>(
        &self,
        containing: &'a DescriptorProto,
        f: &FieldDescriptorProto,
    ) -> Option<&'a DescriptorProto> {
        if f.label.unwrap_or_default() != Label::LABEL_REPEATED {
            return None;
        }
        if f.r#type.unwrap_or_default() != ProtoType::TYPE_MESSAGE {
            return None;
        }
        let tn = f.type_name.as_deref()?;
        // Map entry messages are nested inside the containing message and
        // have name `<FieldName>Entry`. The type_name's last segment is the
        // entry message name.
        let entry_name = tn.rsplit('.').next()?;
        let entry = containing
            .nested_type
            .iter()
            .find(|n| n.name.as_deref() == Some(entry_name))?;
        if entry.options.as_option().and_then(|o| o.map_entry) == Some(true) {
            Some(entry)
        } else {
            None
        }
    }

    fn resolve_map_entry(
        &self,
        entry: &DescriptorProto,
        field_fqn: &str,
    ) -> Result<(ScalarType, SingularKind), PoolError> {
        let key_fd = entry.field.iter().find(|f| f.number == Some(1));
        let val_fd = entry.field.iter().find(|f| f.number == Some(2));
        let (Some(kf), Some(vf)) = (key_fd, val_fd) else {
            return Err(PoolError::MalformedMapEntry {
                message: field_fqn.to_string(),
            });
        };
        let key_ty = ScalarType::from_proto(kf.r#type.unwrap_or_default()).ok_or_else(|| {
            PoolError::MalformedMapEntry {
                message: field_fqn.to_string(),
            }
        })?;
        if !key_ty.is_valid_map_key() {
            return Err(PoolError::MalformedMapEntry {
                message: field_fqn.to_string(),
            });
        }
        let value_kind = self.resolve_singular(
            vf.r#type.unwrap_or_default(),
            vf.type_name.as_deref(),
            field_fqn,
        )?;
        Ok((key_ty, value_kind))
    }
}

/// Derive the default JSON name for a proto field name (lowerCamelCase).
fn derive_json_name(proto_name: &str) -> String {
    let mut out = String::with_capacity(proto_name.len());
    let mut capitalize = false;
    for c in proto_name.chars() {
        if c == '_' {
            capitalize = true;
        } else if capitalize {
            out.extend(c.to_uppercase());
            capitalize = false;
        } else {
            out.push(c);
        }
    }
    out
}

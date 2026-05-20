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

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::desc::{
    EnumDescriptor, EnumIndex, EnumValueDescriptor, FieldDescriptor, FieldKind, MessageDescriptor,
    MessageIndex, MethodDescriptor, OneofDescriptor, ScalarType, ServiceDescriptor, ServiceIndex,
    SingularKind,
};
use crate::features::{self, ResolvedFeatures};
use crate::generated::descriptor::field_descriptor_proto::{Label, Type as ProtoType};
use crate::generated::descriptor::{
    DescriptorProto, EnumDescriptorProto, FieldDescriptorProto, FileDescriptorProto,
    FileDescriptorSet, ServiceDescriptorProto,
};
use buffa::editions::{EnumType, FieldPresence, MessageEncoding, RepeatedFieldEncoding};

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
    /// limit documented on [`MessageDescriptor::field_by_number`].
    TooManyFields { message: String, count: usize },
    /// A field number is outside the valid range
    /// `[1, MAX_FIELD_NUMBER]` (`(1 << 29) - 1`), or an extension range has
    /// a negative bound.
    InvalidFieldNumber { field: String, number: i32 },
    /// A map entry message did not have exactly fields 1 (key) and 2 (value),
    /// or the key type is not a valid map key per the protobuf spec.
    MalformedMapEntry { message: String },
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
    /// FQN (no leading dot) → definition lookup.
    by_name: BTreeMap<String, Definition>,
    /// Service FQN (no leading dot) → index. Separate from `by_name`
    /// because `Definition` is `MessageIndex`-or-`EnumIndex` and services
    /// are linked in a single pass after types resolve.
    service_by_name: BTreeMap<String, ServiceIndex>,
    /// Filename → index into `files`.
    file_by_name: BTreeMap<String, usize>,
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
                    .is_none_or(|n| !self.file_by_name.contains_key(n))
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

        // Pass 3: link services. Services reference message types by name
        // for their input/output, so they link after the type passes. There's
        // no register/link split because services have no forward references
        // to each other and carry no editions features that affect resolution.
        for file in &new_files {
            let pkg = file.package.as_deref().unwrap_or("");
            for svc in &file.service {
                self.link_service(pkg, svc)?;
            }
        }

        // Record filenames for idempotent re-add.
        let base = self.files.len();
        for (i, f) in new_files.iter().enumerate() {
            if let Some(n) = f.name.as_deref() {
                self.file_by_name.insert(n.to_string(), base + i);
            }
        }
        self.files.extend(new_files);

        Ok(())
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

    /// The `FileDescriptorProto` containing a fully-qualified symbol.
    #[must_use]
    pub fn file_containing_symbol(&self, full_name: &str) -> Option<&FileDescriptorProto> {
        let name = full_name.strip_prefix('.').unwrap_or(full_name);
        // Walk up parent scopes until we find a registered name.
        let mut cur = name;
        loop {
            if self.by_name.contains_key(cur) {
                // Find the file declaring this top-level type.
                return self.files.iter().find(|f| {
                    let pkg = f.package.as_deref().unwrap_or("");
                    let prefix = if pkg.is_empty() {
                        String::new()
                    } else {
                        format!("{pkg}.")
                    };
                    if !cur.starts_with(&prefix) {
                        return false;
                    }
                    let local = &cur[prefix.len()..];
                    let top = local.split('.').next().unwrap_or(local);
                    f.message_type
                        .iter()
                        .any(|m| m.name.as_deref() == Some(top))
                        || f.enum_type.iter().any(|e| e.name.as_deref() == Some(top))
                });
            }
            match cur.rfind('.') {
                Some(p) => cur = &cur[..p],
                None => return None,
            }
        }
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
            })
            .collect();

        // Build field descriptors.
        let mut fields = Vec::with_capacity(field_count);
        let mut field_by_number: Vec<(u32, u16)> = Vec::with_capacity(field_count);
        let mut field_by_name: Vec<(String, u16)> = Vec::with_capacity(field_count * 2);
        for (i, f) in msg.field.iter().enumerate() {
            let fd = self.link_field(&fqn, f, &msg_features, msg)?;
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
            })
            .collect();
        self.enums[idx.0 as usize] = EnumDescriptor {
            full_name: fqn,
            values,
            enum_type: enum_features.enum_type,
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
        });
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
        containing_msg: &DescriptorProto,
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
        // map_entry.
        let kind = if is_repeated {
            if let SingularKind::Message(midx) = element {
                if let Some(entry) = self.find_map_entry(containing_msg, f) {
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

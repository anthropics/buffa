//! Convert typed descriptor options into the same validated Rust configuration.

use buffa::ExtensionSet;
use buffa_proto_options as pb;

use crate::generated::descriptor::FileDescriptorProto;

use super::*;

// Unknown metadata fails closed: accepting a future ownership or projection
// rule while ignoring its new fields would silently change the native API.
macro_rules! known {
    ($value:expr) => {
        if !$value.__buffa_unknown_fields.is_empty() {
            return Err(ErrorReason::InvalidManifest.into());
        }
    };
}

/// Combine the `(buffa.ext.file).adapters` options of the requested files.
///
/// Returns `None` when no requested file declares them, so the caller can
/// decide whether an explicit configuration makes them optional.
pub(crate) fn descriptor_options(
    files: &[FileDescriptorProto],
    requested: &[String],
) -> Result<Option<AdapterConfig>, AdapterError> {
    let mut combined = AdapterConfig {
        version: 1,
        selectors: Vec::new(),
        payloads: Vec::new(),
        borrowed: Vec::new(),
        ports: Vec::new(),
    };
    let mut found = false;
    for file in files
        .iter()
        .filter(|f| f.name.as_ref().is_some_and(|n| requested.contains(n)))
    {
        let Some(options) = file.options.as_option() else {
            continue;
        };
        if options.unknown_fields().iter().any(|field| {
            field.number == pb::FILE.number()
                && !matches!(field.data, buffa::UnknownFieldData::LengthDelimited(_))
        }) {
            return Err(ErrorReason::InvalidManifest.into());
        }
        if !options.has_extension(&pb::FILE) {
            continue;
        }
        // The extension is statically paired with FileOptions; no dynamic
        // extendee is accepted. A present but undecodable value is an error.
        let options = options
            .extension(&pb::FILE)
            .ok_or(ErrorReason::InvalidManifest)?;
        known!(options);
        let Some(native) = options.adapters.as_option() else {
            continue;
        };
        let config = AdapterConfig::try_from(native)?;
        found = true;
        combined.selectors.extend(config.selectors);
        combined.payloads.extend(config.payloads);
        combined.borrowed.extend(config.borrowed);
        combined.ports.extend(config.ports);
    }
    Ok(found.then_some(combined))
}

impl TryFrom<&pb::NativeAdapters> for AdapterConfig {
    type Error = AdapterError;

    fn try_from(value: &pb::NativeAdapters) -> Result<Self, Self::Error> {
        known!(value);
        if value.version != 1 {
            return Err(ErrorReason::UnsupportedVersion.into());
        }
        Ok(Self {
            version: value.version,
            selectors: value
                .selectors
                .iter()
                .map(ClosedSelector::try_from)
                .collect::<Result<_, _>>()?,
            payloads: value
                .payloads
                .iter()
                .map(PayloadSelector::try_from)
                .collect::<Result<_, _>>()?,
            borrowed: value
                .borrowed
                .iter()
                .map(BorrowedAdapter::try_from)
                .collect::<Result<_, _>>()?,
            ports: value
                .ports
                .iter()
                .map(CapabilityPort::try_from)
                .collect::<Result<_, _>>()?,
        })
    }
}

impl TryFrom<&pb::NativeWireEnum> for WireEnum {
    type Error = AdapterError;
    fn try_from(v: &pb::NativeWireEnum) -> Result<Self, Self::Error> {
        known!(v);
        Ok(Self {
            source: v.source.clone(),
            rust_type: v.rust_type.clone(),
        })
    }
}

impl TryFrom<&pb::NativeErrorMapping> for ErrorMapping {
    type Error = AdapterError;
    fn try_from(v: &pb::NativeErrorMapping) -> Result<Self, Self::Error> {
        known!(v);
        Ok(Self {
            rust_type: v.rust_type.clone(),
            value: v.value.clone(),
        })
    }
}

impl TryFrom<&pb::NativeNormalization> for Normalization {
    type Error = AdapterError;
    fn try_from(v: &pb::NativeNormalization) -> Result<Self, Self::Error> {
        known!(v);
        Ok(Self {
            source: v.source.clone(),
            target: v.target.clone(),
        })
    }
}

impl TryFrom<&pb::NativeSelectorVariant> for SelectorVariant {
    type Error = AdapterError;
    fn try_from(v: &pb::NativeSelectorVariant) -> Result<Self, Self::Error> {
        known!(v);
        Ok(Self {
            source: v.source.clone(),
            name: v.name.clone(),
            aliases: v.aliases.clone(),
            identifier: v.identifier,
        })
    }
}

impl TryFrom<&pb::NativeLegacySelector> for LegacySelector {
    type Error = AdapterError;
    fn try_from(v: &pb::NativeLegacySelector) -> Result<Self, Self::Error> {
        known!(v);
        Ok(Self {
            wire: WireEnum::try_from(v.wire.as_option().ok_or(ErrorReason::InvalidManifest)?)?,
            normalizations: v
                .normalizations
                .iter()
                .map(Normalization::try_from)
                .collect::<Result<_, _>>()?,
            rejected: v.rejected.clone(),
        })
    }
}

impl TryFrom<&pb::NativeClosedSelector> for ClosedSelector {
    type Error = AdapterError;
    fn try_from(v: &pb::NativeClosedSelector) -> Result<Self, Self::Error> {
        known!(v);
        Ok(Self {
            file: v.file.clone(),
            name: v.name.clone(),
            wire: WireEnum::try_from(v.wire.as_option().ok_or(ErrorReason::InvalidManifest)?)?,
            error: ErrorMapping::try_from(
                v.error.as_option().ok_or(ErrorReason::InvalidManifest)?,
            )?,
            non_exhaustive: v.non_exhaustive.unwrap_or(false),
            variants: v
                .variants
                .iter()
                .map(SelectorVariant::try_from)
                .collect::<Result<_, _>>()?,
            normalizations: v
                .normalizations
                .iter()
                .map(Normalization::try_from)
                .collect::<Result<_, _>>()?,
            rejected: v.rejected.clone(),
            legacy: v
                .legacy
                .iter()
                .map(LegacySelector::try_from)
                .collect::<Result<_, _>>()?,
            identifier_error: v
                .identifier_error
                .as_option()
                .map(ErrorMapping::try_from)
                .transpose()?,
        })
    }
}

impl TryFrom<&pb::NativeFieldSource> for FieldSource {
    type Error = AdapterError;
    fn try_from(v: &pb::NativeFieldSource) -> Result<Self, Self::Error> {
        known!(v);
        Ok(Self {
            message: v.message.clone(),
            path: v.path.clone(),
        })
    }
}

impl TryFrom<&pb::NativePayloadType> for PayloadType {
    type Error = AdapterError;
    fn try_from(v: &pb::NativePayloadType) -> Result<Self, Self::Error> {
        known!(v);
        match v.kind {
            pb::NativePayloadTypeKind::NATIVE_PAYLOAD_TYPE_BYTES
                if v.source.is_none() && v.rust_type.is_none() =>
            {
                Ok(Self::Bytes)
            }
            pb::NativePayloadTypeKind::NATIVE_PAYLOAD_TYPE_VALUE => Ok(Self::Value {
                source: v.source.clone().ok_or(ErrorReason::InvalidManifest)?,
                rust_type: v.rust_type.clone().ok_or(ErrorReason::InvalidManifest)?,
            }),
            _ => Err(ErrorReason::InvalidPayload.into()),
        }
    }
}

impl TryFrom<&pb::NativePayloadField> for PayloadField {
    type Error = AdapterError;
    fn try_from(v: &pb::NativePayloadField) -> Result<Self, Self::Error> {
        known!(v);
        Ok(Self {
            name: v.name.clone(),
            source: FieldSource::try_from(
                v.source.as_option().ok_or(ErrorReason::InvalidManifest)?,
            )?,
            representation: PayloadType::try_from(
                v.representation
                    .as_option()
                    .ok_or(ErrorReason::InvalidManifest)?,
            )?,
        })
    }
}

impl TryFrom<&pb::NativePayload> for Payload {
    type Error = AdapterError;
    fn try_from(v: &pb::NativePayload) -> Result<Self, Self::Error> {
        known!(v);
        match v.kind {
            pb::NativePayloadKind::NATIVE_PAYLOAD_UNIT
                if v.fields.is_empty() && v.port.is_none() =>
            {
                Ok(Self::Unit)
            }
            pb::NativePayloadKind::NATIVE_PAYLOAD_TUPLE
                if v.fields.len() == 1 && v.port.is_none() =>
            {
                let field = v.fields.first().ok_or(ErrorReason::InvalidPayload)?;
                Ok(Self::Tuple {
                    field: PayloadField::try_from(field)?,
                })
            }
            pb::NativePayloadKind::NATIVE_PAYLOAD_STRUCT
                if !v.fields.is_empty() && v.port.is_none() =>
            {
                Ok(Self::Struct {
                    fields: v
                        .fields
                        .iter()
                        .map(PayloadField::try_from)
                        .collect::<Result<_, _>>()?,
                })
            }
            pb::NativePayloadKind::NATIVE_PAYLOAD_PORT if v.fields.is_empty() => Ok(Self::Port {
                name: v.port.clone().ok_or(ErrorReason::InvalidPayload)?,
            }),
            _ => Err(ErrorReason::InvalidPayload.into()),
        }
    }
}

impl TryFrom<&pb::NativePayloadVariant> for PayloadVariant {
    type Error = AdapterError;
    fn try_from(v: &pb::NativePayloadVariant) -> Result<Self, Self::Error> {
        known!(v);
        Ok(Self {
            name: v.name.clone(),
            cases: v.cases.clone(),
            payload: Payload::try_from(v.payload.as_option().ok_or(ErrorReason::InvalidManifest)?)?,
        })
    }
}

impl TryFrom<&pb::NativePayloadSelector> for PayloadSelector {
    type Error = AdapterError;
    fn try_from(v: &pb::NativePayloadSelector) -> Result<Self, Self::Error> {
        known!(v);
        Ok(Self {
            file: v.file.clone(),
            name: v.name.clone(),
            source: v.source.clone(),
            variants: v
                .variants
                .iter()
                .map(PayloadVariant::try_from)
                .collect::<Result<_, _>>()?,
        })
    }
}

impl TryFrom<&pb::NativeBorrowedSource> for BorrowedSource {
    type Error = AdapterError;
    fn try_from(v: &pb::NativeBorrowedSource) -> Result<Self, Self::Error> {
        known!(v);
        match v.kind {
            pb::NativeBorrowedSourceKind::NATIVE_BORROWED_SOURCE_ENUM
                if v.message.is_none() && v.name.is_none() =>
            {
                Ok(Self::Enum {
                    source: v.source.clone().ok_or(ErrorReason::InvalidManifest)?,
                })
            }
            pb::NativeBorrowedSourceKind::NATIVE_BORROWED_SOURCE_ONEOF if v.source.is_none() => {
                Ok(Self::Oneof {
                    message: v.message.clone().ok_or(ErrorReason::InvalidManifest)?,
                    name: v.name.clone().ok_or(ErrorReason::InvalidManifest)?,
                })
            }
            _ => Err(ErrorReason::InvalidPayload.into()),
        }
    }
}

impl TryFrom<&pb::NativeBorrowedAdapter> for BorrowedAdapter {
    type Error = AdapterError;
    fn try_from(v: &pb::NativeBorrowedAdapter) -> Result<Self, Self::Error> {
        known!(v);
        Ok(Self {
            file: v.file.clone(),
            name: v.name.clone(),
            source: BorrowedSource::try_from(
                v.source.as_option().ok_or(ErrorReason::InvalidManifest)?,
            )?,
            variants: v
                .variants
                .iter()
                .map(PayloadVariant::try_from)
                .collect::<Result<_, _>>()?,
        })
    }
}

impl TryFrom<&pb::NativeCapabilityPort> for CapabilityPort {
    type Error = AdapterError;
    fn try_from(v: &pb::NativeCapabilityPort) -> Result<Self, Self::Error> {
        known!(v);
        Ok(Self {
            name: v.name.clone(),
            rust_trait: v.rust_trait.clone(),
        })
    }
}

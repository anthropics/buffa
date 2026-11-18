pub mod domain {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum ErrorReason {
        Invalid,
    }
}

pub mod r#gen {
    pub use super::{
        alias_wire::{AliasedWire, LegacyWire},
        domain::ErrorReason,
    };
}

#[cfg(test)]
mod alias_tests {
    use super::{alias_wire::*, domain::ErrorReason, native::*};
    use buffa::{EnumValue, Enumeration};

    #[test]
    fn primitive_names_cannot_shadow_signature_types() {
        type Scalar = primitive_i64::i64;
        let identifier: fn(Scalar) -> ::core::primitive::i64 = Scalar::identifier;
        let from_identifier: fn(::core::primitive::i64) -> Result<Scalar, super::ContractError> =
            Scalar::from_identifier;
        assert_eq!(identifier(Scalar::Type), 1);
        assert_eq!(from_identifier(2), Ok(Scalar::Current));
        assert_eq!(from_identifier(99), Err(super::ContractError::Invalid));
        let bytes = [1_u8, 2, 3];
        let primitive_u8::u8::DeviceDetached(borrowed) = primitive_u8::u8::DeviceDetached(&bytes)
        else {
            panic!("expected borrowed bytes");
        };
        assert!(core::ptr::eq(bytes.as_ptr(), borrowed.as_ptr()));
        assert_eq!(borrowed, &bytes);
    }

    #[test]
    fn error_named_variants_convert_from_canonical_and_legacy_wire_types() {
        for wire in [AliasedWire::C_ALIAS_ERROR, AliasedWire::C_ALIAS_FAILURE] {
            assert_eq!(
                AliasedSelector::try_from(EnumValue::Known(wire)),
                Ok(AliasedSelector::Error)
            );
        }
        for wire in [LegacyWire::L_ALIAS_ERROR, LegacyWire::L_ALIAS_FAILURE] {
            assert_eq!(
                AliasedSelector::try_from(EnumValue::Known(wire)),
                Ok(AliasedSelector::Error)
            );
        }
        assert_eq!(
            AliasedSelector::try_from(EnumValue::Known(LegacyWire::L_ALIAS_READY)),
            Ok(AliasedSelector::Ready)
        );
        for &value in AliasedSelector::ALL {
            let wire = AliasedWire::from(value);
            assert_eq!(AliasedSelector::try_from(EnumValue::Known(wire)), Ok(value));
            assert_eq!(
                AliasedSelector::try_from(EnumValue::<AliasedWire>::Unknown(wire.to_i32())),
                Err(ErrorReason::Invalid)
            );
        }
        assert_eq!(
            AliasedWire::from(AliasedSelector::Error),
            AliasedWire::C_ALIAS_ERROR
        );
    }

    #[test]
    fn every_zero_alias_and_unknown_value_is_rejected() {
        for wire in [AliasedWire::C_ALIAS_UNSPECIFIED, AliasedWire::C_ALIAS_NONE] {
            assert_eq!(
                AliasedSelector::try_from(EnumValue::Known(wire)),
                Err(ErrorReason::Invalid)
            );
        }
        for wire in [LegacyWire::L_ALIAS_UNSPECIFIED, LegacyWire::L_ALIAS_NONE] {
            assert_eq!(
                AliasedSelector::try_from(EnumValue::Known(wire)),
                Err(ErrorReason::Invalid)
            );
        }
        for number in [0, 1, 11, -1, i32::MIN, i32::MAX] {
            assert_eq!(
                AliasedSelector::try_from(EnumValue::<AliasedWire>::Unknown(number)),
                Err(ErrorReason::Invalid)
            );
            assert_eq!(
                AliasedSelector::try_from(EnumValue::<LegacyWire>::Unknown(number)),
                Err(ErrorReason::Invalid)
            );
        }
    }

    #[test]
    fn payloads_stay_copy_and_borrowed_payloads_retain_caller_storage() {
        let payload = AliasedPayload::Digest(AliasedSelector::Error);
        fn copy<T: Copy>(value: T) -> T {
            value
        }
        assert_eq!(payload, copy(payload));
        let source = [1_u8, 2, 3];
        for value in [
            AliasedBorrow::Message(&source),
            AliasedBorrow::Digest(&source),
        ] {
            let (AliasedBorrow::Message(bytes) | AliasedBorrow::Digest(bytes)) = value;
            assert!(core::ptr::eq(bytes.as_ptr(), source.as_ptr()));
            assert_eq!(bytes, &source);
        }
    }
}

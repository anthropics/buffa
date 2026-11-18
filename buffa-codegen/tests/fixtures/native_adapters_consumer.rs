#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractError {
    Invalid,
    Unsupported,
}

pub mod ports {
    pub trait VerificationKey {
        fn marker(&self) -> u8;
    }
    pub trait CertificateValidator {
        fn marker(&self) -> u8;
    }
}

/// Borrowed adapters cannot escape their caller's buffer lifetime.
///
/// ```compile_fail
/// use native_adapter_compatibility::native::SigningInput;
/// let borrowed = {
///     let payload = [1_u8, 2];
///     SigningInput::Message(&payload)
/// };
/// core::hint::black_box(borrowed);
/// ```
///
/// Borrowed buffers do not gain a diagnostic implementation.
///
/// ```compile_fail
/// use native_adapter_compatibility::native::ContainerContent;
/// let _ = format!("{:?}", ContainerContent::Detached(&[1, 2]));
/// ```
pub struct LifetimeContract;

#[cfg(test)]
mod tests {
    use super::{native::*, *};
    use buffa::{EnumValue, Enumeration};

    fn round_trip<N, W>(all: &[N])
    where
        N: Copy + Eq + core::fmt::Debug + TryFrom<EnumValue<W>, Error = ContractError>,
        W: Enumeration + From<N>,
    {
        for &native in all {
            assert_eq!(N::try_from(EnumValue::from(W::from(native))), Ok(native));
            let number = W::from(native).to_i32();
            assert_eq!(
                N::try_from(EnumValue::Unknown(number)),
                Err(ContractError::Invalid)
            );
        }
        for invalid in [0, -1, i32::MIN, i32::MAX] {
            assert_eq!(
                N::try_from(EnumValue::<W>::from(invalid)),
                Err(ContractError::Invalid)
            );
        }
    }

    #[test]
    fn external_registrations_and_legacy_aliases_remain_distinct() {
        assert_eq!(
            wire::KeywordWire::from(KeywordNative::Type),
            wire::KeywordWire::r#type
        );
        assert_eq!(
            wire::KeywordWire::from(KeywordNative::Current),
            wire::KeywordWire::self_
        );
        assert_eq!(CoseSignatureAlgorithm::Es256.identifier(), -7);
        assert_eq!(CoseSignatureAlgorithm::Esp256.identifier(), -9);
        assert_eq!(CoseSignatureAlgorithm::EdDsa.identifier(), -8);
        assert_eq!(CoseSignatureAlgorithm::Ed25519.identifier(), -19);
        assert_eq!(
            CoseSignatureAlgorithm::try_from(EnumValue::<wire::CoseSignatureAlgorithm>::from(200)),
            Ok(CoseSignatureAlgorithm::Esp256)
        );
        assert_eq!(
            wire::CoseSignatureAlgorithm::from(CoseSignatureAlgorithm::Esp256).to_i32(),
            202
        );
        assert_eq!(
            CoseSignatureAlgorithm::from_identifier(200),
            Err(ContractError::Unsupported)
        );
        assert_eq!(
            CosePointEncoding::FullCoordinates,
            CosePointEncoding::Ec2Coordinates
        );
        assert_eq!(CosePointEncoding::Compressed, CosePointEncoding::Ec2YSign);
        assert_eq!(
            CosePointEncoding::try_from(EnumValue::<wire::CoseEc2PointEncoding>::from(1)),
            Ok(CosePointEncoding::Ec2YSign)
        );
        assert_eq!(
            CosePointEncoding::try_from(EnumValue::<wire::CoseEc2PointEncoding>::from(0)),
            Err(ContractError::Invalid)
        );
        assert_eq!(
            CosePointEncoding::try_from(EnumValue::<wire::CoseEc2PointEncoding>::Unknown(1)),
            Err(ContractError::Invalid)
        );
        assert_eq!(
            CoseMlKemAlgorithm::try_from(EnumValue::<wire::CoseKemAlgorithm>::from(1100)),
            Err(ContractError::Invalid)
        );
        let json =
            serde_json::to_string(&wire::CoseSignatureAlgorithm::COSE_SIGNATURE_ALGORITHM_ES256)
                .unwrap();
        assert_eq!(json, "\"COSE_SIGNATURE_ALGORITHM_ES256\"");
    }

    #[test]
    fn copy_payload_selectors_keep_their_public_shapes() {
        let suite = SignatureSuite {
            registration: CoseSignatureAlgorithm::Es256,
            curve: CoseCurve::P256,
        };
        assert_eq!(
            PeerKeyUsage::Signature(suite),
            PeerKeyUsage::Signature(suite)
        );
        assert!(matches!(PeerKeyUsage::Agreement, PeerKeyUsage::Agreement));
        assert!(matches!(
            PeerKeyUsage::HpkeRecipient,
            PeerKeyUsage::HpkeRecipient
        ));
        assert!(matches!(SigningMode::Message, SigningMode::Message));
        let mode = SigningMode::EcdsaPrehash(Iso18013EcdsaHash::Sha256);
        assert_eq!(mode, SigningMode::EcdsaPrehash(Iso18013EcdsaHash::Sha256));
    }

    #[test]
    fn every_borrowed_content_alternative_retains_the_callers_slice() {
        let bytes = [11_u8, 22, 33];
        let ptr = bytes.as_ptr();
        assert!(matches!(
            Iso18013Sign1Content::IssuerAttached,
            Iso18013Sign1Content::IssuerAttached
        ));
        for content in [
            Iso18013Sign1Content::DeviceDetached(&bytes),
            Iso18013Sign1Content::ReaderDetached(&bytes),
        ] {
            let (Iso18013Sign1Content::DeviceDetached(value)
            | Iso18013Sign1Content::ReaderDetached(value)) = content
            else {
                panic!()
            };
            assert_eq!(value.as_ptr(), ptr);
        }
        assert!(matches!(
            ContainerContent::Attached,
            ContainerContent::Attached
        ));
        let ContainerContent::Detached(value) = ContainerContent::Detached(&bytes) else {
            panic!()
        };
        assert_eq!(value.as_ptr(), ptr);
        for content in [
            SigningContent::Attached(&bytes),
            SigningContent::Detached(&bytes),
        ] {
            let (SigningContent::Attached(value) | SigningContent::Detached(value)) = content;
            assert_eq!(value.as_ptr(), ptr);
        }
        let SigningInput::Message(value) = SigningInput::Message(&bytes) else {
            panic!()
        };
        assert_eq!(value.as_ptr(), ptr);
        let SigningInput::EcdsaPrehash { hash, digest } = (SigningInput::EcdsaPrehash {
            hash: Iso18013EcdsaHash::Sha256,
            digest: &bytes,
        }) else {
            panic!()
        };
        assert_eq!(hash, Iso18013EcdsaHash::Sha256);
        assert_eq!(digest.as_ptr(), ptr);
        assert_eq!(bytes, [11, 22, 33]);
    }

    #[test]
    fn capability_adapters_borrow_without_taking_ownership() {
        struct Provider<'a> {
            dropped: &'a core::cell::Cell<u8>,
            _state: [u8; 16],
        }
        impl ports::VerificationKey for Provider<'_> {
            fn marker(&self) -> u8 {
                7
            }
        }
        impl ports::CertificateValidator for Provider<'_> {
            fn marker(&self) -> u8 {
                9
            }
        }
        impl Drop for Provider<'_> {
            fn drop(&mut self) {
                self.dropped.set(1);
            }
        }
        let dropped = core::cell::Cell::new(0);
        {
            let provider = Provider {
                dropped: &dropped,
                _state: [0xA5; 16],
            };
            {
                let VerificationSource::Key(key) = VerificationSource::Key(&provider) else {
                    panic!()
                };
                assert_eq!(key.marker(), 7);
                let VerificationSource::Certificates(certificates) =
                    VerificationSource::Certificates(&provider)
                else {
                    panic!()
                };
                assert_eq!(certificates.marker(), 9);
            }
            assert_eq!(dropped.get(), 0);
        }
        assert_eq!(dropped.get(), 1);
    }

    // Canonical selector coverage is appended by the compatibility harness.
}

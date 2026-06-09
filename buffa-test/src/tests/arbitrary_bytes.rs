// Regression #88: bytes_fields + generate_arbitrary(true).
//
// BytesContexts has all four bytes field shapes:
//   singular (Bytes), optional (Option<Bytes>), repeated (Vec<Bytes>),
//   oneof variant (Choice::Raw(Bytes)).
// Compilation of basic_arbitrary_bytes (in lib.rs) is the primary assertion.
// The tests below verify runtime correctness when --features arbitrary is on.

#[cfg(feature = "arbitrary")]
mod tests {
    use crate::basic_arbitrary_bytes::BytesContexts;
    use arbitrary::{Arbitrary, Unstructured};

    /// Type-compatibility smoke test: `derive(Arbitrary)` on `BytesContexts`
    /// must succeed, and every bytes-shaped field must be a real
    /// `bytes::Bytes` (`slice(..)` is `Bytes`-specific). The seed pattern is
    /// deliberately varied — content non-emptiness is asserted at the helper
    /// level in `buffa::arbitrary_tests` (helpers are unit-tested in
    /// isolation; this test pins the codegen wiring).
    #[test]
    fn bytes_contexts_arbitrary_all_shapes() {
        let raw: [u8; 256] = core::array::from_fn(|i| i as u8);
        let mut u = Unstructured::new(&raw);
        let msg = BytesContexts::arbitrary(&mut u).unwrap();
        let _ = msg.singular.slice(..);
        if let Some(ref b) = msg.maybe {
            let _ = b.slice(..);
        }
        for b in &msg.many {
            let _ = b.slice(..);
        }
        if let Some(ref choice) = msg.choice {
            use crate::basic_arbitrary_bytes::__buffa::oneof::bytes_contexts::Choice;
            if let Choice::Raw(b) = choice {
                let _ = b.slice(..);
            }
        }
        // `map<string, bytes>` values are `Bytes` too under the shim — `slice(..)`
        // is `Bytes`-specific, so this pins that the value type isn't `Vec<u8>`.
        for b in msg.by_key.values() {
            let _ = b.slice(..);
        }
    }
}

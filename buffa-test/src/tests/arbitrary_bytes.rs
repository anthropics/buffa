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

    #[test]
    fn bytes_contexts_arbitrary_all_shapes() {
        let raw = [0u8; 256];
        let mut u = Unstructured::new(&raw);
        let msg = BytesContexts::arbitrary(&mut u).unwrap();
        // Exercise each bytes-shaped field to confirm the types are real Bytes.
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
    }
}

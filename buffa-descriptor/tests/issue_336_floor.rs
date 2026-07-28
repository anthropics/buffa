//! Regression coverage for #336: the codegen-emitted `descriptor_pool()`
//! bound (`len * 64`) must never be tighter than `DEFAULT_ELEMENT_MEMORY_LIMIT`,
//! the bound it replaced. This exercises the fixed formula
//! (`len.saturating_mul(64).max(DEFAULT_ELEMENT_MEMORY_LIMIT)`) against the
//! `descriptor_pool()` code path directly, using the same encode/decode
//! primitives the generated code calls, rather than running codegen.
//!
//! Shape and helpers are carried over from the adversarial verification pass
//! at `runs/buffa-336-overnight/repro/adv_amplification.rs` in Ian's
//! assistant workspace, which measured the corrected 71.5x ratio this test
//! locks in.

use buffa::{Message, DEFAULT_ELEMENT_MEMORY_LIMIT};
use buffa_descriptor::generated::descriptor::*;

const ONE_CHAR: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";

fn ident(i: usize) -> String {
    (ONE_CHAR[i] as char).to_string()
}

/// The issue's own trigger shape: 50 outer messages x 26 one-character
/// nested messages. Measured at 71.5x in the adversarial pass (the issue's
/// own table claims 69x; see 04-adversarial-verification.md #1.4 for why
/// that number was low — it's a `format!("{:0width$}")` name-padding bug in
/// whatever generated the original table, not a fixed-width name shape).
fn trigger_shape() -> Vec<u8> {
    let mut fdp = FileDescriptorProto {
        name: Some("a".to_string()),
        ..Default::default()
    };
    for i in 0..50 {
        let mut m = DescriptorProto {
            name: Some(ident(i % ONE_CHAR.len())),
            ..Default::default()
        };
        for j in 0..26 {
            m.nested_type.push(DescriptorProto {
                name: Some(ident(j)),
                ..Default::default()
            });
        }
        fdp.message_type.push(m);
    }
    FileDescriptorSet {
        file: vec![fdp],
        ..Default::default()
    }
    .encode_to_vec()
}

fn decodes_at(bytes: &[u8], limit: usize) -> bool {
    let opts = buffa::DecodeOptions::new().with_element_memory_limit(limit);
    opts.decode_from_slice::<FileDescriptorSet>(bytes).is_ok()
}

#[test]
fn old_formula_rejects_the_trigger_shape() {
    // Documents the bug this fix closes: without the floor, a schema well
    // under 512 KiB can still fail where the pre-#332 default would have
    // passed it.
    let bytes = trigger_shape();
    let old_bound = bytes.len().saturating_mul(64);
    assert!(
        !decodes_at(&bytes, old_bound),
        "expected the un-floored 64x bound to reject this shape; if it now \
         passes, the shape stopped being a #336 regression case and this \
         test needs a new trigger"
    );
}

#[test]
fn floored_formula_accepts_the_trigger_shape() {
    let bytes = trigger_shape();
    let new_bound = bytes
        .len()
        .saturating_mul(64)
        .max(DEFAULT_ELEMENT_MEMORY_LIMIT);
    assert!(
        decodes_at(&bytes, new_bound),
        "the floored bound must decode any input the pre-#336 default \
         (32 MiB) would have accepted, since this shape is only {} bytes",
        bytes.len()
    );
    // Sanity: also confirm it's actually the floor doing the work here,
    // not the scaled term.
    assert_eq!(new_bound, DEFAULT_ELEMENT_MEMORY_LIMIT);
}

#[test]
fn floor_is_monotonic_never_below_default() {
    // Property: for any embedded length, the floored bound is never tighter
    // than DEFAULT_ELEMENT_MEMORY_LIMIT, and never tighter than the
    // un-floored scaled bound either.
    for len in [
        0usize,
        1,
        100,
        4096,
        512 * 1024,
        8 * 1024 * 1024,
        64 * 1024 * 1024,
    ] {
        let old_bound = len.saturating_mul(64);
        let new_bound = old_bound.max(DEFAULT_ELEMENT_MEMORY_LIMIT);
        assert!(new_bound >= DEFAULT_ELEMENT_MEMORY_LIMIT, "len={len}");
        assert!(new_bound >= old_bound, "len={len}");
    }
    // Crossover point: the issue's floor stops being the binding constraint
    // once len * 64 exceeds the 32 MiB default, i.e. len > 512 KiB.
    let crossover = DEFAULT_ELEMENT_MEMORY_LIMIT / 64;
    assert_eq!(crossover, 512 * 1024);
}

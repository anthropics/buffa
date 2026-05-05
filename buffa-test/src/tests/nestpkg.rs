//! Nested-package re-export resolution (gh#80).
//!
//! `crate::nestpkg` and `crate::nestpkg::inner` are wrapped with the same
//! `pub mod a { use super::*; pub mod a_b { use super::*; … } }` chain that
//! `buffa-build`'s `_include.rs` emits. That chain glob-imports the parent
//! package's `__buffa` into the inner package's scope, so a bare
//! `pub use __buffa::…;` re-export would be E0659-ambiguous against the
//! inner package's own (macro-expanded) `pub mod __buffa { … }`.
//!
//! Compilation of the generated code is the primary regression guard;
//! these tests additionally pin down that the natural-path re-exports
//! resolve to the *correct* `__buffa::` canonical type, not the parent
//! package's.

use crate::nestpkg;
use crate::nestpkg::inner;

#[test]
fn natural_path_resolves_to_local_canonical() {
    // `inner::probe::Reading` must alias the inner package's
    // `__buffa::oneof::probe::Reading`, not the outer package's `__buffa`
    // (which has no such member). A type-ascription that mentions both
    // paths is enough — failure to compile is the signal.
    let canonical: inner::__buffa::oneof::probe::Reading = inner::probe::Reading::Scalar(1.5);
    let natural: inner::probe::Reading = canonical;
    let inner::probe::Reading::Scalar(v) = natural else {
        panic!("variant mismatch");
    };
    assert!((v - 1.5).abs() < f64::EPSILON);
}

#[test]
fn outer_package_natural_paths_unaffected() {
    // The outer package's re-exports also resolve correctly even though
    // its `__buffa` is exposed to the inner package via the glob chain.
    let body: nestpkg::wrapper::Body = nestpkg::wrapper::Body::Code(7);
    let canonical: nestpkg::__buffa::oneof::wrapper::Body = body;
    let nestpkg::__buffa::oneof::wrapper::Body::Code(c) = canonical else {
        panic!("variant mismatch");
    };
    assert_eq!(c, 7);
}

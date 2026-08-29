//! Shared descriptor pool (`shared_descriptor_pool(true)`, `$OUT_DIR` mode).
//!
//! The build script compiles `shared_pool_a.proto` / `shared_pool_b.proto`
//! with one shared `__buffa_fds` root (see `build.rs` and
//! `crate::shared_pool` in lib.rs); these tests prove both packages observe
//! the same pool instance and that cross-package symbols resolve through it.
//! With the option off, each package builds its own per-package pool, so the
//! `Arc::ptr_eq` assertion here would fail.

use crate::shared_pool::sharedpool::{a, b};

#[test]
fn packages_share_one_pool_instance() {
    let pool_a = a::descriptor_pool();
    let pool_b = b::descriptor_pool();
    assert!(
        std::sync::Arc::ptr_eq(pool_a, pool_b),
        "both packages must delegate to the one shared pool"
    );
    // And the bytes themselves are physically one copy, not two equal ones.
    assert!(
        core::ptr::eq(
            a::FILE_DESCRIPTOR_SET_BYTES.as_ptr(),
            b::FILE_DESCRIPTOR_SET_BYTES.as_ptr()
        ),
        "both packages must alias the one embedded descriptor set"
    );
}

#[test]
fn shared_pool_resolves_both_packages() {
    // One pool covers the whole codegen run: either package's handle resolves
    // both packages' symbols, including the type `sharedpool.b.MsgB`
    // references across the package boundary.
    let pool = b::descriptor_pool();
    let msg_b = pool
        .message_by_name("sharedpool.b.MsgB")
        .expect("MsgB registered in the shared pool");
    assert!(msg_b.field_by_name("a").is_some(), "field a on MsgB");
    assert!(
        pool.message_by_name("sharedpool.a.MsgA").is_some(),
        "cross-package type must resolve through the same pool"
    );
}

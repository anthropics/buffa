//! Generated prost types with `bytes::Bytes` substituted for every `bytes`
//! field (via `prost-build`'s `.bytes(["."])` config). Decode benchmarks
//! consume `bytes::Bytes` input so prost's zero-copy slicing path for
//! `Bytes` fields is exercised — the fair comparison point for buffa's
//! view-based zero-copy decode (issue
//! [#56](https://github.com/anthropics/buffa/issues/56)).
//!
//! The substitution applies only to proto `bytes` fields, not `string`s,
//! messages, or scalar repeateds. For schemas without `bytes` fields, decode
//! numbers are expected to track the default-prost crate within noise; the
//! `MediaFrame` message in this benchmark suite is the bytes-heavy case
//! where the substitution actually pays off.

pub mod bench {
    include!(concat!(env!("OUT_DIR"), "/bench.rs"));
}

pub mod benchmarks {
    include!(concat!(env!("OUT_DIR"), "/benchmarks.rs"));

    pub mod proto3 {
        include!(concat!(env!("OUT_DIR"), "/benchmarks.proto3.rs"));
    }
}

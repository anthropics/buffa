//! A fast, non-DoS-resistant hasher for `buffa-codegen`'s internal maps
//! (naming/deconfliction bookkeeping, comment lookup, oneof/message
//! indices).
//!
//! Codegen only ever processes trusted, locally-generated `.proto`
//! descriptors — never attacker-controlled input — so `std::HashMap`'s
//! default SipHash (hash-flooding resistance) buys nothing here, and its
//! cost is measurable: profiling a multi-thousand-file codegen run showed
//! `Hasher::write`/`hash_one` (SipHash) as the single largest sampled cost
//! bucket, ahead of any actual codegen logic. `foldhash` is already in the
//! dependency tree transitively (via `hashbrown`), so this adds no new
//! third-party code, just a direct dependency on what's already resolved.
pub(crate) type FxHashMap<K, V> = std::collections::HashMap<K, V, foldhash::fast::RandomState>;

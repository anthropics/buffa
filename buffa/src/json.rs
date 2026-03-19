//! JSON parse options for protobuf JSON deserialization.
//!
//! The protobuf JSON spec says parsers should reject unknown fields by default
//! but **may provide an option** to ignore them. This module exposes that
//! option, matching the per-call semantics of C++ and Java reference
//! implementations.
//!
//! # Two mutually exclusive APIs: scoped (std) vs global (no_std)
//!
//! Serde's `Deserialize` trait has no context parameter, so runtime options
//! must be passed through ambient state. The available mechanism differs
//! between `std` and `no_std` builds — each build exposes exactly one:
//!
//! | Build | Mechanism | API | Scoping |
//! |---|---|---|---|
//! | `std` | Thread-local | `with_json_parse_options` | Per-closure, nestable, per-thread |
//! | `no_std` | `AtomicPtr` to leaked `Box` | `set_global_json_parse_options` | Process-wide, set-once |
//!
//! The two APIs do not interact. `set_global_json_parse_options` is only
//! compiled in `no_std` builds; `with_json_parse_options` only in `std`.
//!
//! ## `std`: scoped per-closure options
//!
//! ```ignore
//! use buffa::json::{JsonParseOptions, with_json_parse_options};
//!
//! let opts = JsonParseOptions::new().ignore_unknown_enum_values(true);
//! let msg = with_json_parse_options(&opts, || {
//!     serde_json::from_str::<MyMessage>(json_str)
//! });
//! // Options revert to defaults here. Concurrent threads are independent.
//! ```
//!
//! ## `no_std`: global set-once options
//!
//! ```ignore
//! use buffa::json::{JsonParseOptions, set_global_json_parse_options};
//!
//! // Call ONCE during startup (e.g. in your init function).
//! set_global_json_parse_options(
//!     &JsonParseOptions::new().ignore_unknown_enum_values(true)
//! );
//!
//! // All subsequent JSON deserialization uses these options.
//! let msg: MyMessage = serde_json::from_str(json_str)?;
//! ```
//!
//! The global setter is **idempotent for identical options** — calling it
//! multiple times with the same configuration is a no-op, so initialization
//! from multiple modules is safe as long as they agree. Calling it with
//! *different* options after the first call triggers a `debug_assert!` (panic
//! in debug builds; the second call is silently ignored in release). Treat the
//! first successful call as locking in behaviour for the process lifetime.
//!
//! ### `no_std` caveat: no container filtering
//!
//! In `std`, `ignore_unknown_enum_values` supports *filtering*: unknown
//! entries in `repeated enum` or `map<_, enum>` fields are dropped from the
//! container. This requires temporarily forcing strict mode to get a
//! distinguishable error, which needs the scoped thread-local.
//!
//! In `no_std`, only *accept-with-default* works: unknown singular enum
//! values become the default (0) variant. Unknown entries in containers
//! still produce an error — the filtering behaviour is unavailable.

/// Options controlling protobuf JSON parsing behavior.
///
/// Use [`JsonParseOptions::new`] plus builder methods to construct:
///
/// ```
/// # use buffa::json::JsonParseOptions;
/// let opts = JsonParseOptions::new().ignore_unknown_enum_values(true);
/// # assert!(opts.ignore_unknown_enum_values);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct JsonParseOptions {
    /// When `true`, unknown enum string values are silently replaced with the
    /// default value (0) for singular fields, or skipped for repeated/map
    /// fields, instead of producing an error.
    pub ignore_unknown_enum_values: bool,
    /// When `true`, `"[pkg.ext]"` JSON keys that are not in the extension
    /// registry produce a parse error instead of being silently dropped.
    ///
    /// The default (`false`, lenient) matches the pre-extension-registry
    /// behavior where all unknown keys were dropped by serde's derive.
    /// protobuf-go (`protojson/decode.go:175`) and protobuf-es
    /// (`from-json.ts:251`) both error on unregistered extension keys unless
    /// their respective ignore-unknown flags are set; set `true` here to
    /// match. The error pinpoints the missing registration.
    ///
    /// Extendee mismatch (key IS registered but extends a different message)
    /// always errors regardless of this flag — that's a contract violation,
    /// not a mere miss.
    pub strict_extension_keys: bool,
}

impl JsonParseOptions {
    /// Create new parse options with all flags at their default (strict) values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set whether unknown enum string values are ignored (replaced with the
    /// default) instead of producing a parse error.
    #[must_use]
    pub fn ignore_unknown_enum_values(mut self, ignore: bool) -> Self {
        self.ignore_unknown_enum_values = ignore;
        self
    }

    /// Set whether unregistered `"[pkg.ext]"` JSON keys produce a parse error
    /// (`true`) or are silently dropped (`false`, the default).
    #[must_use]
    pub fn strict_extension_keys(mut self, strict: bool) -> Self {
        self.strict_extension_keys = strict;
        self
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// std: thread-local scoped options
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "std")]
mod std_impl {
    use super::JsonParseOptions;
    use std::cell::Cell;

    thread_local! {
        static OPTIONS: Cell<JsonParseOptions> = const {
            Cell::new(JsonParseOptions {
                ignore_unknown_enum_values: false,
                strict_extension_keys: false,
            })
        };
    }

    /// Run a closure with the given parse options active.
    ///
    /// The options affect enum deserialization within the closure. This is
    /// **thread-local** state — concurrent parses on different threads are
    /// independent. The previous options are restored when the closure returns
    /// (or panics), so scopes nest correctly.
    ///
    /// This function is only available with the `std` feature. In `no_std`
    /// builds, use `set_global_json_parse_options` instead (see the module
    /// documentation for the usage contract).
    pub fn with_json_parse_options<T>(opts: &JsonParseOptions, f: impl FnOnce() -> T) -> T {
        let prev = OPTIONS.with(|c| c.replace(*opts));
        struct Guard(JsonParseOptions);
        impl Drop for Guard {
            fn drop(&mut self) {
                OPTIONS.with(|c| c.set(self.0));
            }
        }
        let _guard = Guard(prev);
        f()
    }

    pub(crate) fn ignore_unknown_enum_values() -> bool {
        OPTIONS.with(|c| c.get().ignore_unknown_enum_values)
    }

    pub(crate) fn strict_extension_keys() -> bool {
        OPTIONS.with(|c| c.get().strict_extension_keys)
    }
}

#[cfg(feature = "std")]
pub use std_impl::with_json_parse_options;

// ─────────────────────────────────────────────────────────────────────────────
// no_std: global once-cell options (set once, read lock-free)
// ─────────────────────────────────────────────────────────────────────────────
//
// The module is always compiled so its tests run in std builds too, but
// the public `set_global_json_parse_options` re-export is gated on `not(std)`.

/// Global parse-options state for `no_std` builds.
///
/// Uses [`once_cell::race::OnceBox`] — an `AtomicPtr`-based set-once cell
/// that's `no_std`-compatible and already a buffa dependency (for
/// `DefaultInstance`). This stores the full struct on the heap, so future
/// additions to `JsonParseOptions` (non-boolean fields, integers, strings,
/// etc.) require no changes here.
///
/// [`once_cell::race::OnceBox`]: https://docs.rs/once_cell/latest/once_cell/race/struct.OnceBox.html
#[cfg_attr(feature = "std", allow(dead_code))]
mod global {
    use super::JsonParseOptions;
    use alloc::boxed::Box;
    use once_cell::race::OnceBox;

    static OPTS: OnceBox<JsonParseOptions> = OnceBox::new();

    /// Defaults, used when `set_global_json_parse_options` has never been
    /// called. Identical to `JsonParseOptions::default()` but `const`-eval.
    static DEFAULT: JsonParseOptions = JsonParseOptions {
        ignore_unknown_enum_values: false,
        strict_extension_keys: false,
    };

    /// Set the global JSON parse options.
    ///
    /// # Usage contract
    ///
    /// **Call this once during startup** (e.g. in your firmware init or
    /// `main()`). After the first call, the options are locked in for the
    /// process lifetime.
    ///
    /// Multiple calls with **identical** options are permitted and are no-ops —
    /// this supports initialization from multiple modules that agree on
    /// configuration. Multiple calls with **different** options are a bug:
    /// - In debug builds: `debug_assert!` panics with a mismatch diagnostic.
    /// - In release builds: the second call is silently ignored; the first
    ///   call's options remain in effect.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use buffa::json::{JsonParseOptions, set_global_json_parse_options};
    ///
    /// fn init() {
    ///     set_global_json_parse_options(
    ///         &JsonParseOptions::new().ignore_unknown_enum_values(true)
    ///     );
    /// }
    /// ```
    pub fn set_global_json_parse_options(opts: &JsonParseOptions) {
        if OPTS.set(Box::new(*opts)).is_err() {
            // Already set. OnceBox::set returned our rejected Box (which it
            // drops internally); the first caller's options remain in effect.
            // Idempotent re-init with the same options is fine; mismatch is a bug.
            let existing = OPTS.get().expect("set() returned Err ⇒ get() is Some");
            debug_assert_eq!(
                existing, opts,
                "set_global_json_parse_options called with options that differ from the \
                 first call. The first call's options remain in effect. \
                 (existing: {existing:?}, new: {opts:?})"
            );
        }
    }

    /// Get the currently-active options (global or defaults if never set).
    #[inline]
    pub(crate) fn get() -> &'static JsonParseOptions {
        OPTS.get().unwrap_or(&DEFAULT)
    }

    pub(crate) fn ignore_unknown_enum_values() -> bool {
        get().ignore_unknown_enum_values
    }

    pub(crate) fn strict_extension_keys() -> bool {
        get().strict_extension_keys
    }
}

#[cfg(not(feature = "std"))]
pub use global::set_global_json_parse_options;

// ─────────────────────────────────────────────────────────────────────────────
// Crate-internal read accessor (called from json_helpers.rs)
// ─────────────────────────────────────────────────────────────────────────────

/// Returns `true` if unknown enum string values should be silently accepted.
///
/// In `std` builds, reads the thread-local set by [`with_json_parse_options`].
/// In `no_std` builds, reads the global atomic set by
/// [`set_global_json_parse_options`]. The two mechanisms are mutually
/// exclusive (different builds) and do not interact.
pub(crate) fn ignore_unknown_enum_values() -> bool {
    #[cfg(feature = "std")]
    {
        std_impl::ignore_unknown_enum_values()
    }
    #[cfg(not(feature = "std"))]
    {
        global::ignore_unknown_enum_values()
    }
}

/// Returns `true` if unregistered `"[pkg.ext]"` JSON keys should produce a
/// parse error instead of being silently dropped.
pub(crate) fn strict_extension_keys() -> bool {
    #[cfg(feature = "std")]
    {
        std_impl::strict_extension_keys()
    }
    #[cfg(not(feature = "std"))]
    {
        global::strict_extension_keys()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Thread-local (std) behaviour ───────────────────────────────────────
    // These tests use the crate-internal `ignore_unknown_enum_values()` which
    // in std reads ONLY the thread-local, so they don't need to coordinate
    // with the global-atomic tests below.

    #[test]
    fn thread_local_default_does_not_ignore() {
        assert!(!ignore_unknown_enum_values());
    }

    #[test]
    fn thread_local_scope_enables_flag() {
        let opts = JsonParseOptions {
            ignore_unknown_enum_values: true,
            ..Default::default()
        };
        with_json_parse_options(&opts, || {
            assert!(ignore_unknown_enum_values());
        });
        // Restored after closure returns.
        assert!(!ignore_unknown_enum_values());
    }

    #[test]
    fn thread_local_nested_scopes_restore_correctly() {
        let outer = JsonParseOptions {
            ignore_unknown_enum_values: true,
            ..Default::default()
        };
        let inner = JsonParseOptions {
            ignore_unknown_enum_values: false,
            ..Default::default()
        };
        with_json_parse_options(&outer, || {
            assert!(ignore_unknown_enum_values());
            with_json_parse_options(&inner, || {
                assert!(!ignore_unknown_enum_values());
            });
            assert!(ignore_unknown_enum_values());
        });
    }

    #[test]
    fn thread_local_restored_on_panic() {
        let opts = JsonParseOptions {
            ignore_unknown_enum_values: true,
            ..Default::default()
        };
        let result = std::panic::catch_unwind(|| {
            with_json_parse_options(&opts, || {
                assert!(ignore_unknown_enum_values());
                panic!("boom");
            });
        });
        assert!(result.is_err());
        assert!(!ignore_unknown_enum_values());
    }

    // ── Global set-once behaviour (OnceBox) ─────────────────────────────────
    // Single lifecycle test — the production contract is "set once", so we
    // test that contract once. No reset-between-tests (OnceBox deliberately
    // has no reset; that's the point of "once"). The phases run sequentially
    // in one #[test] so ordering is guaranteed.
    //
    // In no_std builds, `global::set_global_json_parse_options` IS the public
    // `set_global_json_parse_options` — this test verifies its contract.

    #[test]
    fn global_set_once_lifecycle() {
        // Phase 1: never set → defaults.
        assert!(
            !global::ignore_unknown_enum_values(),
            "unset global should return strict defaults"
        );
        assert_eq!(*global::get(), JsonParseOptions::default());

        // Phase 2: first set wins, locks in.
        let lenient = JsonParseOptions::new().ignore_unknown_enum_values(true);
        global::set_global_json_parse_options(&lenient);
        assert!(global::ignore_unknown_enum_values());

        // Phase 3: idempotent re-init with identical options — no panic.
        global::set_global_json_parse_options(&lenient);
        global::set_global_json_parse_options(&lenient);
        assert!(global::ignore_unknown_enum_values());

        // Phase 4: mismatch → debug_assert.
        //
        // Run in a child thread so the panic is catchable (debug_assert panics
        // without unwinding the current test); in release builds this phase
        // asserts the first call's options remain in effect instead.
        let strict = JsonParseOptions::new().ignore_unknown_enum_values(false);
        let result = std::thread::spawn(move || {
            global::set_global_json_parse_options(&strict);
        })
        .join();

        #[cfg(debug_assertions)]
        {
            assert!(
                result.is_err(),
                "mismatch should debug_assert-panic in debug builds"
            );
            // Verify the panic message mentions the mismatch.
            let msg = result.unwrap_err();
            let msg_str = msg
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| msg.downcast_ref::<&str>().copied())
                .unwrap_or("");
            assert!(
                msg_str.contains("differ from the first call"),
                "expected mismatch diagnostic, got: {msg_str}"
            );
        }
        #[cfg(not(debug_assertions))]
        {
            assert!(
                result.is_ok(),
                "release builds silently ignore mismatched second call"
            );
        }

        // Either way: first call's options remain in effect.
        assert!(
            global::ignore_unknown_enum_values(),
            "first call's lenient options should remain in effect after mismatch"
        );
    }
}

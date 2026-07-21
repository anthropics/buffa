//! The element-memory bound the `protoc` plugins apply to a
//! `CodeGeneratorRequest`, and its environment-variable override.

use crate::{
    decode_failure, parse_element_memory_limit, tooling_decode_options, ELEMENT_MEMORY_LIMIT_ENV,
    TOOLING_ELEMENT_MEMORY_LIMIT,
};

#[test]
fn unset_and_blank_use_the_plugin_default() {
    for raw in [None, Some(""), Some("   ")] {
        assert_eq!(
            parse_element_memory_limit(raw),
            Ok(TOOLING_ELEMENT_MEMORY_LIMIT),
            "{raw:?} should fall back to the plugin default"
        );
    }
}

#[test]
fn a_byte_count_is_taken_literally() {
    assert_eq!(parse_element_memory_limit(Some("1048576")), Ok(1024 * 1024));
    assert_eq!(parse_element_memory_limit(Some("  4096  ")), Ok(4096));
    // Zero is a legitimate request to reject every repeated element, not a
    // stand-in for "unset" — the blank cases above cover that.
    assert_eq!(parse_element_memory_limit(Some("0")), Ok(0));
}

#[test]
fn unlimited_and_max_remove_the_bound() {
    assert_eq!(
        parse_element_memory_limit(Some("unlimited")),
        Ok(usize::MAX)
    );
    assert_eq!(parse_element_memory_limit(Some("max")), Ok(usize::MAX));
}

#[test]
fn a_bad_value_names_the_variable_and_echoes_the_input() {
    let err = parse_element_memory_limit(Some("banana")).unwrap_err();
    assert!(err.contains(ELEMENT_MEMORY_LIMIT_ENV), "{err}");
    assert!(err.contains("banana"), "{err}");

    // Negative and float forms are the plausible typos; both must be rejected
    // rather than silently truncated.
    assert!(parse_element_memory_limit(Some("-1")).is_err());
    assert!(parse_element_memory_limit(Some("1.5")).is_err());
    assert!(parse_element_memory_limit(Some("64MiB")).is_err());
}

// Properties of the plugin default, checked at compile time since all three
// operands are constants. A CodeGeneratorRequest's element footprint runs ~6x
// its encoded size, and the largest public schema (googleapis with source
// info, ~81 MB) needs ~512 MiB, so the default must clear that with headroom —
// while staying finite, so a truncated request errors instead of exhausting
// memory, and staying well clear of the untrusted-input default it replaces.
const _: () = assert!(TOOLING_ELEMENT_MEMORY_LIMIT >= 1024 * 1024 * 1024);
const _: () = assert!(TOOLING_ELEMENT_MEMORY_LIMIT < usize::MAX);
const _: () = assert!(TOOLING_ELEMENT_MEMORY_LIMIT > buffa::DEFAULT_ELEMENT_MEMORY_LIMIT * 8);

#[test]
fn the_hint_names_the_override_and_the_effective_budget() {
    let msg = decode_failure(
        "CodeGeneratorRequest",
        &buffa::DecodeError::ElementMemoryLimitExceeded,
        2 * 1024 * 1024,
    );
    assert!(msg.contains("CodeGeneratorRequest"), "{msg}");
    // The remedy has to be in the message a user actually sees; the guide is
    // not discoverable from an error you have not yet connected to a knob.
    assert!(msg.contains(ELEMENT_MEMORY_LIMIT_ENV), "{msg}");
    assert!(msg.contains("unlimited"), "{msg}");
    // The budget reported is the one in force, not the default.
    assert!(msg.contains("2 MiB"), "{msg}");
    assert!(
        !msg.contains(&format!("{TOOLING_ELEMENT_MEMORY_LIMIT}")),
        "must not quote the default when an override is in force: {msg}"
    );
}

#[test]
fn a_sub_mib_budget_is_reported_in_bytes_not_rounded_to_zero() {
    // A raw byte count is a legal override, and truncating 512 KiB to "0 MiB"
    // would tell the user to raise a limit the message says is already zero.
    let msg = decode_failure(
        "CodeGeneratorRequest",
        &buffa::DecodeError::ElementMemoryLimitExceeded,
        512 * 1024,
    );
    assert!(msg.contains("524288-byte"), "{msg}");
    assert!(!msg.contains("0 MiB"), "{msg}");
}

#[test]
fn other_decode_errors_get_no_element_memory_hint() {
    let msg = decode_failure(
        "CodeGeneratorRequest",
        &buffa::DecodeError::RecursionLimitExceeded,
        TOOLING_ELEMENT_MEMORY_LIMIT,
    );
    assert!(
        msg.starts_with("failed to decode CodeGeneratorRequest:"),
        "{msg}"
    );
    assert!(
        !msg.contains(ELEMENT_MEMORY_LIMIT_ENV),
        "an unrelated error must not advertise the element-memory knob: {msg}"
    );
}

#[test]
fn tooling_decode_options_carries_the_default_when_unset() {
    // Reads the real environment; asserts only under the common unset case so
    // the test never mutates process-global state.
    if std::env::var(ELEMENT_MEMORY_LIMIT_ENV).is_err() {
        let opts = tooling_decode_options().expect("no override set");
        assert_eq!(opts.element_memory_limit(), TOOLING_ELEMENT_MEMORY_LIMIT);
    }
}

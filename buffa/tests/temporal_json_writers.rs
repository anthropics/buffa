#![cfg(feature = "json")]

use buffa::json_helpers::wkt::{
    fmt_duration, fmt_timestamp, write_duration, write_timestamp, TemporalFormatError,
    MAX_DURATION_SECS, MAX_TIMESTAMP_SECS, MIN_TIMESTAMP_SECS,
};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::fmt::Write;

thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
}

struct AllocationObserver;

// SAFETY: allocations and frees are delegated unchanged to System. Counting
// uses nonallocating thread-local cells and never reads the allocated memory.
unsafe impl GlobalAlloc for AllocationObserver {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|count| {
            if let Some(current) = count.get() {
                count.set(Some(current.saturating_add(1)));
            }
        });
        // SAFETY: the caller supplies a valid layout, forwarded unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the caller's live System allocation and layout are unchanged.
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: AllocationObserver = AllocationObserver;

struct Fixed<const N: usize> {
    bytes: [u8; N],
    length: usize,
}

impl<const N: usize> Default for Fixed<N> {
    fn default() -> Self {
        Self {
            bytes: [0; N],
            length: 0,
        }
    }
}

impl<const N: usize> Fixed<N> {
    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.length]).unwrap()
    }
}

impl<const N: usize> Write for Fixed<N> {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        let end = self.length.checked_add(text.len()).ok_or(std::fmt::Error)?;
        self.bytes
            .get_mut(self.length..end)
            .ok_or(std::fmt::Error)?
            .copy_from_slice(text.as_bytes());
        self.length = end;
        Ok(())
    }
}

const TIMESTAMPS: &[(i64, i32, &str)] = &[
    (MIN_TIMESTAMP_SECS, 0, "0001-01-01T00:00:00Z"),
    (
        MAX_TIMESTAMP_SECS,
        999_999_999,
        "9999-12-31T23:59:59.999999999Z",
    ),
    (0, 0, "1970-01-01T00:00:00Z"),
    (-1, 999_999_999, "1969-12-31T23:59:59.999999999Z"),
    (951_782_400, 0, "2000-02-29T00:00:00Z"),
    (0, 120_000_000, "1970-01-01T00:00:00.120Z"),
    (0, 1_000, "1970-01-01T00:00:00.000001Z"),
    (0, 1, "1970-01-01T00:00:00.000000001Z"),
];

const DURATIONS: &[(i64, i32, &str)] = &[
    (0, 0, "0s"),
    (-1, 0, "-1s"),
    (0, -1, "-0.000000001s"),
    (1, 1_000_000, "1.001s"),
    (1, 1_000, "1.000001s"),
    (MAX_DURATION_SECS, 999_999_999, "315576000000.999999999s"),
    (-MAX_DURATION_SECS, -999_999_999, "-315576000000.999999999s"),
];

#[test]
fn existing_temporal_forms_keep_their_canonical_output() {
    for &(seconds, nanos, expected) in TIMESTAMPS {
        assert_eq!(fmt_timestamp(seconds, nanos).unwrap(), expected);
    }
    for &(seconds, nanos, expected) in DURATIONS {
        assert_eq!(fmt_duration(seconds, nanos).unwrap(), expected);
    }
}

#[test]
fn caller_owned_writers_match_the_legacy_golden_forms() {
    for &(seconds, nanos, expected) in TIMESTAMPS {
        let mut output = Fixed::<30>::default();
        write_timestamp(&mut output, seconds, nanos).unwrap();
        assert_eq!(output.as_str(), expected);
    }
    for &(seconds, nanos, expected) in DURATIONS {
        let mut output = Fixed::<24>::default();
        write_duration(&mut output, seconds, nanos).unwrap();
        assert_eq!(output.as_str(), expected);
    }
}

#[test]
fn writers_validate_before_output_and_propagate_destination_failures() {
    for (seconds, nanos, reason) in [
        (0, -1, TemporalFormatError::TimestampNanos),
        (0, 1_000_000_000, TemporalFormatError::TimestampNanos),
        (
            MIN_TIMESTAMP_SECS.checked_sub(1).unwrap(),
            0,
            TemporalFormatError::TimestampSeconds,
        ),
        (
            MAX_TIMESTAMP_SECS.checked_add(1).unwrap(),
            0,
            TemporalFormatError::TimestampSeconds,
        ),
    ] {
        let mut output = Fixed::<30>::default();
        assert_eq!(write_timestamp(&mut output, seconds, nanos), Err(reason));
        assert_eq!(output.as_str(), "");
        assert!(fmt_timestamp(seconds, nanos).is_err());
    }
    for (seconds, nanos) in [
        (1, -1),
        (-1, 1),
        (0, 1_000_000_000),
        (MAX_DURATION_SECS.checked_add(1).unwrap(), 0),
    ] {
        let mut output = Fixed::<24>::default();
        assert_eq!(
            write_duration(&mut output, seconds, nanos),
            Err(TemporalFormatError::Duration)
        );
        assert_eq!(output.as_str(), "");
        assert!(fmt_duration(seconds, nanos).is_err());
    }
    assert_eq!(
        write_timestamp(&mut Fixed::<0>::default(), 0, 0),
        Err(TemporalFormatError::Write)
    );
    assert_eq!(
        write_duration(&mut Fixed::<0>::default(), 0, 0),
        Err(TemporalFormatError::Write)
    );
    assert_eq!(
        write_timestamp(&mut Fixed::<29>::default(), MAX_TIMESTAMP_SECS, 999_999_999),
        Err(TemporalFormatError::Write)
    );
    assert_eq!(
        write_duration(
            &mut Fixed::<23>::default(),
            -MAX_DURATION_SECS,
            -999_999_999
        ),
        Err(TemporalFormatError::Write)
    );
}

#[test]
fn caller_owned_formatting_does_not_allocate() {
    // The positive control proves that an intermediate owning buffer would
    // be visible to this observer, independently of other test threads.
    ALLOCATIONS.with(|count| count.set(Some(0)));
    let mut temporary = Vec::with_capacity(32);
    temporary.push(7_u8);
    std::hint::black_box(&mut temporary);
    let control = ALLOCATIONS.with(|count| count.replace(None).unwrap());
    assert!(control > 0);

    let mut timestamp = Fixed::<30>::default();
    let mut duration = Fixed::<24>::default();
    ALLOCATIONS.with(|count| count.set(Some(0)));
    let timestamp_result = write_timestamp(&mut timestamp, MAX_TIMESTAMP_SECS, 999_999_999);
    let duration_result = write_duration(&mut duration, -MAX_DURATION_SECS, -999_999_999);
    let allocations = ALLOCATIONS.with(|count| count.replace(None).unwrap());
    assert_eq!(timestamp_result, Ok(()));
    assert_eq!(duration_result, Ok(()));
    assert_eq!(allocations, 0);
}

#[test]
fn writers_append_through_trait_objects_and_keep_partial_output_on_failure() {
    let mut timestamp = String::from("prefix:");
    let writer: &mut dyn Write = &mut timestamp;
    write_timestamp(writer, 0, 0).unwrap();
    assert_eq!(timestamp, "prefix:1970-01-01T00:00:00Z");

    let mut duration = Fixed::<6>::default();
    duration.write_str("keep:").unwrap();
    assert_eq!(
        write_duration(&mut duration, 1, 0),
        Err(TemporalFormatError::Write)
    );
    assert_eq!(duration.as_str(), "keep:1");

    let mut unchanged = String::from("prefix:");
    assert_eq!(
        write_timestamp(&mut unchanged, 0, -1),
        Err(TemporalFormatError::TimestampNanos)
    );
    assert_eq!(unchanged, "prefix:");
}

#[test]
fn quoted_integers_stream_without_temporary_allocations() {
    let mut output = Vec::with_capacity(32);
    for (value, expected) in [
        (i64::MIN, "\"-9223372036854775808\""),
        (0, "\"0\""),
        (i64::MAX, "\"9223372036854775807\""),
    ] {
        output.clear();
        let mut serializer = serde_json::Serializer::new(&mut output);
        ALLOCATIONS.with(|count| count.set(Some(0)));
        let result = buffa::json_helpers::int64::serialize(&value, &mut serializer);
        let allocations = ALLOCATIONS.with(|count| count.replace(None).unwrap());
        assert!(result.is_ok());
        assert_eq!(output, expected.as_bytes());
        assert_eq!(allocations, 0);
    }
    for (value, expected) in [(0, "\"0\""), (u64::MAX, "\"18446744073709551615\"")] {
        output.clear();
        let mut serializer = serde_json::Serializer::new(&mut output);
        ALLOCATIONS.with(|count| count.set(Some(0)));
        let result = buffa::json_helpers::uint64::serialize(&value, &mut serializer);
        let allocations = ALLOCATIONS.with(|count| count.replace(None).unwrap());
        assert!(result.is_ok());
        assert_eq!(output, expected.as_bytes());
        assert_eq!(allocations, 0);
    }
}

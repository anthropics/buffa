//! Demonstrates "natural-path" re-exports of buffa's ancillary types and
//! the manual `__buffa::` fallback when a re-export is dropped.
//!
//! Generated views and oneof enums live unconditionally under a reserved
//! `__buffa::` module so they can never collide with user-declared proto
//! types. As an ergonomic convenience buffa *also* `pub use`-re-exports
//! each one at the path you'd reach for first:
//!
//! | ancillary item            | canonical (`__buffa::`)              | natural re-export      |
//! |---------------------------|--------------------------------------|------------------------|
//! | view of `Foo`             | `__buffa::view::FooView`             | `FooView`              |
//! | view of `Foo.Bar`         | `__buffa::view::foo::BarView`        | `foo::BarView`         |
//! | oneof `kind` of `Foo`     | `__buffa::oneof::foo::Kind`          | `foo::Kind`            |
//! | view of oneof `kind`      | `__buffa::view::oneof::foo::Kind`    | `foo::KindView`        |
//! | file-level `extend` const | `__buffa::ext::MY_OPT`               | `MY_OPT`               |
//! | `register_types`          | `__buffa::register_types`            | `register_types`       |
//!
//! When the natural name is already taken by a real proto item — or two
//! re-exports would collide with *each other* — that re-export is silently
//! skipped. Generated code (field types, method signatures) always uses the
//! `__buffa::` form, so nothing breaks; you just need to import the
//! conflicting type via its canonical path.
//!
//! `proto/conflicts.proto` declares `Event` (no conflicts) and `Probe`
//! (every re-export shadowed) to showcase both halves.

mod proto {
    include!(concat!(env!("OUT_DIR"), "/_include.rs"));
}

use buffa::{Message, MessageView};
use proto::buffa::examples::conflicts::v1 as conflicts;

// ── Happy path: `Event` has no conflicts ────────────────────────────────────
//
// Every ancillary type for `Event` is re-exported at the natural path —
// no `__buffa::` in sight.
use conflicts::event::{Detail, DetailView, Payload, PayloadView};
use conflicts::{Event, EventView};

// ── Conflict path: `Probe` shadows every re-export ──────────────────────────
//
// `Probe` declares a nested `Reading` message (shadows the oneof's
// `probe::Reading` re-export) and a nested `ReadingView` message (shadows
// both view re-exports), and the package declares a top-level `ProbeView`
// message (shadows `Probe`'s view re-export at the package root). The
// canonical `__buffa::` paths are the only way to reach the ancillary
// types. To keep the call sites readable, pick a local alias convention
// that signals "this came from the `__buffa::` fallback" — here we mirror
// the sentinel's `__` prefix. Other reasonable choices: a `Sentinel` or
// `Canonical` suffix, or referencing the full path inline at the (usually
// few) call sites.
use conflicts::__buffa::oneof::probe::Reading as __Reading;
use conflicts::__buffa::view::oneof::probe::Reading as __ReadingView;
use conflicts::__buffa::view::probe::ReadingView as __ReadingMsgView;
use conflicts::__buffa::view::ProbeView as __ProbeView;
use conflicts::probe::{Reading, ReadingView};
use conflicts::{Probe, ProbeView};

fn main() {
    happy_path();
    conflict_path();
    println!("ok");
}

/// `Event`: re-exports work, the natural paths read like the proto.
fn happy_path() {
    let event = Event {
        payload: Some(Payload::Detail(Box::new(Detail {
            note: "ping".to_string(),
            ..Default::default()
        }))),
        ..Default::default()
    };

    let bytes = event.encode_to_vec();

    // Decode an owned copy and match the oneof — `Payload` is the
    // re-export of `__buffa::oneof::event::Payload`.
    let decoded = Event::decode(&mut &bytes[..]).unwrap();
    let note = match &decoded.payload {
        Some(Payload::Detail(d)) => d.note.clone(),
        Some(Payload::Text(t)) => t.clone(),
        None => String::new(),
    };
    assert_eq!(note, "ping");

    // Decode a zero-copy view and match the oneof view — `PayloadView`
    // is the renamed re-export of `__buffa::view::oneof::event::Payload`.
    let view: EventView<'_> = EventView::decode_view(&bytes[..]).unwrap();
    let view_note = match &view.payload {
        Some(PayloadView::Detail(d)) => d.note.to_string(),
        Some(PayloadView::Text(t)) => t.to_string(),
        None => String::new(),
    };
    assert_eq!(view_note, "ping");

    // The nested view re-export resolves too — used here only to show that
    // the path exists.
    fn _typed(_: DetailView<'_>) {}
}

/// `Probe`: every re-export is shadowed by a real proto type, so the
/// canonical `__buffa::` path is the only way to reach the ancillary
/// types. The `__`-prefixed aliases above keep the call sites readable
/// while making it visually obvious that the import goes through `__buffa::`.
fn conflict_path() {
    let probe = Probe {
        // `__Reading` is the oneof enum (`__buffa::oneof::probe::Reading`),
        // *not* the nested `probe::Reading` message that shadows it.
        reading: Some(__Reading::Sample(Box::new(Reading {
            value: 1.5,
            ..Default::default()
        }))),
        ..Default::default()
    };

    let bytes = probe.encode_to_vec();

    // `Probe`'s view: `conflicts::ProbeView` is the *message* declared in
    // the proto, not `Probe`'s zero-copy view — that's only at
    // `__buffa::view::ProbeView`, aliased above as `__ProbeView`.
    let view: __ProbeView<'_> = __ProbeView::decode_view(&bytes[..]).unwrap();
    let value = match &view.reading {
        Some(__ReadingView::Sample(s)) => s.value,
        Some(__ReadingView::Scalar(s)) => *s,
        None => 0.0,
    };
    assert!((value - 1.5).abs() < f64::EPSILON);

    // The shadowing types are still ordinary, usable proto messages:
    let _msg = ProbeView {
        label: "i shadow Probe's view re-export".to_string(),
        ..Default::default()
    };
    let _nested_msg = ReadingView {
        value: 2.0,
        ..Default::default()
    };
    // And nested `ReadingView`'s zero-copy view *is* still reachable
    // through `__buffa::` even though its natural re-export was dropped.
    fn _typed(_: __ReadingMsgView<'_>) {}
}

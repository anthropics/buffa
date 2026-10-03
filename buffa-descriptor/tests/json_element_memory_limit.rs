//! The reflective JSON parser bounds the repeated elements and map entries it
//! builds.
//!
//! `{},` is three input bytes and one `size_of::<Value>()` slot in the `Vec`
//! it lands in, so without a bound about 1.5 MB of JSON builds 32 MiB. These
//! tests check that every site that accumulates from input —
//! repeated fields, map entries, `Struct` members, `ListValue` elements,
//! `FieldMask` paths, and the `serde_json::Value` tree an `Any` payload is
//! buffered into — is charged, that one budget covers a whole parse
//! rather than resetting per nested message or `Any` layer, and that the
//! element charges match the reflective binary decoder's.
//!
//! Run standalone with (the file compiles to nothing without the features):
//! `cargo test -p buffa-descriptor --features reflect,json --test json_element_memory_limit`

#![cfg(all(feature = "reflect", feature = "json", feature = "std"))]

use std::mem::size_of;
use std::sync::Arc;

use buffa_descriptor::reflect::{DynamicMessage, DynamicMessageSeed, MapKey, Value};
use buffa_descriptor::DescriptorPool;

/// `reflect.test.{Containers,Inner,Scalars}` + `field_mask.proto`.
const FDS_BYTES: &[u8] = include_bytes!("protos/reflect_test.fds");

/// `descriptor.proto` + `any.proto` + `reflect.opt.Envelope { Any payload = 1; }`.
const OPTIONS_FDS_BYTES: &[u8] = include_bytes!("protos/reflect_test_options.fds");

/// `any.proto` + `reflect.json.Batch { repeated int32 xs = 1; Any payload = 2;
/// repeated Any payloads = 3; }`, an `Any` beside a repeated field. Regenerate
/// with: `protoc --descriptor_set_out=buffa-descriptor/tests/protos/json_any_test.fds
/// --include_imports -I buffa-types/protos -I buffa-descriptor/tests/protos
/// json_any_test.proto`
const ANY_FDS_BYTES: &[u8] = include_bytes!("protos/json_any_test.fds");

/// `struct.proto` + `reflect.json.Payload`, for the `Struct` / `Value` /
/// `ListValue` codecs. Regenerate with:
/// `protoc --descriptor_set_out=buffa-descriptor/tests/protos/json_struct_test.fds
/// --include_imports -I buffa-types/protos -I buffa-descriptor/tests/protos
/// json_struct_test.proto`
const STRUCT_FDS_BYTES: &[u8] = include_bytes!("protos/json_struct_test.fds");

/// One repeated element costs a `Value` slot, as in `reflect/dynamic.rs`.
const ELEMENT: usize = size_of::<Value>();
/// One map entry costs a key slot plus a value slot, likewise.
const MAP_ENTRY: usize = size_of::<MapKey>() + size_of::<Value>();
/// The buffer an `Any` payload is read into costs a map entry for each object
/// member it holds, and a repeated element for each array element.
const BUFFERED_MEMBER: usize = MAP_ENTRY;
const BUFFERED_ELEMENT: usize = ELEMENT;

/// The budget lives in a `Cell` borrowed for the duration of one parse, not
/// in an `Rc`/`Arc<Cell<_>>` field on the seed — holding one would make
/// `DynamicMessageSeed` `!Send`, a silent break for anyone moving a
/// configured seed across threads. This fails to compile if that changes.
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<DynamicMessageSeed>();
};

fn pool(bytes: &[u8]) -> Arc<DescriptorPool> {
    Arc::new(DescriptorPool::decode(bytes).expect("pool builds from protoc FDS"))
}

/// Parse under an explicit budget.
fn parse_with_limit(
    p: &Arc<DescriptorPool>,
    full_name: &str,
    json: &str,
    limit: usize,
) -> Result<DynamicMessage, serde_json::Error> {
    let idx = p.message_index(full_name).expect("message is in the pool");
    DynamicMessageSeed::new(Arc::clone(p), idx)
        .with_element_memory_limit(limit)
        .parse_json(json)
}

/// The one error this budget produces, whatever the site that raised it.
fn assert_over_budget(err: &serde_json::Error) {
    assert!(
        DynamicMessageSeed::is_element_memory_limit_error(err),
        "expected the element-memory error, got: {err}"
    );
}

/// `n` copies of `item`, comma-joined, inside `open`/`close`.
fn repeat_json(open: &str, item: &str, n: usize, close: &str) -> String {
    let mut s = String::with_capacity(open.len() + n * (item.len() + 1) + close.len());
    s.push_str(open);
    for i in 0..n {
        if i > 0 {
            s.push(',');
        }
        s.push_str(item);
    }
    s.push_str(close);
    s
}

/// `DynamicMessage::from_json` with no options refuses the amplification that
/// `DecodeOptions` refuses on the wire.
///
/// Sized off the real default rather than a lowered budget, so a regression
/// that drops the default (or makes the limit opt-in) fails here.
#[test]
fn from_json_bounds_repeated_message_elements_by_default() {
    let p = pool(FDS_BYTES);
    let idx = p.message_index("reflect.test.Containers").unwrap();
    let n = buffa::DEFAULT_ELEMENT_MEMORY_LIMIT / ELEMENT + 1;

    let json = repeat_json(r#"{"inners":["#, "{}", n, "]}");
    // The input is a fraction of the footprint it would build.
    assert!(json.len() * 4 < buffa::DEFAULT_ELEMENT_MEMORY_LIMIT);

    let err = DynamicMessage::from_json(Arc::clone(&p), idx, &json)
        .expect_err("an over-budget repeated field must be refused");
    assert_over_budget(&err);

    // The lenient entry point is bounded too — `ignore_unknown` relaxes the
    // unknown-field check, not the budget.
    let err = DynamicMessage::from_json_ignoring_unknown(Arc::clone(&p), idx, &json)
        .expect_err("lenient parsing does not lift the budget");
    assert_over_budget(&err);

    // An ordinary message is unaffected.
    let ok = DynamicMessage::from_json(p, idx, r#"{"inners":[{"id":"a"},{"id":"b"}]}"#)
        .expect("a small message still parses");
    assert_eq!(
        ok.to_json().unwrap(),
        r#"{"inners":[{"id":"a"},{"id":"b"}]}"#
    );
}

/// The budget is spent exactly, not approximately: `k` elements fit a
/// `k * ELEMENT` budget and `k + 1` do not.
#[test]
fn repeated_elements_are_charged_one_value_slot_each() {
    let p = pool(FDS_BYTES);
    let budget = 64 * ELEMENT;

    let fits = repeat_json(r#"{"inners":["#, "{}", 64, "]}");
    parse_with_limit(&p, "reflect.test.Containers", &fits, budget).expect("64 elements fit");

    let one_over = repeat_json(r#"{"inners":["#, "{}", 65, "]}");
    let err = parse_with_limit(&p, "reflect.test.Containers", &one_over, budget)
        .expect_err("65 elements do not");
    assert_over_budget(&err);
}

/// Repeated scalars are charged, matching the reflective binary decoder and
/// unlike the generated one: this store is a `Vec<Value>`, so a one-byte
/// JSON number costs a whole `Value` slot.
#[test]
fn repeated_scalars_are_charged_like_the_reflective_binary_decoder() {
    let p = pool(FDS_BYTES);
    let budget = 8 * ELEMENT;

    let fits = repeat_json(r#"{"packedInts":["#, "1", 8, "]}");
    parse_with_limit(&p, "reflect.test.Containers", &fits, budget).expect("8 scalars fit");

    let one_over = repeat_json(r#"{"packedInts":["#, "1", 9, "]}");
    let err = parse_with_limit(&p, "reflect.test.Containers", &one_over, budget)
        .expect_err("9 scalars do not");
    assert_over_budget(&err);
}

/// A map entry costs a key slot plus a value slot — the charge
/// `reflect/dynamic.rs` applies on the wire, so the same map costs the same
/// budget on either codec.
#[test]
fn map_entries_are_charged_a_key_slot_plus_a_value_slot() {
    let p = pool(FDS_BYTES);
    let budget = 4 * MAP_ENTRY;

    let fits = r#"{"tags":{"a":1,"b":2,"c":3,"d":4}}"#;
    parse_with_limit(&p, "reflect.test.Containers", fits, budget).expect("4 entries fit");

    let one_over = r#"{"tags":{"a":1,"b":2,"c":3,"d":4,"e":5}}"#;
    let err = parse_with_limit(&p, "reflect.test.Containers", one_over, budget)
        .expect_err("5 entries do not");
    assert_over_budget(&err);

    // A budget one byte short of four entries is still short.
    let err = parse_with_limit(&p, "reflect.test.Containers", fits, budget - 1)
        .expect_err("the budget is spent exactly");
    assert_over_budget(&err);
}

/// `FieldMask` parses a comma-separated string straight into
/// `repeated string paths`, so each comma buys another `Value` slot.
#[test]
fn field_mask_paths_are_charged() {
    let p = pool(FDS_BYTES);
    let budget = 4 * ELEMENT;

    let fits = r#"{"fFieldMask":"a,b,c,d"}"#;
    parse_with_limit(&p, "reflect.test.Scalars", fits, budget).expect("4 paths fit");

    let one_over = r#"{"fFieldMask":"a,b,c,d,e"}"#;
    let err =
        parse_with_limit(&p, "reflect.test.Scalars", one_over, budget).expect_err("5 paths do not");
    assert_over_budget(&err);

    // The charge lands before `field_mask_to_snake` runs, so an over-budget
    // mask is refused for the budget rather than for its (valid) contents.
    let empty_budget =
        parse_with_limit(&p, "reflect.test.Scalars", fits, 0).expect_err("no budget, no paths");
    assert_over_budget(&empty_budget);
}

/// `google.protobuf.Struct` and `ListValue` accumulate from input through
/// their own hand-written visitors, and are charged the same as the map and
/// repeated fields they mirror.
#[test]
fn struct_members_and_list_value_elements_are_charged() {
    let p = pool(STRUCT_FDS_BYTES);

    let fits = r#"{"a":1,"b":2,"c":3}"#;
    parse_with_limit(&p, "google.protobuf.Struct", fits, 3 * MAP_ENTRY)
        .expect("3 Struct members fit");
    let err = parse_with_limit(&p, "google.protobuf.Struct", fits, 3 * MAP_ENTRY - 1)
        .expect_err("3 Struct members do not fit in less");
    assert_over_budget(&err);

    let list = r#"[1,2,3,4]"#;
    parse_with_limit(&p, "google.protobuf.ListValue", list, 4 * ELEMENT)
        .expect("4 ListValue elements fit");
    let err = parse_with_limit(&p, "google.protobuf.ListValue", list, 4 * ELEMENT - 1)
        .expect_err("4 ListValue elements do not fit in less");
    assert_over_budget(&err);
}

/// A declared field of type `google.protobuf.Value` takes its own dispatch
/// path (JSON `null` there means "present, null" rather than "unset"), so it
/// gets its own check that the budget travels with it.
#[test]
fn value_typed_fields_are_charged_on_the_parse_budget() {
    let p = pool(STRUCT_FDS_BYTES);

    // Singular `Value` holding an object: charged as `Struct` members.
    let singular = r#"{"value":{"a":1,"b":2}}"#;
    parse_with_limit(&p, "reflect.json.Payload", singular, 2 * MAP_ENTRY).expect("2 members fit");
    let err = parse_with_limit(&p, "reflect.json.Payload", singular, 2 * MAP_ENTRY - 1)
        .expect_err("2 members do not fit in less");
    assert_over_budget(&err);

    // `repeated Value`: charged by the ordinary repeated path, one slot each.
    let repeated = r#"{"items":[1,2,3]}"#;
    parse_with_limit(&p, "reflect.json.Payload", repeated, 3 * ELEMENT).expect("3 elements fit");
    let err = parse_with_limit(&p, "reflect.json.Payload", repeated, 3 * ELEMENT - 1)
        .expect_err("3 elements do not fit in less");
    assert_over_budget(&err);
}

/// The budget belongs to the parse, not to a message: nesting must not hand
/// a sub-message a fresh allowance. A per-message budget would accept this
/// input, since neither subtree exceeds it alone.
#[test]
fn nested_messages_draw_on_one_shared_budget() {
    let p = pool(STRUCT_FDS_BYTES);
    // Two sibling subtrees, three levels down, three elements each.
    let json = r#"{"x":{"y":[1,2,3]},"z":{"w":[1,2,3]}}"#;

    // 4 `Struct` members (x, y, z, w) + 6 `ListValue` elements.
    let exact = 4 * MAP_ENTRY + 6 * ELEMENT;
    parse_with_limit(&p, "google.protobuf.Struct", json, exact).expect("the whole tree fits");

    let err = parse_with_limit(&p, "google.protobuf.Struct", json, exact - 1)
        .expect_err("the subtrees share one budget");
    assert_over_budget(&err);

    // Enough for either subtree on its own, but not both: the case a
    // per-message budget would wrongly accept.
    let err = parse_with_limit(&p, "google.protobuf.Struct", json, exact - ELEMENT)
        .expect_err("a per-message budget would accept this");
    assert_over_budget(&err);
}

/// An `Any` payload is buffered, so that `@type` can follow the fields, and
/// then decoded from the buffer into a second `DynamicMessage`. Both draw on
/// the outer budget, and the buffer is held while the decode runs.
#[test]
fn any_payloads_continue_the_outer_budget() {
    let p = pool(OPTIONS_FDS_BYTES);
    let inner = repeat_json(
        r#"{"@type":"type.googleapis.com/google.protobuf.DescriptorProto","nestedType":["#,
        "{}",
        8,
        "]}",
    );
    let json = format!(r#"{{"payload":{inner}}}"#);

    // The buffer holds 2 members (`@type`, `nestedType`) and the 8
    // `nestedType` elements. The decode then builds the 8 elements while the
    // buffer is held.
    let peak = 2 * BUFFERED_MEMBER + 8 * BUFFERED_ELEMENT + 8 * ELEMENT;
    parse_with_limit(&p, "reflect.opt.Envelope", &json, peak)
        .expect("the buffer and the 8 elements fit");

    // The error is identifiable from inside the `Any` too.
    let err = parse_with_limit(&p, "reflect.opt.Envelope", &json, peak - 1)
        .expect_err("one byte short, the Any payload is refused");
    assert_over_budget(&err);
    // The text is the documented one, which the binary codec's error displays.
    assert!(err.to_string().starts_with("element memory limit exceeded"));
    assert_eq!(
        buffa::DecodeError::ElementMemoryLimitExceeded.to_string(),
        "element memory limit exceeded"
    );
}

/// The budget error keeps its text through two `Any` layers, and an inner
/// error that only quotes that text does not pass for it.
#[test]
fn the_budget_error_is_identifiable_through_nested_any_layers() {
    let p = pool(OPTIONS_FDS_BYTES);
    let descriptor = repeat_json(
        r#"{"@type":"type.googleapis.com/google.protobuf.DescriptorProto","nestedType":["#,
        "{}",
        2,
        "]}",
    );
    let json = format!(
        r#"{{"payload":{{"@type":"type.googleapis.com/reflect.opt.Envelope","payload":{descriptor}}}}}"#
    );

    // The outer buffer holds its two members, the inner payload's two members
    // and the 2 `nestedType` elements. Decoding it reaches the inner `Any`,
    // which buffers its own 2 members and 2 elements, and then builds the 2
    // elements. All three are held at once.
    let outer_buffer = 4 * BUFFERED_MEMBER + 2 * BUFFERED_ELEMENT;
    let inner_buffer = 2 * BUFFERED_MEMBER + 2 * BUFFERED_ELEMENT;
    let peak = outer_buffer + inner_buffer + 2 * ELEMENT;
    parse_with_limit(&p, "reflect.opt.Envelope", &json, peak)
        .expect("both buffers and the 2 elements fit");
    let err = parse_with_limit(&p, "reflect.opt.Envelope", &json, peak - 1)
        .expect_err("one byte short across the two Any layers");
    assert_over_budget(&err);

    let quoted = concat!(
        r#"{"payload":{"@type":"type.googleapis.com/google.protobuf.DescriptorProto","#,
        r#""element memory limit exceeded":1}}"#
    );
    let err = parse_with_limit(&p, "reflect.opt.Envelope", quoted, usize::MAX)
        .expect_err("an unknown field inside the Any is an error");
    assert!(
        !DynamicMessageSeed::is_element_memory_limit_error(&err),
        "{err}"
    );
    assert!(
        err.to_string()
            .starts_with("Any inner deserialize failed: "),
        "{err}"
    );
}

/// The buffer is charged for every member, including singular fields, which
/// cost nothing in the message decoded from it.
#[test]
fn an_any_buffer_is_charged_even_when_the_decode_is_not() {
    let p = pool(OPTIONS_FDS_BYTES);
    // The buffer holds two members: `@type` and `name`.
    let json =
        r#"{"payload":{"@type":"type.googleapis.com/google.protobuf.DescriptorProto","name":"X"}}"#;

    parse_with_limit(&p, "reflect.opt.Envelope", json, 2 * BUFFERED_MEMBER)
        .expect("the two-member buffer fits");
    let err = parse_with_limit(&p, "reflect.opt.Envelope", json, 2 * BUFFERED_MEMBER - 1)
        .expect_err("the buffer's second member does not fit");
    assert_over_budget(&err);
}

/// With the default limit, an `Any` payload that is one repeated field holds
/// about a quarter of a million elements. Each costs an element in the buffer
/// and an element in the message decoded from it.
#[test]
fn the_default_limit_covers_an_any_buffer_and_its_message_together() {
    let p = pool(OPTIONS_FDS_BYTES);
    let idx = p.message_index("reflect.opt.Envelope").unwrap();
    let json = |n: usize| {
        let payload = repeat_json(
            r#"{"@type":"type.googleapis.com/google.protobuf.DescriptorProto","nestedType":["#,
            "{}",
            n,
            "]}",
        );
        format!(r#"{{"payload":{payload}}}"#)
    };

    // The buffer holds 2 members and `n` elements, and the decode builds `n`.
    let limit = buffa::DEFAULT_ELEMENT_MEMORY_LIMIT;
    let n = (limit - 2 * BUFFERED_MEMBER) / (BUFFERED_ELEMENT + ELEMENT);
    // The `with_element_memory_limit` documentation gives this capacity.
    #[cfg(target_pointer_width = "64")]
    assert_eq!(n, 262_142);

    DynamicMessage::from_json(Arc::clone(&p), idx, &json(n)).expect("n elements fit");
    let err = DynamicMessage::from_json(p, idx, &json(n + 1))
        .expect_err("one more element is charged twice");
    assert_over_budget(&err);
}

/// With the default limit, the buffer of an `Any` payload of singular fields
/// holds about 381,000 members. The decode charges nothing for them. A member
/// whose name repeats an earlier one is charged again, so the payload here
/// repeats one field.
#[test]
fn the_default_limit_covers_an_any_buffer_of_singular_fields() {
    let p = pool(OPTIONS_FDS_BYTES);
    let idx = p.message_index("reflect.opt.Envelope").unwrap();
    // `@type` and `names` copies of `name`.
    let json = |names: usize| {
        let payload = repeat_json(
            r#"{"@type":"type.googleapis.com/google.protobuf.DescriptorProto","#,
            r#""name":"X""#,
            names,
            "}",
        );
        format!(r#"{{"payload":{payload}}}"#)
    };

    let members = buffa::DEFAULT_ELEMENT_MEMORY_LIMIT / BUFFERED_MEMBER;
    // The `with_element_memory_limit` documentation gives this capacity.
    #[cfg(target_pointer_width = "64")]
    assert_eq!(members, 381_300);

    DynamicMessage::from_json(Arc::clone(&p), idx, &json(members - 1))
        .expect("the members fit the default limit");
    let err = DynamicMessage::from_json(p, idx, &json(members))
        .expect_err("one more member does not fit");
    assert_over_budget(&err);
}

/// The parse fails at the value that exceeds the budget, before it reads the
/// rest of the payload. The input here stops being JSON after the third
/// element, and the error is still the budget's.
#[test]
fn an_any_buffer_stops_at_the_value_that_exceeds_the_budget() {
    let p = pool(OPTIONS_FDS_BYTES);
    let json = concat!(
        r#"{"payload":{"@type":"type.googleapis.com/google.protobuf.DescriptorProto","#,
        r#""nestedType":[{},{},{},!"#
    );

    // Room for `@type` and two of the elements.
    let limit = BUFFERED_MEMBER + 2 * BUFFERED_ELEMENT;
    let err = parse_with_limit(&p, "reflect.opt.Envelope", json, limit)
        .expect_err("the third element does not fit");
    assert_over_budget(&err);

    let err = parse_with_limit(&p, "reflect.opt.Envelope", json, usize::MAX)
        .expect_err("with room for it, the parse reaches the malformed input");
    assert!(
        !DynamicMessageSeed::is_element_memory_limit_error(&err),
        "{err}"
    );
}

/// An `Any` whose payload is an `Any` carries it under `"value"`. The outer
/// buffer holds 4 members (`@type`, `value`, and inside it `@type` and `xs`)
/// and one element. The inner buffer holds 2 members and one element, and the
/// decode builds 1 element.
#[test]
fn a_value_wrapped_any_payload_is_charged_at_each_layer() {
    let p = pool(ANY_FDS_BYTES);
    let json = concat!(
        r#"{"payload":{"@type":"type.googleapis.com/google.protobuf.Any","#,
        r#""value":{"@type":"type.googleapis.com/reflect.json.Batch","xs":[1]}}}"#
    );

    let outer_buffer = 4 * BUFFERED_MEMBER + BUFFERED_ELEMENT;
    let inner_buffer = 2 * BUFFERED_MEMBER + BUFFERED_ELEMENT;
    let peak = outer_buffer + inner_buffer + ELEMENT;
    parse_with_limit(&p, "reflect.json.Batch", json, peak).expect("both buffers fit");
    let err =
        parse_with_limit(&p, "reflect.json.Batch", json, peak - 1).expect_err("one byte short");
    assert_over_budget(&err);
}

/// An unknown member of an `Any` is buffered with the rest of the payload,
/// so it is charged while the buffer is held, and the charge is returned
/// with the buffer's.
#[test]
fn an_ignored_unknown_member_of_an_any_is_charged_while_it_is_buffered() {
    let p = pool(ANY_FDS_BYTES);
    let idx = p.message_index("reflect.json.Batch").unwrap();
    let parse = |limit: usize| {
        DynamicMessageSeed::new(Arc::clone(&p), idx)
            .ignore_unknown_fields(true)
            .with_element_memory_limit(limit)
            .parse_json(concat!(
                r#"{"payload":{"@type":"type.googleapis.com/google.protobuf.Any","#,
                r#""value":{},"junk":[1,2,3]},"xs":[1,2]}"#
            ))
    };

    // The buffer holds the members `@type`, `value` and `junk`, and the three
    // elements of `junk`. The 2 `xs` elements fit after that charge is
    // returned.
    let buffer = 3 * BUFFERED_MEMBER + 3 * BUFFERED_ELEMENT;
    assert!(buffer > 2 * ELEMENT);
    parse(buffer).expect("the buffer fits");
    let err = parse(buffer - 1).expect_err("one byte short");
    assert_over_budget(&err);
}

/// The buffer's charge is returned when its payload has been decoded, so a
/// run of `Any` payloads needs room for one buffer, not for all of them.
#[test]
fn an_any_buffer_charge_is_returned_after_its_payload_is_decoded() {
    let p = pool(ANY_FDS_BYTES);
    let json = repeat_json(
        r#"{"payloads":["#,
        r#"{"@type":"type.googleapis.com/reflect.json.Batch","xs":[1,2,3]}"#,
        3,
        "]}",
    );

    // Each payload buffers 2 members (`@type`, `xs`) and three elements, and
    // keeps 4 elements: its three `xs` elements, and its own slot in
    // `payloads`. The third payload peaks at the 8 elements kept before it,
    // its buffer and its 3 elements.
    let buffer = 2 * BUFFERED_MEMBER + 3 * BUFFERED_ELEMENT;
    let peak = 8 * ELEMENT + buffer + 3 * ELEMENT;
    parse_with_limit(&p, "reflect.json.Batch", &json, peak).expect("one buffer at a time fits");
    let err = parse_with_limit(&p, "reflect.json.Batch", &json, peak - 1)
        .expect_err("one byte short at the third payload");
    assert_over_budget(&err);
    // Three buffers that stayed charged would need more than that with the
    // 12 elements the parse keeps.
    assert!(3 * buffer + 12 * ELEMENT > peak);
}

/// The buffer is charged on top of what the parse has kept so far, so an
/// `Any` that follows a repeated field needs more room than one that precedes
/// it.
#[test]
fn an_any_buffer_is_charged_on_top_of_the_elements_kept_before_it() {
    let p = pool(ANY_FDS_BYTES);
    let payload = r#""payload":{"@type":"type.googleapis.com/reflect.json.Batch","xs":[1]}"#;
    let xs = r#""xs":[1,2]"#;

    // `Any` first: the buffer holds 2 members and 1 element, and the decode
    // builds 1 element. The buffer's charge is returned before `xs` adds 2.
    let any_alone = 2 * BUFFERED_MEMBER + BUFFERED_ELEMENT + ELEMENT;
    let any_first = format!("{{{payload},{xs}}}");
    parse_with_limit(&p, "reflect.json.Batch", &any_first, any_alone).expect("the Any fits");
    let err = parse_with_limit(&p, "reflect.json.Batch", &any_first, any_alone - 1)
        .expect_err("one byte short");
    assert_over_budget(&err);

    // `Any` last: the 2 `xs` elements are kept while the same charge is made.
    let after_xs = 2 * ELEMENT + any_alone;
    let any_last = format!("{{{xs},{payload}}}");
    parse_with_limit(&p, "reflect.json.Batch", &any_last, after_xs)
        .expect("the Any fits on top of xs");
    let err = parse_with_limit(&p, "reflect.json.Batch", &any_last, after_xs - 1)
        .expect_err("one byte short");
    assert_over_budget(&err);
}

/// A `ListValue` element is itself a `Value` and can hold another list. The
/// inner lists draw on the budget the outer one does.
#[test]
fn nested_list_values_draw_on_one_shared_budget() {
    let p = pool(STRUCT_FDS_BYTES);
    let json = "[[1,2],[3,4]]";

    // 2 outer elements + 4 inner ones.
    let exact = 6 * ELEMENT;
    parse_with_limit(&p, "google.protobuf.ListValue", json, exact).expect("the whole tree fits");

    let err = parse_with_limit(&p, "google.protobuf.ListValue", json, exact - 1)
        .expect_err("the inner lists share the outer budget");
    assert_over_budget(&err);
}

/// `is_element_memory_limit_error` is true for the budget error only.
#[test]
fn malformed_input_is_not_the_budget_error() {
    let p = pool(FDS_BYTES);
    for malformed in [
        r#"{"inners":7}"#,
        r#"{"inners":["#,
        // An unknown field whose name is the budget error's text.
        r#"{"element memory limit exceeded":1}"#,
    ] {
        let err = parse_with_limit(&p, "reflect.test.Containers", malformed, usize::MAX)
            .expect_err("malformed input is an error");
        assert!(
            !DynamicMessageSeed::is_element_memory_limit_error(&err),
            "{malformed}: {err}"
        );
    }
}

/// `parse_json` applies the seed's options, and rejects input after the
/// document as `from_json` does.
#[test]
fn parse_json_applies_the_seed_options_and_rejects_trailing_input() {
    let p = pool(FDS_BYTES);
    let idx = p.message_index("reflect.test.Containers").unwrap();
    let seed = || DynamicMessageSeed::new(Arc::clone(&p), idx);
    let json = repeat_json(r#"{"inners":["#, "{}", 3, "]}");

    seed()
        .with_element_memory_limit(3 * ELEMENT)
        .parse_json(&json)
        .expect("3 elements fit");
    let err = seed()
        .with_element_memory_limit(2 * ELEMENT)
        .parse_json(&json)
        .expect_err("3 elements do not fit in 2 slots");
    assert_over_budget(&err);

    seed()
        .parse_json(r#"{"nope":1}"#)
        .expect_err("an unknown field is an error by default");
    seed()
        .ignore_unknown_fields(true)
        .parse_json(r#"{"nope":1}"#)
        .expect("an unknown field is ignored on request");

    let err = seed()
        .parse_json(r#"{"inners":[]} {}"#)
        .expect_err("input after the document is an error");
    assert!(!DynamicMessageSeed::is_element_memory_limit_error(&err));
}

/// `usize::MAX` means no limit, for callers who parse trusted input.
#[test]
fn usize_max_disables_the_budget() {
    let p = pool(FDS_BYTES);
    let n = buffa::DEFAULT_ELEMENT_MEMORY_LIMIT / ELEMENT + 1;
    let json = repeat_json(r#"{"inners":["#, "{}", n, "]}");

    let msg = parse_with_limit(&p, "reflect.test.Containers", &json, usize::MAX)
        .expect("an unlimited budget parses what the default refuses");
    match msg.field_by_number(8) {
        Some(Value::List(l)) => assert_eq!(l.len(), n),
        other => panic!("expected a list of {n} elements, got {other:?}"),
    }
}

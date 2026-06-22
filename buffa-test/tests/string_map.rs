//! Custom `ProtoString` type for `map` keys and values.
//!
//! `string_map.proto` is compiled with `.string_type_custom("crate::string_map::MapStr")`,
//! so every `string` map key and value is the crate-local `MapStr` newtype
//! instead of `String`. The fields cover each JSON serde-dispatch path for a
//! custom string key/value:
//!
//!   ss    string→string  : serde derive (object key + string value)
//!   si64  string→int64   : `proto_str_key_map` (quoted int64 value)
//!   sf64  string→double  : `proto_str_key_map`
//!   i32s  int32→string   : `string_key_map` (non-string key + custom value)
//!   smsg  string→message : serde derive
//!   senum string→enum    : `map_enum`
//!
//! The checks below pin the field types and the binary / JSON / text / view→owned
//! / reflection round-trips. `MapStr` is `Hash + Eq + Ord` (map key) and
//! derives serde + (under the feature) `Arbitrary`.

use buffa::{Map, Message};
use buffa_test::string_map::{Color, Inner, MapStr, Maps};

fn s(v: &str) -> MapStr {
    MapStr(v.to_string())
}

fn sample() -> Maps {
    let mut ss = Map::default();
    ss.insert(s("alpha"), s("first"));
    ss.insert(s("beta"), s("second"));

    let mut si64 = Map::default();
    si64.insert(s("big"), 9_000_000_000i64);
    si64.insert(s("neg"), -42i64);

    let mut sf64 = Map::default();
    sf64.insert(s("half"), 0.5f64);

    let mut i32s = Map::default();
    i32s.insert(7i32, s("seven"));
    i32s.insert(-1i32, s("minus-one"));

    let mut smsg = Map::default();
    smsg.insert(
        s("one"),
        Inner {
            v: 11,
            ..Default::default()
        },
    );

    let mut senum = Map::default();
    senum.insert(s("red"), buffa::EnumValue::Known(Color::COLOR_RED));
    senum.insert(s("blue"), buffa::EnumValue::Known(Color::COLOR_BLUE));

    Maps {
        ss,
        si64,
        sf64,
        i32s,
        smsg,
        senum,
        ..Default::default()
    }
}

#[test]
fn field_types_use_custom_string() {
    // Fails to compile if codegen emitted `String` for any custom `string` map
    // slot. The key type is the custom `MapStr` (so it had to be `Hash + Eq`).
    let m = Maps::default();
    let _: &Map<MapStr, MapStr> = &m.ss;
    let _: &Map<MapStr, i64> = &m.si64;
    let _: &Map<MapStr, f64> = &m.sf64;
    let _: &Map<i32, MapStr> = &m.i32s;
    let _: &Map<MapStr, Inner> = &m.smsg;
    let _: &Map<MapStr, buffa::EnumValue<Color>> = &m.senum;
}

#[test]
fn binary_round_trip() {
    let msg = sample();
    let bytes = msg.encode_to_vec();
    let decoded = Maps::decode(&mut bytes.as_slice()).expect("decode");
    assert_eq!(decoded, msg);
    // The `ProtoStringMap` codec decoded both key and value into `MapStr`.
    assert_eq!(decoded.ss.get(&s("alpha")), Some(&s("first")));
    assert_eq!(decoded.si64.get(&s("big")), Some(&9_000_000_000));
    assert_eq!(decoded.i32s.get(&7), Some(&s("seven")));
}

#[test]
fn json_round_trip() {
    let msg = sample();
    let json = serde_json::to_string(&msg).expect("serialize");
    let back: Maps = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, msg);
}

#[test]
fn json_proto_str_key_map_shape() {
    // `proto_str_key_map`: the custom string key is a JSON object key, and the
    // int64 value is proto3-JSON quoted. This is the path `proto_map` could not
    // serve (its key bound is `Display + FromStr`, which `MapStr` lacks).
    let msg = sample();
    let v: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&msg).unwrap()).expect("parse");
    assert_eq!(v["si64"]["big"], serde_json::json!("9000000000"));
    assert_eq!(v["si64"]["neg"], serde_json::json!("-42"));
    // A custom string key + string value (derive path) stays a plain object.
    assert_eq!(v["ss"]["alpha"], serde_json::json!("first"));
}

#[test]
fn view_to_owned_round_trip() {
    // The view yields borrowed `&str` keys/values; `to_owned_message` converts
    // each into `MapStr` via its `From<String>` and `.collect()`s the map.
    let bytes = bytes::Bytes::from(sample().encode_to_vec());
    let owned: Maps = buffa_test::string_map::MapsOwnedView::decode(bytes)
        .expect("decode view")
        .to_owned_message()
        .expect("to_owned");
    assert_eq!(owned, sample());
}

#[test]
fn text_round_trip() {
    let msg = sample();
    let text = buffa::text::encode_to_string(&msg);
    let back: Maps = buffa::text::decode_from_str(&text).expect("decode text");
    assert_eq!(back, msg);
}

#[test]
fn reflect_map_key_and_value() {
    use buffa_descriptor::reflect::{Reflectable, ValueRef};

    let msg = sample();
    let r = msg.reflect();
    let md = r.message_descriptor();

    // Field 1 (`ss`) is a map: the vtable `ReflectMap` walks `MapStr` keys
    // (through the emitted `ReflectMapKey`) and values (`ReflectElement`).
    let ValueRef::Map(map) = r.get(md.field(1).unwrap()) else {
        panic!("expected Map for ss");
    };
    assert_eq!(map.len(), 2);
}

#[cfg(feature = "arbitrary")]
#[test]
fn arbitrary_builds_custom_string_map() {
    use arbitrary::{Arbitrary, Unstructured};

    // The struct derives `Arbitrary`; each custom-string map slot requires the
    // newtype's own `Arbitrary` impl (the map path has no per-key shim). Build
    // one and touch the custom-keyed field to pin the derive ran.
    let raw: [u8; 256] = core::array::from_fn(|i| i as u8);
    let mut u = Unstructured::new(&raw);
    let msg = Maps::arbitrary(&mut u).unwrap();
    for k in msg.ss.keys() {
        let _: &str = k.as_ref();
    }
}

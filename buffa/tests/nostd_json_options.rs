#![cfg(not(feature = "std"))]

use buffa::json::{set_global_json_parse_options, JsonParseOptions};
use buffa::json_helpers::{map_enum, opt_enum, repeated_enum};
use buffa::{EnumValue, Enumeration, Map};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Color {
    Red,
    Green,
}

impl Enumeration for Color {
    fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Red),
            1 => Some(Self::Green),
            _ => None,
        }
    }

    fn to_i32(&self) -> i32 {
        match self {
            Self::Red => 0,
            Self::Green => 1,
        }
    }

    fn proto_name(&self) -> &'static str {
        match self {
            Self::Red => "RED",
            Self::Green => "GREEN",
        }
    }

    fn from_proto_name(name: &str) -> Option<Self> {
        match name {
            "RED" => Some(Self::Red),
            "GREEN" => Some(Self::Green),
            _ => None,
        }
    }
}

#[test]
fn global_lenient_option_filters_open_enum_containers() {
    set_global_json_parse_options(
        &JsonParseOptions::new().ignore_unknown_enum_values(true),
    );

    let json = r#"["RED","UNKNOWN",99,true]"#;
    let repeated: Vec<EnumValue<Color>> =
        repeated_enum::deserialize(&mut serde_json::Deserializer::from_str(json)).unwrap();
    assert_eq!(
        repeated,
        vec![EnumValue::Known(Color::Red), EnumValue::Unknown(99)]
    );

    let json = r#"{"known":"GREEN","unknown_name":"UNKNOWN","unknown_number":99,"bad":true}"#;
    let map: Map<String, EnumValue<Color>> =
        map_enum::deserialize(&mut serde_json::Deserializer::from_str(json)).unwrap();
    assert_eq!(map.len(), 2);
    assert_eq!(map["known"], EnumValue::Known(Color::Green));
    assert_eq!(map["unknown_number"], EnumValue::Unknown(99));

    for json in [r#""UNKNOWN""#, "true"] {
        let optional: Option<EnumValue<Color>> =
            opt_enum::deserialize(&mut serde_json::Deserializer::from_str(json)).unwrap();
        assert_eq!(optional, None, "lenient mode must drop {json}");
    }

    let optional: Option<EnumValue<Color>> =
        opt_enum::deserialize(&mut serde_json::Deserializer::from_str("99")).unwrap();
    assert_eq!(optional, Some(EnumValue::Unknown(99)));
}

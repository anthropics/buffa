const NULL_VALUE_TYPE_NAME: &str = "google.protobuf.NullValue";
const VALUE_TYPE_NAME: &str = "google.protobuf.Value";

fn without_reference_prefix(type_name: Option<&str>) -> Option<&str> {
    type_name.map(|name| name.strip_prefix('.').unwrap_or(name))
}

/// Protobuf enum whose JSON value is null.
pub(crate) fn is_null_value_type_name(type_name: Option<&str>) -> bool {
    without_reference_prefix(type_name) == Some(NULL_VALUE_TYPE_NAME)
}

/// Protobuf types whose null representation carries a value.
pub(crate) fn null_is_valid_type_name(type_name: Option<&str>) -> bool {
    is_null_value_type_name(type_name)
        || without_reference_prefix(type_name) == Some(VALUE_TYPE_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn descriptor_and_generator_type_names_share_null_policy() {
        for name in ["google.protobuf.NullValue", ".google.protobuf.NullValue"] {
            assert!(is_null_value_type_name(Some(name)));
            assert!(null_is_valid_type_name(Some(name)));
        }
        for name in ["google.protobuf.Value", ".google.protobuf.Value"] {
            assert!(!is_null_value_type_name(Some(name)));
            assert!(null_is_valid_type_name(Some(name)));
        }
        for name in [
            None,
            Some(".other.NullValue"),
            Some("..google.protobuf.NullValue"),
        ] {
            assert!(!is_null_value_type_name(name));
            assert!(!null_is_valid_type_name(name));
        }
    }
}

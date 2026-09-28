// Example demonstrating compile-time checking for deterministic ordering

use visualsign::{
    assert_deterministic, DeterministicOrdering, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldTextV2,
};

// This function will only accept types that implement DeterministicOrdering
fn process_deterministic_type<T: DeterministicOrdering>(value: &T) -> Result<String, String> {
    // At compile time, this ensures T has deterministic ordering
    value.verify_deterministic_ordering()?;

    // Serialize to JSON
    serde_json::to_string(value).map_err(|e| e.to_string())
}

// Example of a type that would FAIL compile-time checking if it doesn't implement DeterministicOrdering
#[derive(serde::Serialize, serde::Deserialize)]
#[allow(dead_code)] // illustrative only; never constructed
struct BadType {
    field_b: String,
    field_a: String, // Note: fields are not alphabetically ordered in the struct
}

// If you uncomment the following line, it will fail at compile time:
// impl DeterministicOrdering for BadType {}
// This would fail because BadType doesn't have a custom Serialize implementation
// that ensures deterministic ordering

fn main() {
    // These will compile because they implement DeterministicOrdering
    let field = SignablePayloadField::TextV2 {
        common: SignablePayloadFieldCommon {
            fallback_text: "fallback".to_string(),
            label: "label".to_string(),
        },
        text_v2: SignablePayloadFieldTextV2 {
            text: "text".to_string(),
        },
    };
    let _result = process_deterministic_type(&field);

    // Use const assertion at compile time
    const _: visualsign::StaticAssertDeterministic<SignablePayloadField> =
        assert_deterministic::<SignablePayloadField>();

    // This would fail at compile time if uncommented (BadType doesn't implement DeterministicOrdering):
    // let bad = BadType { field_b: "b".into(), field_a: "a".into() };
    // let _result = process_deterministic_type(&bad);  // COMPILE ERROR!

    println!("All types passed compile-time deterministic ordering checks!");
}

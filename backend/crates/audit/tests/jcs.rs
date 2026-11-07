//! RFC 8785 canonicalization behaviour used for audit row hashing.

use rs_admin_audit::canonical_json;
use serde_json::json;

#[test]
fn keys_are_sorted_and_numbers_follow_jcs() {
    let value = json!({
        "b": 1,
        "a": 2,
        "nested": {"z": true, "a": null},
        "big": 1e30,
        "small": 1e-7,
        "fraction": 0.5
    });

    assert_eq!(
        canonical_json(&value).expect("canonical"),
        r#"{"a":2,"b":1,"big":1e+30,"fraction":0.5,"nested":{"a":null,"z":true},"small":1e-7}"#
    );
}

#[test]
fn keys_are_sorted_by_utf16_code_units() {
    // RFC 8785 Appendix B ordering: the surrogate pair for the emoji sorts
    // before U+FB33 even though its code point is larger.
    let value = json!({
        "\u{20ac}": "Euro Sign",
        "\r": "Carriage Return",
        "\u{fb33}": "Hebrew Letter Dalet With Dagesh",
        "1": "One",
        "\u{1f600}": "Emoji: Grinning Face",
        "\u{0080}": "Control",
        "\u{00f6}": "Latin Small Letter O With Diaeresis"
    });

    assert_eq!(
        canonical_json(&value).expect("canonical"),
        "{\"\\r\":\"Carriage Return\",\"1\":\"One\",\"\u{0080}\":\"Control\",\
         \"\u{00f6}\":\"Latin Small Letter O With Diaeresis\",\"\u{20ac}\":\"Euro Sign\",\
         \"\u{1f600}\":\"Emoji: Grinning Face\",\"\u{fb33}\":\"Hebrew Letter Dalet With Dagesh\"}"
    );
}

#[test]
fn strings_are_escaped_per_jcs() {
    let value = json!({"s": "line\nbreak\u{0001}quote\"slash\\"});
    assert_eq!(
        canonical_json(&value).expect("canonical"),
        r#"{"s":"line\nbreak\u0001quote\"slash\\"}"#
    );
}

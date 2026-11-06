use super::*;

#[test]
fn an_id_is_1_to_255_of_jmaps_alphabet() {
    let (longest, long) = ("a".repeat(255), "a".repeat(256));
    for valid in ["a", "Ab_9-z", &longest] {
        assert_eq!(id(valid).unwrap(), valid);
    }
    for invalid in ["", "a.b", "a/b", "a%2Fb", "@evil.com", "é", &long] {
        assert!(id(invalid).is_err(), "{invalid}");
    }
    assert_eq!(id("a\nb").unwrap_err().to_string(), r"a\nb is not a JMAP id");
}

#[test]
fn a_template_takes_each_value_encoded_in_its_own_place() {
    let download =
        "https://mail.example.org/jmap/download/{accountId}/{blobId}/{name}?accept={type}";
    let values = [
        ("accountId", "a"),
        ("blobId", "B_1"),
        ("name", "a b/../c"),
        ("type", "application/octet-stream"),
    ];
    assert_eq!(
        fill(download, &values),
        "https://mail.example.org/jmap/download/a/B_1/a%20b%2F..%2Fc?accept=application%2Foctet-stream"
    );
    assert_eq!(fill("https://h/{x}", &[("x", "é?#@~")]), "https://h/%C3%A9%3F%23%40~");
    assert_eq!(fill("https://h/{x}{y}", &[("x", "{y}"), ("y", "2")]), "https://h/%7By%7D2");
    assert_eq!(fill("https://h/{x}/{y}", &[("x", "1")]), "https://h/1/{y}");
}

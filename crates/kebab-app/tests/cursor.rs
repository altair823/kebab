//! p9-fb-34: cursor encode/decode round-trip + corpus_revision mismatch.

use kebab_app::cursor;

#[test]
fn cursor_roundtrip_preserves_offset() {
    let encoded = cursor::encode(5, "rev-abc");
    let offset = cursor::decode(&encoded, "rev-abc").unwrap();
    assert_eq!(offset, 5);
}

#[test]
fn cursor_decode_rejects_mismatched_revision() {
    let encoded = cursor::encode(7, "rev-old");
    let err = cursor::decode(&encoded, "rev-new").unwrap_err();
    assert_eq!(err.code, "stale_cursor");
    assert!(err.message.contains("rev-old") || err.message.contains("rev-new"));
}

#[test]
fn cursor_decode_rejects_garbage_input() {
    let err = cursor::decode("not-base64!!!", "any").unwrap_err();
    assert_eq!(err.code, "stale_cursor");
}

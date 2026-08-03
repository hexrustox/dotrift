mod common;

use std::collections::HashMap;

use common::MockRegistry;
use proptest::prelude::*;
use templater::Template;

/// Escapes a byte payload so it can be placed inside a template string literal.
///
/// Each `"` becomes `\"` and each `\\` becomes `\\\\`. All other bytes pass
/// through unchanged. Rendering the resulting literal should reproduce the
/// original `bytes` exactly.
fn string_literal_safe_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut escaped = Vec::with_capacity(bytes.len());
    for &b in bytes {
        if b == b'"' || b == b'\\' {
            escaped.push(b'\\');
        }
        escaped.push(b);
    }
    escaped
}

proptest! {
    // Generate arbitrary byte buffers up to 1 KiB. Delimiters, invalid UTF-8,
    // embedded NULs, and binary garbage are all fair game: the engine must
    // reject malformed input with an error, never panic.
    #[test]
    fn random_bytes_do_not_panic(src in proptest::collection::vec(any::<u8>(), 0..1024)) {
        let template = Template::from_bytes(src.clone());
        let mut out = Vec::new();
        let _ = template.render(&mut out, &HashMap::new(), &MockRegistry);
    }

    // A string literal whose special bytes have been escaped renders back to the
    // original payload: escape processing inverts the escaping applied by
    // `literal_safe_bytes`, and top-level strings are emitted verbatim.
    #[test]
    fn string_literal_renders_identity(bytes in proptest::collection::vec(any::<u8>(), 0..1024)) {
        let escaped = string_literal_safe_bytes(&bytes);

        let mut src = Vec::with_capacity(4 + escaped.len() + 4);
        src.extend_from_slice(br#"{{ ""#);
        src.extend_from_slice(&escaped);
        src.extend_from_slice(br#"" }}"#);

        let mut out = Vec::new();
        Template::from_bytes(src)
            .render(&mut out, &HashMap::new(), &MockRegistry)
            .unwrap();

        assert_eq!(out, bytes);
    }

    // Any i64 integer literal renders as its canonical decimal form.
    #[test]
    fn int_literal_renders_identity(n in any::<i64>()) {
        let mut src = Vec::new();
        src.extend_from_slice(b"{{ ");
        src.extend_from_slice(n.to_string().as_bytes());
        src.extend_from_slice(b" }}");

        let mut out = Vec::new();
        Template::from_bytes(src)
            .render(&mut out, &HashMap::new(), &MockRegistry)
            .unwrap();

        assert_eq!(String::from_utf8(out).unwrap(), n.to_string());
    }

    #[test]
    fn bool_literal_renders_identity(bool in prop_oneof!["true", "false"]) {
        let mut src = Vec::new();
        src.extend_from_slice(b"{{ ");
        src.extend_from_slice(bool.as_bytes());
        src.extend_from_slice(b" }}");

        let mut out = Vec::new();
        Template::from_bytes(src)
            .render(&mut out, &HashMap::new(), &MockRegistry)
            .unwrap();

        assert_eq!(out, bool.as_bytes());
    }
}

mod common;

use std::collections::HashMap;

use common::MockRegistry;
use proptest::prelude::*;
use templater::Template;

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
}

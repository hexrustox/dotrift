use std::ops::Range;

/// A scanned region of the source. Ranges are byte offsets.
#[derive(Debug)]
pub(crate) enum Token {
    /// Plain text outside any tag.
    Text(Range<usize>),
}

/// Scans `src` into tokens. This slice recognizes no tags: the entire source
/// is plain text (passthrough).
pub(crate) fn scan(src: &[u8]) -> Vec<Token> {
    if src.is_empty() {
        Vec::new()
    } else {
        vec![Token::Text(0..src.len())]
    }
}

use std::ops::Range;

/// A parsed template node. Ranges are byte offsets into the source.
#[derive(Debug)]
pub(crate) enum Node {
    /// Plain text, emitted verbatim.
    Text(Range<usize>),
}

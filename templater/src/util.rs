use std::ops::Range;

use miette::SourceSpan;

/// Builds a miette `SourceSpan` from a byte range in source coordinates.
pub(crate) fn source_span(range: Range<usize>) -> SourceSpan {
    (range.start, range.end - range.start).into()
}

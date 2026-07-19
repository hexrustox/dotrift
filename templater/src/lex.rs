/// Byte classification shared by the scanner and parser.
///
/// Inner-edge ASCII whitespace inside a `{{ ... }}` body: space, tab, and
/// `\n` only. `\r` is intentionally *not* classified as whitespace — per
/// spec, only `\n` is a line terminator; `\r` is ordinary text and must
/// survive trimming so it is not silently lost in CRLF sources.
pub(crate) fn is_inner_ws(b: u8) -> bool {
    b == b' ' || b == b'\t' || b == b'\n'
}

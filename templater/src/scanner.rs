use std::ops::Range;

use crate::error::ParseError;

/// A scanned region of the source. Ranges are byte offsets.
#[derive(Debug, PartialEq)]
pub(crate) enum Token {
    /// Plain text (outside any recognized tag, plus all bytes inside tag
    /// kinds this slice does not yet recognize — `{% %}` / `{# #}`).
    Text(Range<usize>),
    /// `{{ ... }}` interpolation.
    Interp {
        /// Full tag span: `open .. close_end` (open is the `{{` position,
        /// close_end is one past the closing `}}`). Used by whole-tag
        /// errors such as `EmptyInterpolation`.
        tag: Range<usize>,
        /// Trimmed interior — inner-edge ASCII whitespace (` `, `\t`,
        /// `\n`; not `\r`) stripped by the scanner.
        body: Range<usize>,
    },
}

/// Scans `src` into tokens. This slice recognizes only `{{ }}`; the other
/// four delimiters (`{%`, `%}`, `{#`, `#}`), escapes, and modifiers are
/// inert — they render as plain text.
///
/// The scanner trims inner-edge ASCII whitespace of an interpolation body
/// before emitting it, and stashes the full `{{ ... }}` span alongside
/// the trimmed body so whole-tag error spans (e.g. `EmptyInterpolation`)
/// can cover the full delimiter pair even when the body collapses to
/// zero width.
///
/// `}}` appearing inside a closed `"..."` literal within a tag body is
/// shielded: it does not close the tag. If a `"..."` opened inside a tag
/// body has no matching close, the fallback is byte-by-byte scanning from
/// the opening `"`, so the tag still picks up the next unescaped `}}`
/// (the parser then reports `UnclosedString`). An *unclosed* `{{`
/// (no matching `}}` anywhere) raises `UnclosedDelimiter`.
pub(crate) fn scan(src: &[u8]) -> std::result::Result<Vec<Token>, ParseError> {
    let mut tokens = Vec::new();
    let mut pos = 0;

    while pos < src.len() {
        let Some(open) = find_subslice(src, pos, b"{{") else {
            push_text(&mut tokens, pos..src.len());
            break;
        };

        // Bytes strictly between the previous token and `{{` are plain text
        // (including lone `{` not followed by `{`).
        if open > pos {
            push_text(&mut tokens, pos..open);
        }

        let body_start = open + 2;
        let (body_end, found_close) = find_close(src, body_start);
        if !found_close {
            return Err(ParseError::UnclosedDelimiter {
                span: (open, 2).into(),
            });
        }

        let close_end = body_end + 2; // one past the closing `}}`
        let body = trim_inner_ws(src, body_start, body_end);
        tokens.push(Token::Interp {
            tag: open..close_end,
            body,
        });
        pos = close_end;
    }

    Ok(tokens)
}

fn push_text(tokens: &mut Vec<Token>, range: Range<usize>) {
    if range.start >= range.end {
        return;
    }
    if let Some(Token::Text(last)) = tokens.last_mut() {
        last.end = range.end;
    } else {
        tokens.push(Token::Text(range));
    }
}

fn find_subslice(src: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if from + needle.len() > src.len() {
        return None;
    }
    src[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| from + p)
}

/// Walks `src` from `start` searching for `}}`, honoring string literals
/// in the tag body. A `"` byte begins a string literal — we try to find
/// its matching `"` (with `\"` / `\\` escape skipping). If found, we jump
/// past it (the shielded `}}` inside is ignored). If not found, the
/// opening `"` is treated as an ordinary byte and scanning continues
/// byte-by-byte — so the next `}}` still closes the tag (the parser will
/// then report `UnclosedString` on the unterminated literal). Returns
/// `(position_of_}}, found)`. If `!found`, returns `(src.len(), false)`.
fn find_close(src: &[u8], start: usize) -> (usize, bool) {
    let mut i = start;
    while i + 2 <= src.len() {
        let b = src[i];
        if b == b'"' {
            if let Some(close) = find_string_end(src, i + 1) {
                i = close + 1;
                continue;
            }
            // Unclosed string: treat `"` as ordinary byte, keep scanning.
            i += 1;
            continue;
        }
        if b == b'}' && src[i + 1] == b'}' {
            return (i, true);
        }
        i += 1;
    }
    (src.len(), false)
}

/// Returns the absolute position of the closing `"` for a string literal
/// that opened at `from - 1` (i.e., the byte after the opening `"` is
/// `from`). Honors `\"` and `\\` escapes (the next byte is skipped).
/// Returns `None` if no closing `"` exists before EOF.
fn find_string_end(src: &[u8], from: usize) -> Option<usize> {
    let mut j = from;
    while j < src.len() {
        match src[j] {
            b'\\' if j + 1 < src.len() => j += 2,
            b'"' => return Some(j),
            _ => j += 1,
        }
    }
    None
}

/// Trims ASCII whitespace (` `, `\t`, `\n`; not `\r`) from the inner edges
/// of a tag body. Per spec: only `\n` is a line terminator — `\r` is
/// ordinary text and does not get trimmed.
fn trim_inner_ws(src: &[u8], body_start: usize, body_end: usize) -> Range<usize> {
    let mut start = body_start;
    let mut end = body_end;
    while start < end && is_inner_ws(src[start]) {
        start += 1;
    }
    while end > start && is_inner_ws(src[end - 1]) {
        end -= 1;
    }
    start..end
}

fn is_inner_ws(b: u8) -> bool {
    b == b' ' || b == b'\t' || b == b'\n'
}

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use super::*;

    fn text(r: Range<usize>) -> Token {
        Token::Text(r)
    }
    fn interp(tag: Range<usize>, body: Range<usize>) -> Token {
        Token::Interp { tag, body }
    }

    // `body` is the trimmed interior; `tag` is `open .. close_end`.
    #[test_case(b"" => Vec::<Token>::new(); "empty")]
    #[test_case(b"hello" => vec![text(0..5)]; "plain_text")]
    #[test_case(b"{{ x }}" => vec![interp(0..7, 3..4)]; "interp_padded")]
    #[test_case(b"{{x}}" => vec![interp(0..5, 2..3)]; "interp_tight")]
    #[test_case(b"{{   x   }}" => vec![interp(0..11, 5..6)]; "interp_heavy_padding")]
    #[test_case(b"{{}}" => vec![interp(0..4, 2..2)]; "interp_empty")]
    #[test_case(b"{{  }}" => vec![interp(0..6, 4..4)]; "interp_whitespace_only")]
    #[test_case(b"{{\nx\n}}" => vec![interp(0..7, 3..4)]; "interp_newline_in_body")]
    #[test_case(b"{{\tx}}" => vec![interp(0..6, 3..4)]; "interp_tab_in_body")]
    #[test_case(b"a{{ x }}b" => vec![text(0..1), interp(1..8, 4..5), text(8..9)]; "interp_between_text")]
    #[test_case(b"{{ x }}{{ y }}" => vec![interp(0..7, 3..4), interp(7..14, 10..11)]; "two_interps")]
    #[test_case(br#"{{ "}}" }}"# => vec![interp(0..10, 3..7)]; "string_shields_close_delim")]
    #[test_case(br#"{{ "\" }}"# => vec![interp(0..9, 3..6)]; "escaped_quote_in_string")]
    #[test_case(br#"{{ "a\nb" }}"# => vec![interp(0..12, 3..9)]; "string_with_passthrough_escape")]
    #[test_case(b"{{ \"line1\nline2\" }}" => vec![interp(0..19, 3..16)]; "string_with_raw_newline")]
    #[test_case(b"{ not a tag" => vec![text(0..11)]; "single_brace_passthrough")]
    #[test_case(b"{% if %}" => vec![text(0..8)]; "stmt_delimiters_inert")]
    #[test_case(b"{# note #}" => vec![text(0..10)]; "comment_delimiters_inert")]
    #[test_case(b"{{ x }}}}" => vec![interp(0..7, 3..4), text(7..9)]; "extra_close_is_text")]
    fn scan_cases(input: &[u8]) -> Vec<Token> {
        scan(input).unwrap()
    }

    #[test_case(b"{{ name" => (0, 2); "unclosed_no_text_after")]
    #[test_case(b"prefix {{ name" => (7, 2); "unclosed_after_text")]
    #[test_case(br#"{{ "unclosed"# => (0, 2); "unclosed_with_string_literal")]
    fn unclosed_returns_open_span(input: &[u8]) -> (usize, usize) {
        let ParseError::UnclosedDelimiter { span } = scan(input).unwrap_err() else {
            unreachable!("expected UnclosedDelimiter")
        };
        (span.offset(), span.len())
    }
}

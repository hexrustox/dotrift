use std::ops::Range;

use crate::error::{Error, ParseError};
use crate::util::{is_whitespace, source_span};

/// A whitespace-control sigil attached to an interpolation or statement
/// delimiter. Comments never carry modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Modifier {
    None,
    Dash,
    Equal,
}

/// A scanned region of the source. Ranges are byte offsets.
#[derive(Debug, PartialEq)]
pub(crate) enum Token {
    /// Plain text outside any recognized tag, plus literal output from escaped
    /// delimiter clusters.
    Text(Range<usize>),
    /// `{{ ... }}` interpolation.
    Interp {
        tag: Range<usize>,
        body: Range<usize>,
        left: Modifier,
        right: Modifier,
    },
    /// `{% ... %}` statement.
    Stmt {
        tag: Range<usize>,
        body: Range<usize>,
        left: Modifier,
        right: Modifier,
    },
    /// A comment delimiter position, stripped of all content. Used only as a
    /// whitespace-control barrier; its position is inferred from the
    /// surrounding token stream.
    Barrier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClusterKind {
    OpenInterp,
    OpenStmt,
    OpenComment,
    CloseInterp,
    CloseStmt,
    CloseComment,
}

impl ClusterKind {
    fn is_close(self) -> bool {
        matches!(
            self,
            ClusterKind::CloseInterp | ClusterKind::CloseStmt | ClusterKind::CloseComment
        )
    }
}

#[derive(Debug, Clone, Copy)]
struct Cluster {
    core: usize,
    kind: ClusterKind,
    /// Start of the active sequence, including a closing-side modifier. For
    /// opening delimiters this equals `core`; for a closing delimiter with a
    /// modifier it is one byte earlier.
    seq_start: usize,
    /// Number of consecutive backslashes immediately before `seq_start`.
    backslashes: usize,
}

impl Cluster {
    fn escaped(&self) -> bool {
        self.backslashes % 2 == 1
    }
}

/// Scans `src` into tokens. Recognizes all six delimiters, applies the
/// odd/even backslash escape rule, handles whitespace-control modifiers on
/// interpolation and statement tags, and strips comments while leaving
/// barrier markers for downstream `=` trimming.
pub(crate) fn scan(src: &[u8]) -> std::result::Result<Vec<Token>, Error> {
    let mut tokens = Vec::new();
    let mut text_start = 0;

    while let Some(cluster) = find_next_cluster(src, text_start) {
        if cluster.escaped() {
            emit_escaped_cluster(&mut tokens, &mut text_start, cluster);
            continue;
        }

        // Active delimiter: emit surviving literal backslashes before it.
        let literal_backslashes = cluster.backslashes / 2;
        let text_end = cluster.seq_start - literal_backslashes;
        if text_end > text_start {
            push_text(&mut tokens, text_start..text_end);
        }

        match cluster.kind {
            ClusterKind::OpenInterp | ClusterKind::OpenStmt => {
                let (left, body_start) = parse_left_modifier(src, cluster)?;
                let open_pos = cluster.core;
                let (close_core, right) = scan_body(src, body_start, cluster.kind, open_pos)?;

                let close_seq_start = if right == Modifier::None {
                    close_core
                } else {
                    close_core - 1
                };
                let close_end = close_core + 2;
                let body = trim_inner_ws(src, body_start, close_seq_start);
                let tag = open_pos..close_end;

                if cluster.kind == ClusterKind::OpenInterp {
                    tokens.push(Token::Interp {
                        tag,
                        body,
                        left,
                        right,
                    });
                } else {
                    tokens.push(Token::Stmt {
                        tag,
                        body,
                        left,
                        right,
                    });
                }
                text_start = close_end;
            }
            ClusterKind::OpenComment => {
                let open_pos = cluster.core;
                if src.len() > open_pos + 2 && is_modifier(src[open_pos + 2]) {
                    return Err(Error::parse(
                        ParseError::InvalidModifier,
                        source_span(open_pos + 2..open_pos + 3),
                    ));
                }
                let body_start = open_pos + 2;
                let close_end = scan_comment_body(src, body_start, open_pos)?;
                tokens.push(Token::Barrier);
                text_start = close_end;
            }
            ClusterKind::CloseInterp | ClusterKind::CloseStmt | ClusterKind::CloseComment => {
                return Err(Error::parse(
                    ParseError::StrayDelimiter,
                    source_span(cluster.core..cluster.core + 2),
                ));
            }
        }
    }

    // Trailing plain text after the last cluster.
    push_text(&mut tokens, text_start..src.len());
    Ok(tokens)
}

fn emit_escaped_cluster(tokens: &mut Vec<Token>, text_start: &mut usize, cluster: Cluster) {
    let kept_backslashes = (cluster.backslashes - 1) / 2;
    let literal_start = cluster.seq_start - kept_backslashes;
    let literal_end = cluster.core + 2;

    // Text before the consumed backslashes.
    let dropped_start = cluster.seq_start - cluster.backslashes;
    if dropped_start > *text_start {
        push_text(tokens, *text_start..dropped_start);
    }

    // Literal backslashes + delimiter (and, for an escaped closing sequence,
    // the unprocessed modifier byte).
    push_text(tokens, literal_start..literal_end);

    *text_start = cluster.core + 2;
}

fn parse_left_modifier(
    src: &[u8],
    cluster: Cluster,
) -> std::result::Result<(Modifier, usize), Error> {
    let after_delim = cluster.core + 2;
    if after_delim < src.len() && is_modifier(src[after_delim]) {
        Ok((modifier(src[after_delim]), after_delim + 1))
    } else {
        Ok((Modifier::None, after_delim))
    }
}

// -----------------------------------------------------------------------------
// Cluster finding

fn find_next_cluster(src: &[u8], from: usize) -> Option<Cluster> {
    let mut i = from;
    while i + 2 <= src.len() {
        let Some((core, kind)) = classify_opening(src, i).or_else(|| classify_closing(src, i))
        else {
            i += 1;
            continue;
        };

        let seq_start = if kind.is_close() && core > 0 && is_modifier(src[core - 1]) {
            core - 1
        } else {
            core
        };
        let backslashes = count_backslashes(src, seq_start);
        return Some(Cluster {
            core,
            kind,
            seq_start,
            backslashes,
        });
    }
    None
}

fn classify_opening(src: &[u8], i: usize) -> Option<(usize, ClusterKind)> {
    if src[i] != b'{' {
        return None;
    }
    match src.get(i + 1)? {
        b'{' => Some((i, ClusterKind::OpenInterp)),
        b'%' => Some((i, ClusterKind::OpenStmt)),
        b'#' => Some((i, ClusterKind::OpenComment)),
        _ => None,
    }
}

fn classify_closing(src: &[u8], i: usize) -> Option<(usize, ClusterKind)> {
    match src[i] {
        b'}' if src.get(i + 1) == Some(&b'}') => Some((i, ClusterKind::CloseInterp)),
        b'%' if src.get(i + 1) == Some(&b'}') => Some((i, ClusterKind::CloseStmt)),
        b'#' if src.get(i + 1) == Some(&b'}') => Some((i, ClusterKind::CloseComment)),
        _ => None,
    }
}

fn is_modifier(b: u8) -> bool {
    b == b'-' || b == b'='
}

fn modifier(b: u8) -> Modifier {
    if b == b'-' {
        Modifier::Dash
    } else {
        Modifier::Equal
    }
}

/// Counts consecutive `\` bytes immediately before `pos`.
fn count_backslashes(src: &[u8], pos: usize) -> usize {
    let mut n = 0;
    while pos > n && src[pos - 1 - n] == b'\\' {
        n += 1;
    }
    n
}

// -----------------------------------------------------------------------------
// Tag body scanning

/// Scans an interpolation or statement body from `body_start` to its first
/// unescaped matching closing delimiter. Returns `(close_core_start, right_mod)`.
fn scan_body(
    src: &[u8],
    body_start: usize,
    open_kind: ClusterKind,
    open_pos: usize,
) -> std::result::Result<(usize, Modifier), Error> {
    debug_assert!(matches!(
        open_kind,
        ClusterKind::OpenInterp | ClusterKind::OpenStmt
    ));

    let mut i = body_start;
    while i + 2 <= src.len() {
        if src[i] == b'"' {
            if let Some(close) = find_string_end(src, i + 1) {
                i = close + 1;
                continue;
            }
            // Unclosed string: treat the opening quote as an ordinary byte and
            // keep scanning. The parser will report UnclosedString later.
            i += 1;
            continue;
        }

        let close_kind = match open_kind {
            ClusterKind::OpenInterp => ClusterKind::CloseInterp,
            ClusterKind::OpenStmt => ClusterKind::CloseStmt,
            _ => unreachable!(),
        };
        let b = src[i];
        let core = match close_kind {
            ClusterKind::CloseInterp if b == b'}' && src[i + 1] == b'}' => i,
            ClusterKind::CloseStmt if b == b'%' && src[i + 1] == b'}' => i,
            _ => {
                i += 1;
                continue;
            }
        };

        let (seq_start, mod_byte) = if core > 0 && is_modifier(src[core - 1]) {
            (core - 1, Some(src[core - 1]))
        } else {
            (core, None)
        };
        let backslashes = count_backslashes(src, seq_start);
        if backslashes % 2 == 1 {
            // Escaped close: skip the core and keep scanning.
            i = core + 2;
            continue;
        }

        let right = mod_byte.map(modifier).unwrap_or(Modifier::None);
        return Ok((core, right));
    }

    Err(Error::parse(
        ParseError::UnclosedDelimiter,
        source_span(open_pos..open_pos + 2),
    ))
}

/// Returns the absolute position of the closing `"` for a string literal that
/// opened just before `from`. Honors `\"` and `\\` escapes.
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

// -----------------------------------------------------------------------------
// Comment scanning

fn scan_comment_body(
    src: &[u8],
    body_start: usize,
    open_pos: usize,
) -> std::result::Result<usize, Error> {
    let mut i = body_start;
    while i + 2 <= src.len() {
        if src[i] == b'#' && src[i + 1] == b'}' {
            let backslashes = count_backslashes(src, i);
            if backslashes % 2 == 1 {
                // Escaped `#}`: literal text, skip it.
                i += 2;
                continue;
            }
            if i > 0 && is_modifier(src[i - 1]) {
                return Err(Error::parse(
                    ParseError::InvalidModifier,
                    source_span(i - 1..i),
                ));
            }
            return Ok(i + 2);
        }
        i += 1;
    }
    Err(Error::parse(
        ParseError::UnclosedDelimiter,
        source_span(open_pos..open_pos + 2),
    ))
}

// -----------------------------------------------------------------------------
// Helpers

fn push_text(tokens: &mut Vec<Token>, range: Range<usize>) {
    if range.start >= range.end {
        return;
    }
    if let Some(Token::Text(last)) = tokens.last_mut()
        && last.end == range.start
    {
        last.end = range.end;
        return;
    }
    tokens.push(Token::Text(range));
}

/// Trims ASCII whitespace (` `, `\t`, `\n`; not `\r`) from the inner edges
/// of a tag body.
fn trim_inner_ws(src: &[u8], body_start: usize, body_end: usize) -> Range<usize> {
    let mut start = body_start;
    let mut end = body_end;
    while start < end && is_whitespace(src[start]) {
        start += 1;
    }
    while end > start && is_whitespace(src[end - 1]) {
        end -= 1;
    }
    start..end
}

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use super::*;

    macro_rules! text {
        ($range:expr) => {
            Token::Text($range)
        };
    }

    macro_rules! interp {
        ($tag:expr, $body:expr) => {
            Token::Interp {
                tag: $tag,
                body: $body,
                left: Modifier::None,
                right: Modifier::None,
            }
        };
        ($tag:expr, $body:expr, $left:ident, $right:ident) => {
            Token::Interp {
                tag: $tag,
                body: $body,
                left: Modifier::$left,
                right: Modifier::$right,
            }
        };
    }

    macro_rules! stmt {
        ($tag:expr, $body:expr) => {
            Token::Stmt {
                tag: $tag,
                body: $body,
                left: Modifier::None,
                right: Modifier::None,
            }
        };
        ($tag:expr, $body:expr, $left:ident, $right:ident) => {
            Token::Stmt {
                tag: $tag,
                body: $body,
                left: Modifier::$left,
                right: Modifier::$right,
            }
        };
    }

    #[test_case(b"" => Vec::<Token>::new(); "empty")]
    #[test_case(b"hello" => vec![text!(0..5)]; "plain_text")]
    #[test_case(b"{}" => vec![text!(0..2)]; "brackets")]
    #[test_case(b"{{x}}" => vec![interp!(0..5, 2..3)]; "basic_interpolation")]
    #[test_case(b"{{ x }}" => vec![interp!(0..7, 3..4)]; "interpolation_with_spaces")]
    #[test_case(b"{{  x  }}" => vec![interp!(0..9, 4..5)]; "interpolation_with_multiple_spaces")]
    #[test_case(b"{{x}}y{{z}}" => vec![interp!(0..5, 2..3), text!(5..6), interp!(6..11, 8..9)]; "adjacent_interpolations")]
    #[test_case(b"{%if x%}" => vec![stmt!(0..8, 2..6)]; "basic_statement")]
    #[test_case(b"{# c #}" => vec![Token::Barrier]; "comment_with_content")]
    #[test_case(b"a{#c#}b" => vec![text!(0..1), Token::Barrier, text!(6..7)]; "comment_between_text")]
    #[test_case(b"{##}" => vec![Token::Barrier]; "minimal_comment")]
    #[test_case(b"{{\nx\n}}" => vec![interp!(0..7, 3..4)]; "interpolation_with_newlines")]
    #[test_case(b"{{\tx\t}}" => vec![interp!(0..7, 3..4)]; "interpolation_with_tabs")]
    #[test_case(b"{{ x\r }}" => vec![interp!(0..8, 3..5)]; "carriage_return_survives")]
    #[test_case(b"{{\n x\n}}" => vec![interp!(0..8, 4..5)]; "multiline_interpolation")]
    #[test_case(b"{{-x-}}" => vec![interp!(0..7, 3..4, Dash, Dash)]; "dash_modifiers_tight")]
    #[test_case(b"{{- x -}}" => vec![interp!(0..9, 4..5, Dash, Dash)]; "dash_modifiers_with_spaces")]
    #[test_case(b"{{=x=}}" => vec![interp!(0..7, 3..4, Equal, Equal)]; "equal_modifiers_tight")]
    #[test_case(b"{{= x =}}" => vec![interp!(0..9, 4..5, Equal, Equal)]; "equal_modifiers_with_spaces")]
    #[test_case(b"{%-if x-%}" => vec![stmt!(0..10, 3..7, Dash, Dash)]; "statement_dash_modifiers")]
    #[test_case(b"{%=if x=%}" => vec![stmt!(0..10, 3..7, Equal, Equal)]; "statement_equal_modifiers")]
    #[test_case(b"{{-x}}" => vec![interp!(0..6, 3..4, Dash, None)]; "left_dash_only")]
    #[test_case(b"{{x-}}" => vec![interp!(0..6, 2..3, None, Dash)]; "right_dash_only")]
    #[test_case(b"{{=x}}" => vec![interp!(0..6, 3..4, Equal, None)]; "left_equal_only")]
    #[test_case(b"{{x=}}" => vec![interp!(0..6, 2..3, None, Equal)]; "right_equal_only")]
    #[test_case(b"{{- x =}}" => vec![interp!(0..9, 4..5, Dash, Equal)]; "dash_left_equal_right")]
    #[test_case(b"{{= x -}}" => vec![interp!(0..9, 4..5, Equal, Dash)]; "equal_left_dash_right")]
    #[test_case(b"{# a\nb #}" => vec![Token::Barrier]; "multiline_comment")]
    #[test_case(b"x{# c #}y" => vec![text!(0..1), Token::Barrier, text!(8..9)]; "comment_with_surrounding_text")]
    #[test_case(b"\\{{" => vec![text!(1..3)]; "escaped_opening_interpolation")]
    #[test_case(b"\\{{\\}}" => vec![text!(1..3), text!(4..6)]; "escaped_interpolation_pair")]
    #[test_case(b"\\\\{{x}}" => vec![text!(0..1), interp!(2..7, 4..5)]; "two_backslashes_then_tag")]
    #[test_case(b"\\\\\\\\{{x}}" => vec![text!(0..2), interp!(4..9, 6..7)]; "four_backslashes_then_tag")]
    #[test_case(b"\\\\\\{{" => vec![text!(2..5)]; "three_backslashes_escaped")]
    #[test_case(b"a\\{{b" => vec![text!(0..1), text!(2..5)]; "text_with_escaped_interpolation")]
    #[test_case(b"{{ \\{{ }}" => vec![interp!(0..9, 3..6)]; "escaped_opening_inside_interpolation")]
    #[test_case(b"{{ \\}} }}" => vec![interp!(0..9, 3..6)]; "escaped_closing_inside_interpolation")]
    #[test_case(b"{% \\{% %}" => vec![stmt!(0..9, 3..6)]; "escaped_opening_inside_statement")]
    #[test_case(b"{% \\%} %}" => vec![stmt!(0..9, 3..6)]; "escaped_closing_inside_statement")]
    #[test_case(b"{# \\#{ #}" => vec![Token::Barrier]; "escaped_opening_inside_comment")]
    #[test_case(b"{# \\#} #}" => vec![Token::Barrier]; "escaped_closing_inside_comment")]
    #[test_case(b"{{ \"}}\" }}" => vec![interp!(0..10, 3..7)]; "string_shields_closing_delimiter")]
    #[test_case(b"{% \"}}\" %}" => vec![stmt!(0..10, 3..7)]; "string_shields_in_statement")]
    fn scan_token(input: &[u8]) -> Vec<Token> {
        scan(input).unwrap()
    }

    #[test_case(b"}}" => (ParseError::StrayDelimiter, (0, 2)) ; "stray closing interpolation")]
    #[test_case(b"%}" => (ParseError::StrayDelimiter, (0, 2)) ; "stray closing statement")]
    #[test_case(b"#}" => (ParseError::StrayDelimiter, (0, 2)) ; "stray closing comment")]
    #[test_case(b"a}}" => (ParseError::StrayDelimiter, (1, 2)) ; "stray delimiter after text")]
    #[test_case(b"\\{{}}" => (ParseError::StrayDelimiter, (3, 2)) ; "stray close after escaped open")]
    #[test_case(b"{{" => (ParseError::UnclosedDelimiter, (0, 2)) ; "unclosed interpolation")]
    #[test_case(b"{{ x" => (ParseError::UnclosedDelimiter, (0, 2)) ; "unclosed interpolation with content")]
    #[test_case(b"{%" => (ParseError::UnclosedDelimiter, (0, 2)) ; "unclosed statement")]
    #[test_case(b"{#" => (ParseError::UnclosedDelimiter, (0, 2)) ; "unclosed comment")]
    #[test_case(b"{{ \"unterminated" => (ParseError::UnclosedDelimiter, (0, 2)) ; "unclosed string in interpolation")]
    #[test_case(b"{#- x #}" => (ParseError::InvalidModifier, (2, 1)) ; "modifier after opening comment")]
    #[test_case(b"{#= =#}" => (ParseError::InvalidModifier, (2, 1)) ; "equal modifier after opening comment")]
    #[test_case(b"{# x -#}" => (ParseError::InvalidModifier, (5, 1)) ; "modifier before closing comment")]
    fn scan_errors(input: &[u8]) -> (ParseError, (usize, usize)) {
        let Error::Parse { err, span } = scan(input).unwrap_err() else {
            panic!("expected parse error");
        };
        (err, (span.offset(), span.len()))
    }
}

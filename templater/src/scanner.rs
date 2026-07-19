use std::ops::Range;

use crate::error::ParseError;
use crate::lex::is_inner_ws;

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

    fn matching_close(self) -> Self {
        match self {
            ClusterKind::OpenInterp => ClusterKind::CloseInterp,
            ClusterKind::OpenStmt => ClusterKind::CloseStmt,
            ClusterKind::OpenComment => ClusterKind::CloseComment,
            _ => unreachable!("matching_close is only called on opening kinds"),
        }
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
pub(crate) fn scan(src: &[u8]) -> std::result::Result<Vec<Token>, ParseError> {
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
                let (left, body_start) = parse_left_modifier(src, cluster, cluster.kind)?;
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
                    return Err(ParseError::InvalidModifier {
                        span: (open_pos + 2, 1).into(),
                    });
                }
                let body_start = open_pos + 2;
                let close_end = scan_comment_body(src, body_start, open_pos)?;
                tokens.push(Token::Barrier);
                tokens.push(Token::Barrier);
                text_start = close_end;
            }
            ClusterKind::CloseInterp | ClusterKind::CloseStmt | ClusterKind::CloseComment => {
                return Err(ParseError::StrayDelimiter {
                    span: (cluster.core, 2).into(),
                });
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
    kind: ClusterKind,
) -> std::result::Result<(Modifier, usize), ParseError> {
    let after_delim = cluster.core + 2;
    if after_delim < src.len() && is_modifier(src[after_delim]) {
        if kind == ClusterKind::OpenComment {
            return Err(ParseError::InvalidModifier {
                span: (after_delim, 1).into(),
            });
        }
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
) -> std::result::Result<(usize, Modifier), ParseError> {
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

        let close_kind = open_kind.matching_close();
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

    Err(ParseError::UnclosedDelimiter {
        span: (open_pos, 2).into(),
    })
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
) -> std::result::Result<usize, ParseError> {
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
                return Err(ParseError::InvalidModifier {
                    span: (i - 1, 2).into(),
                });
            }
            return Ok(i + 2);
        }
        i += 1;
    }
    Err(ParseError::UnclosedDelimiter {
        span: (open_pos, 2).into(),
    })
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
    while start < end && is_inner_ws(src[start]) {
        start += 1;
    }
    while end > start && is_inner_ws(src[end - 1]) {
        end -= 1;
    }
    start..end
}

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use super::*;

    fn text(r: Range<usize>) -> Token {
        Token::Text(r)
    }
    fn interp(tag: Range<usize>, body: Range<usize>) -> Token {
        Token::Interp {
            tag,
            body,
            left: Modifier::None,
            right: Modifier::None,
        }
    }
    fn stmt(tag: Range<usize>, body: Range<usize>) -> Token {
        Token::Stmt {
            tag,
            body,
            left: Modifier::None,
            right: Modifier::None,
        }
    }
    fn imod(tag: Range<usize>, body: Range<usize>, left: Modifier, right: Modifier) -> Token {
        Token::Interp {
            tag,
            body,
            left,
            right,
        }
    }
    fn smod(tag: Range<usize>, body: Range<usize>, left: Modifier, right: Modifier) -> Token {
        Token::Stmt {
            tag,
            body,
            left,
            right,
        }
    }

    // --- Basic tokenization -------------------------------------------------

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
    #[test_case(br#"{{ "a\xb" }}"# => vec![interp(0..12, 3..9)]; "string_with_passthrough_escape")]
    #[test_case(b"{{ \"line1\nline2\" }}" => vec![interp(0..19, 3..16)]; "string_with_raw_newline")]
    #[test_case(b"{ not a tag" => vec![text(0..11)]; "single_brace_passthrough")]
    #[test_case(b"{% if %}" => vec![stmt(0..8, 3..5)]; "stmt_delimiters")]
    #[test_case(b"{# note #}" => vec![Token::Barrier, Token::Barrier]; "comment_delimiters_stripped")]
    fn scan_cases(input: &[u8]) -> Vec<Token> {
        scan(input).unwrap()
    }

    // --- Escape rule table --------------------------------------------------

    fn backslashes(n: usize) -> Vec<u8> {
        std::iter::repeat_n(b'\\', n).collect()
    }

    fn build(prefix: &[u8], n: usize, suffix: &[u8]) -> Vec<u8> {
        let mut v = backslashes(n);
        v.extend_from_slice(prefix);
        v.extend_from_slice(suffix);
        v
    }

    #[test]
    fn escape_rule_table() {
        struct OpenCase {
            n: usize,
            active: bool,
            expected: Vec<Token>,
        }
        let open_cases = [
            OpenCase {
                n: 0,
                active: true,
                expected: vec![interp(0..5, 2..3)],
            },
            OpenCase {
                n: 1,
                active: false,
                expected: vec![text(1..4)],
            },
            OpenCase {
                n: 2,
                active: true,
                expected: vec![text(0..1), interp(2..7, 4..5)],
            },
            OpenCase {
                n: 3,
                active: false,
                expected: vec![text(2..6)],
            },
            OpenCase {
                n: 4,
                active: true,
                expected: vec![text(0..2), interp(4..9, 6..7)],
            },
        ];
        for case in open_cases {
            let src = if case.active {
                build(b"{{x}}", case.n, b"")
            } else {
                build(b"{{", case.n, b"x")
            };
            assert_eq!(scan(&src).unwrap(), case.expected, "open n={}", case.n);
        }

        struct CloseCase {
            n: usize,
            err: bool,
            expected: Vec<Token>,
        }
        let close_cases = [
            CloseCase {
                n: 0,
                err: true,
                expected: Vec::new(),
            },
            CloseCase {
                n: 1,
                err: false,
                expected: vec![text(1..3)],
            },
            CloseCase {
                n: 2,
                err: true,
                expected: Vec::new(),
            },
            CloseCase {
                n: 3,
                err: false,
                expected: vec![text(2..5)],
            },
            CloseCase {
                n: 4,
                err: true,
                expected: Vec::new(),
            },
        ];
        for case in close_cases {
            let src = build(b"}}", case.n, b"");
            let result = scan(&src);
            if case.err {
                assert!(
                    matches!(result, Err(ParseError::StrayDelimiter { .. })),
                    "close n={} should stray",
                    case.n
                );
            } else {
                assert_eq!(result.unwrap(), case.expected, "close n={}", case.n);
            }
        }
    }

    // --- Modifiers ----------------------------------------------------------

    #[test_case(b"{{-x-}}" => imod(0..7, 3..4, Modifier::Dash, Modifier::Dash); "dash_both_tight")]
    #[test_case(b"{{=x=}}" => imod(0..7, 3..4, Modifier::Equal, Modifier::Equal); "equal_both_tight")]
    #[test_case(b"{{-x=}}" => imod(0..7, 3..4, Modifier::Dash, Modifier::Equal); "dash_left_equal_right_tight")]
    #[test_case(b"{{=x-}}" => imod(0..7, 3..4, Modifier::Equal, Modifier::Dash); "equal_left_dash_right_tight")]
    #[test_case(b"{%-x-%}" => smod(0..7, 3..4, Modifier::Dash, Modifier::Dash); "stmt_dash_both_tight")]
    #[test_case(b"{{- x -}}" => imod(0..9, 4..5, Modifier::Dash, Modifier::Dash); "dash_both_padded")]
    #[test_case(b"{{= x =}}" => imod(0..9, 4..5, Modifier::Equal, Modifier::Equal); "equal_both_padded")]
    #[test_case(b"{{- x =}}" => imod(0..9, 4..5, Modifier::Dash, Modifier::Equal); "dash_left_equal_right_padded")]
    #[test_case(b"{{= x -}}" => imod(0..9, 4..5, Modifier::Equal, Modifier::Dash); "equal_left_dash_right_padded")]
    #[test_case(b"{%- x -%}" => smod(0..9, 4..5, Modifier::Dash, Modifier::Dash); "stmt_dash_both_padded")]
    fn modifier_combos(input: &[u8]) -> Token {
        let mut toks = scan(input).unwrap();
        assert_eq!(toks.len(), 1);
        toks.pop().unwrap()
    }

    // --- Delimiters inside string literals ----------------------------------

    #[test_case(br#"{% "%}" %}"# => stmt(0..10, 3..7); "stmt_string_shields_close")]
    #[test_case(br#"{{ "}}" }}"# => interp(0..10, 3..7); "interp_string_shields_close")]
    fn string_shielding(input: &[u8]) -> Token {
        let mut toks = scan(input).unwrap();
        assert_eq!(toks.len(), 1);
        toks.pop().unwrap()
    }

    // --- Errors -------------------------------------------------------------

    #[test_case(b"{{ name" => (0, 2); "unclosed_no_text_after")]
    #[test_case(b"prefix {{ name" => (7, 2); "unclosed_after_text")]
    #[test_case(br#"{{ "unclosed"# => (0, 2); "unclosed_with_string_literal")]
    fn unclosed_returns_open_span(input: &[u8]) -> (usize, usize) {
        let ParseError::UnclosedDelimiter { span } = scan(input).unwrap_err() else {
            unreachable!("expected UnclosedDelimiter")
        };
        (span.offset(), span.len())
    }

    #[test_case(b"}}" => (0, 2); "stray_close")]
    #[test_case(b"prefix }}" => (7, 2); "stray_close_after_text")]
    #[test_case(b"\\{{}}" => (3, 2); "escaped_open_unescaped_close")]
    fn stray_delimiter_span(input: &[u8]) -> (usize, usize) {
        let ParseError::StrayDelimiter { span } = scan(input).unwrap_err() else {
            unreachable!("expected StrayDelimiter")
        };
        (span.offset(), span.len())
    }

    #[test_case(b"{#= x #}" => (2, 1); "comment_open_modifier")]
    fn comment_modifier_span(input: &[u8]) -> (usize, usize) {
        let ParseError::InvalidModifier { span } = scan(input).unwrap_err() else {
            unreachable!("expected InvalidModifier")
        };
        (span.offset(), span.len())
    }
}

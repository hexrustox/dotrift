use std::ops::Range;

use crate::error::{Error, ParseError};
use crate::lex::is_inner_ws;
use crate::util::source_span;

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
            // ClusterKind::OpenComment => ClusterKind::CloseComment,
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
    use proptest::prelude::*;
    use test_case::{test_case, test_matrix};

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

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum MatrixVal {
        OpenInterp,
        CloseInterp,
        OpenStmt,
        CloseStmt,
        OpenComment,
        CloseComment,
        Dash,
        Equal,
    }

    impl MatrixVal {
        fn as_str(&self) -> &'static str {
            match self {
                MatrixVal::OpenInterp => "{{",
                MatrixVal::CloseInterp => "}}",
                MatrixVal::OpenStmt => "{%",
                MatrixVal::CloseStmt => "%}",
                MatrixVal::OpenComment => "{#",
                MatrixVal::CloseComment => "#}",
                MatrixVal::Dash => "-",
                MatrixVal::Equal => "=",
            }
        }

        fn modifier(&self) -> Modifier {
            match self {
                MatrixVal::Dash => Modifier::Dash,
                MatrixVal::Equal => Modifier::Equal,
                _ => Modifier::None,
            }
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
    #[test_case(b"{{\rx}}" => vec![interp(0..6, 2..4)]; "interp_cr_in_body")]
    #[test_case(b"{{\tx}}" => vec![interp(0..6, 3..4)]; "interp_tab_in_body")]
    #[test_case(b"a{{ x }}b" => vec![text(0..1), interp(1..8, 4..5), text(8..9)]; "interp_between_text")]
    #[test_case(b"{{ x }}{{ y }}" => vec![interp(0..7, 3..4), interp(7..14, 10..11)]; "two_interps")]
    #[test_case(br#"{{ "}}" }}"# => vec![interp(0..10, 3..7)]; "string_shields_close_delim")]
    #[test_case(br#"{{ "\" }}"# => vec![interp(0..9, 3..6)]; "escaped_quote_in_string")]
    #[test_case(br#"{{ "a\xb" }}"# => vec![interp(0..12, 3..9)]; "string_with_passthrough_escape")]
    #[test_case(b"{{ \"line1\nline2\" }}" => vec![interp(0..19, 3..16)]; "string_with_raw_newline")]
    #[test_case(b"{ not a tag" => vec![text(0..11)]; "single_brace_passthrough")]
    #[test_case(b"{% if %}" => vec![stmt(0..8, 3..5)]; "stmt_delimiters")]
    #[test_case(b"{# note #}" => vec![Token::Barrier]; "comment_delimiters_stripped")]
    #[test_case(b"\\{{" => vec![text(1..3)]; "backslash_escapes_open_delim")]
    #[test_case(b"\\}}" => vec![text(1..3)]; "backslash_escapes_close_delim")]
    #[test_case(b"\\\\{{}}" => vec![text(0..1), interp(2..6, 4..4)]; "escaped_backslash_then_empty_interp")]
    #[test_case(b"{{ \\}} }}" => vec![interp(0..9, 3..6)]; "backslash_escapes_close_within_interp")]
    fn scan_cases(input: &[u8]) -> Vec<Token> {
        scan(input).unwrap()
    }

    // --- Escape rule table --------------------------------------------------

    fn backslashes(n: usize) -> Vec<u8> {
        std::iter::repeat_n(b'\\', n).collect()
    }

    fn matching_delim(delim: MatrixVal) -> MatrixVal {
        match delim {
            MatrixVal::OpenInterp => MatrixVal::CloseInterp,
            MatrixVal::CloseInterp => MatrixVal::OpenInterp,
            MatrixVal::OpenStmt => MatrixVal::CloseStmt,
            MatrixVal::CloseStmt => MatrixVal::OpenStmt,
            MatrixVal::OpenComment => MatrixVal::CloseComment,
            MatrixVal::CloseComment => MatrixVal::OpenComment,
            _ => panic!("not a delimiter: {delim:?}"),
        }
    }

    fn count_backslash_bytes(src: &[u8]) -> usize {
        src.iter().filter(|&&b| b == b'\\').count()
    }

    fn trailing_backslashes(src: &[u8]) -> usize {
        src.iter().rev().take_while(|&&b| b == b'\\').count()
    }

    /// Counts literal backslashes represented by a token stream, accounting
    /// for the escape rule's consumption of half of an even run that precedes
    /// an active closing delimiter.
    fn literal_backslash_count(tokens: &[Token], src: &[u8]) -> usize {
        tokens
            .iter()
            .map(|t| match t {
                Token::Text(r) => count_backslash_bytes(&src[r.clone()]),
                Token::Interp { body, .. } | Token::Stmt { body, .. } => {
                    // In these test sources the body is only a (possibly empty)
                    // run of backslashes immediately before the closing
                    // delimiter; half of an even run are consumed as escapes.
                    let body_src = &src[body.clone()];
                    let trailing = trailing_backslashes(body_src);
                    (body_src.len() - trailing) + trailing / 2
                }
                Token::Barrier => 0,
            })
            .sum()
    }

    fn expected_literal_backslashes(delim: MatrixVal, n: usize) -> usize {
        match delim {
            // An active closing comment delimiter swallows the preceding
            // backslashes inside the comment, so they never become text.
            MatrixVal::CloseComment if n.is_multiple_of(2) => 0,
            _ => n / 2,
        }
    }

    proptest! {
        #[test]
        fn escape_rule_parity(n in 0usize..=255) {
            let prefix = b"prefix";
            for delim in [
                MatrixVal::OpenInterp,
                MatrixVal::CloseInterp,
                MatrixVal::OpenStmt,
                MatrixVal::CloseStmt,
                MatrixVal::OpenComment,
                MatrixVal::CloseComment,
            ] {
            let mut src = Vec::new();
            src.extend_from_slice(prefix);

            let is_open = matches!(
                delim,
                MatrixVal::OpenInterp | MatrixVal::OpenStmt | MatrixVal::OpenComment
            );

            if is_open {
                src.extend(backslashes(n));
                src.extend_from_slice(delim.as_str().as_bytes());
                if n.is_multiple_of(2) {
                    src.extend_from_slice(matching_delim(delim).as_str().as_bytes());
                }
            } else {
                if n.is_multiple_of(2) {
                    src.extend_from_slice(matching_delim(delim).as_str().as_bytes());
                }
                src.extend(backslashes(n));
                src.extend_from_slice(delim.as_str().as_bytes());
            }

            let tokens = scan(&src).unwrap();

            if n.is_multiple_of(2) {
                assert_eq!(
                    tokens.len(),
                    2,
                    "active delimiter should produce [text, tag]"
                );
                assert!(matches!(tokens[0], Token::Text(_)));
                match delim {
                    MatrixVal::OpenInterp | MatrixVal::CloseInterp => {
                        assert!(matches!(tokens[1], Token::Interp { .. }));
                    }
                    MatrixVal::OpenStmt | MatrixVal::CloseStmt => {
                        assert!(matches!(tokens[1], Token::Stmt { .. }));
                    }
                    MatrixVal::OpenComment | MatrixVal::CloseComment => {
                        assert_eq!(tokens[1], Token::Barrier);
                    }
                    _ => unreachable!(),
                }
            } else {
                assert!(
                    tokens.iter().all(|t| matches!(t, Token::Text(_))),
                    "escaped delimiter should render as plain text"
                );
            }

            assert_eq!(
                literal_backslash_count(&tokens, &src),
                expected_literal_backslashes(delim, n),
                "literal backslash count mismatch for {delim:?} n={n}"
            );
            }
        }
    }

    // --- Modifiers ----------------------------------------------------------

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ExpectedError {
        UnclosedDelimiter,
        StrayDelimiter,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Context {
        None,
        Prefix,
        Suffix,
    }

    #[test_matrix(
        [
            (MatrixVal::OpenInterp, MatrixVal::CloseInterp),
            (MatrixVal::OpenStmt, MatrixVal::CloseStmt),
        ],
        [MatrixVal::Dash, MatrixVal::Equal],
        [MatrixVal::Dash, MatrixVal::Equal],
        [0, 1]
    )]
    fn modifier_combos(
        (open, close): (MatrixVal, MatrixVal),
        left: MatrixVal,
        right: MatrixVal,
        space_count: usize,
    ) {
        let spaces = " ".repeat(space_count);
        let src = format!(
            "{open}{left}{spaces}x{spaces}{right}{close}",
            open = open.as_str(),
            left = left.as_str(),
            right = right.as_str(),
            close = close.as_str(),
        );
        let input = src.as_bytes();

        let open_len = open.as_str().len();
        let close_len = close.as_str().len();
        let left_len = left.as_str().len();
        let right_len = right.as_str().len();
        let tag = 0..(open_len + left_len + space_count + 1 + space_count + right_len + close_len);
        let body_start = open_len + left_len + space_count;
        let body = body_start..(body_start + 1);

        let expected = if open == MatrixVal::OpenInterp {
            Token::Interp {
                tag,
                body,
                left: left.modifier(),
                right: right.modifier(),
            }
        } else {
            Token::Stmt {
                tag,
                body,
                left: left.modifier(),
                right: right.modifier(),
            }
        };

        let mut toks = scan(input).unwrap();
        assert_eq!(toks.len(), 1);
        assert_eq!(toks.pop().unwrap(), expected);
    }

    // --- Delimiters inside string literals ----------------------------------

    #[test_case(br#"{% "%}" %}"# => stmt(0..10, 3..7); "stmt_string_shields_close")]
    #[test_case(br#"{{ "}}" }}"# => interp(0..10, 3..7); "interp_string_shields_close")]
    #[test_case(br#"{# \#} #}"# => Token::Barrier; "")]
    fn string_shielding(input: &[u8]) -> Token {
        let mut toks = scan(input).unwrap();
        assert_eq!(toks.len(), 1);
        toks.pop().unwrap()
    }

    // --- Errors -------------------------------------------------------------

    #[test_matrix(
        [
            (MatrixVal::OpenInterp, MatrixVal::CloseInterp),
            (MatrixVal::OpenStmt, MatrixVal::CloseStmt),
            (MatrixVal::OpenComment, MatrixVal::CloseComment),
        ],
        [ExpectedError::UnclosedDelimiter, ExpectedError::StrayDelimiter],
        [Context::None, Context::Prefix, Context::Suffix]
    )]
    fn delimiter_error_span(
        (open, close): (MatrixVal, MatrixVal),
        error: ExpectedError,
        context: Context,
    ) {
        let delim = match error {
            ExpectedError::UnclosedDelimiter => open,
            ExpectedError::StrayDelimiter => close,
        };

        let mut input = Vec::new();
        let prefix: &[u8] = match context {
            Context::Prefix => b"prefix ",
            _ => b"",
        };
        input.extend_from_slice(prefix);
        input.extend_from_slice(delim.as_str().as_bytes());
        if context == Context::Suffix {
            input.extend_from_slice(b" suffix");
        }

        let err = scan(&input).unwrap_err();
        match (error, &err) {
            (
                ExpectedError::UnclosedDelimiter,
                Error::Parse {
                    err: ParseError::UnclosedDelimiter,
                    span,
                },
            )
            | (
                ExpectedError::StrayDelimiter,
                Error::Parse {
                    err: ParseError::StrayDelimiter,
                    span,
                },
            ) => {
                assert_eq!(span.offset(), prefix.len());
                assert_eq!(span.len(), 2);
            }
            (_, _) => panic!(
                "expected {error:?} for {:?}, got {err:?}",
                String::from_utf8_lossy(&input)
            ),
        }
    }
}

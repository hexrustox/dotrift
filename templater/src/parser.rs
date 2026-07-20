use std::ops::Range;

use miette::SourceSpan;

use crate::{
    ast::{Expr, Node},
    error::{Error, ParseError},
    lex::is_inner_ws,
    scanner::{Modifier, Token},
    util::source_span,
};

/// Assembles tokens into AST nodes, recognizing `{{ expr }}` interpolations
/// and applying whitespace-control modifiers to adjacent text ranges.
pub(crate) fn parse(tokens: Vec<Token>, source: &[u8]) -> std::result::Result<Vec<Node>, Error> {
    let mut items: Vec<Item> = Vec::with_capacity(tokens.len());
    for token in tokens {
        match token {
            Token::Text(range) => items.push(Item::Text(range)),
            Token::Barrier => items.push(Item::Barrier),
            Token::Interp {
                tag,
                body,
                left,
                right,
            } => {
                let expr = parse_interp_body(source, body, tag)?;
                items.push(Item::Tag {
                    left,
                    right,
                    node: Node::Interpolate(expr),
                });
            }
            Token::Stmt { tag, .. } => {
                return Err(Error::parse(
                    ParseError::UnrecognizedStatement,
                    source_span(tag.clone()),
                ));
            }
        }
    }

    let mut nodes = Vec::new();
    for (i, item) in items.iter().enumerate() {
        match item {
            Item::Text(range) => {
                let mut r = range.clone();
                if let Some(Item::Tag { right, .. }) = i.checked_sub(1).and_then(|j| items.get(j)) {
                    r.start = trim_left(source, &r, *right);
                }
                if let Some(Item::Tag { left, .. }) = items.get(i + 1) {
                    r.end = trim_right(source, &r, *left);
                }
                if r.start < r.end {
                    nodes.push(Node::Text(r));
                }
            }
            Item::Tag { node, .. } => nodes.push(node.clone()),
            Item::Barrier => {}
        }
    }

    Ok(nodes)
}

enum Item {
    Text(Range<usize>),
    Tag {
        left: Modifier,
        right: Modifier,
        node: Node,
    },
    Barrier,
}

fn trim_left(src: &[u8], range: &Range<usize>, right: Modifier) -> usize {
    match right {
        Modifier::None => range.start,
        Modifier::Dash => {
            let mut i = range.start;
            while i < range.end && (src[i] == b' ' || src[i] == b'\t') {
                i += 1;
            }
            i
        }
        Modifier::Equal => match src[range.start..range.end].iter().position(|&b| b == b'\n') {
            Some(k) => range.start + k + 1,
            None => range.end,
        },
    }
}

fn trim_right(src: &[u8], range: &Range<usize>, left: Modifier) -> usize {
    match left {
        Modifier::None => range.end,
        Modifier::Dash => {
            let mut i = range.end;
            while i > range.start && (src[i - 1] == b' ' || src[i - 1] == b'\t') {
                i -= 1;
            }
            i
        }
        Modifier::Equal => match src[range.start..range.end]
            .iter()
            .rposition(|&b| b == b'\n')
        {
            // Left `=` stops before the newline, so the newline is preserved.
            Some(k) => range.start + k + 1,
            None => range.start,
        },
    }
}

/// Parses the trimmed body of a `{{ ... }}` tag into one `Expr`. The body
/// arrives already trimmed of inner-edge ASCII whitespace by the scanner;
/// an empty body is `EmptyInterpolation` and the error span covers the
/// entire `{{ ... }}` tag (`tag.start .. tag.end`).
fn parse_interp_body(
    source: &[u8],
    body: Range<usize>,
    tag: Range<usize>,
) -> std::result::Result<Expr, Error> {
    if body.start >= body.end {
        return Err(Error::parse(
            ParseError::EmptyInterpolation,
            source_span(tag.clone()),
        ));
    }

    let bytes = &source[body.clone()];
    let first = bytes[0];

    let kind = match first {
        b'"' => parse_string_literal(bytes, &tag, &body)?,
        b'-' | b'0'..=b'9' => parse_integer_literal(bytes, &body)?,
        _ if is_ident_start(first) => parse_ident_or_keyword(bytes, &body)?,
        _ => {
            return Err(Error::parse(
                ParseError::UnexpectedToken,
                body_span(&body, 0..1),
            ));
        }
    };

    // Reject trailing junk after the single expression. Skip interior
    // whitespace, then if any bytes remain the error points at the first
    // non-whitespace leftover byte.
    let consumed = kind.consumed;
    if consumed < bytes.len() {
        let mut leftover = consumed;
        while leftover < bytes.len() && is_inner_ws(bytes[leftover]) {
            leftover += 1;
        }
        let at = if leftover >= bytes.len() {
            consumed
        } else {
            leftover
        };
        return Err(Error::parse(
            ParseError::UnexpectedTokensAfterExpr,
            body_span(&body, at..at + 1),
        ));
    }

    Ok(kind.expr)
}

/// Convenience bundle so `parse_interp_body` can read `consumed` before
/// moving `expr`.
struct KindBundle {
    expr: Expr,
    consumed: usize,
}

fn parse_string_literal(
    bytes: &[u8],
    tag: &Range<usize>,
    body: &Range<usize>,
) -> std::result::Result<KindBundle, Error> {
    debug_assert_eq!(bytes[0], b'"');
    let mut i = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if i + 1 < bytes.len() => i += 2,
            b'"' => {
                // Closing quote at body offset `i`. Interior is `1..i`.
                let interior = body_range(body, 1..i);
                return Ok(KindBundle {
                    expr: Expr::StrLit(interior),
                    consumed: i + 1,
                });
            }
            _ => i += 1,
        }
    }

    // Unclosed: span from the opening `"` (trimmed body start) through the
    // end of the closing `}}` delimiter — the "rest of the tag" the parser
    // would have consumed had the string closed.
    Err(Error::parse(
        ParseError::UnclosedString,
        source_span(body.start..tag.end),
    ))
}

fn parse_integer_literal(
    bytes: &[u8],
    body: &Range<usize>,
) -> std::result::Result<KindBundle, Error> {
    let mut i = 0;
    let negative = bytes[0] == b'-';
    if negative {
        i += 1;
    }
    let digits_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }

    if i == digits_start {
        // `-` with no following digits is not a valid integer literal.
        return Err(Error::parse(
            ParseError::UnexpectedToken,
            body_span(body, 0..1),
        ));
    }

    let digits = &bytes[digits_start..i];

    // i128 accumulator so we can detect i64 overflow ourselves (and report
    // a precise span) rather than relying on i64 overflow semantics.
    let mut acc: i128 = 0;
    for &d in digits {
        acc = acc
            .checked_mul(10)
            .and_then(|n| n.checked_add((d - b'0') as i128))
            .ok_or(Error::parse(
                ParseError::IntegerOutOfRange,
                body_span(body, 0..i),
            ))?;
    }

    let value = if negative {
        let neg = acc.checked_neg().ok_or(Error::parse(
            ParseError::IntegerOutOfRange,
            body_span(body, 0..i),
        ))?;
        if neg < i64::MIN as i128 {
            return Err(Error::parse(
                ParseError::IntegerOutOfRange,
                body_span(body, 0..i),
            ));
        }
        neg as i64
    } else {
        if acc > i64::MAX as i128 {
            return Err(Error::parse(
                ParseError::IntegerOutOfRange,
                body_span(body, 0..i),
            ));
        }
        acc as i64
    };

    Ok(KindBundle {
        expr: Expr::IntLit(value),
        consumed: i,
    })
}

fn parse_ident_or_keyword(
    bytes: &[u8],
    body: &Range<usize>,
) -> std::result::Result<KindBundle, Error> {
    let mut i = 0;
    while i < bytes.len() && is_ident_byte(bytes[i]) {
        i += 1;
    }
    debug_assert!(i > 0);

    let range = body_range(body, 0..i);
    let ident = &bytes[..i];
    let expr = match ident {
        b"true" => Expr::BoolLit(true),
        b"false" => Expr::BoolLit(false),
        _ => Expr::Var(range),
    };
    Ok(KindBundle { expr, consumed: i })
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Translates a body-relative `Range` into absolute source-space (used for
/// AST node byte offsets such as `Expr::Var`).
fn body_range(body: &Range<usize>, rel: Range<usize>) -> Range<usize> {
    (body.start + rel.start)..(body.start + rel.end)
}

/// Translates a body-relative `Range` into a miette `SourceSpan` (used for
/// error spans).
fn body_span(body: &Range<usize>, rel: Range<usize>) -> SourceSpan {
    source_span(body.start + rel.start..body.start + rel.end)
}

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use super::*;

    macro_rules! expr {
        (var $r:expr) => {
            Expr::Var($r)
        };
        (str $r:expr) => {
            Expr::StrLit($r)
        };
        (int $n:expr) => {
            Expr::IntLit($n)
        };
        (bool $b:expr) => {
            Expr::BoolLit($b)
        };
    }

    /// Drives `parse` with one `{{ ... }}` token constructed the way the
    /// scanner would: `tag` covers `{{ ... }}`, `body` is the trimmed
    /// interior.
    fn parse_one(input: &[u8]) -> Expr {
        let token = token_from_input(input);
        let nodes = parse(vec![token], input).unwrap();
        assert_eq!(nodes.len(), 1);
        match nodes.into_iter().next().unwrap() {
            Node::Interpolate(e) => e,
            _ => unreachable!("expected Interpolate"),
        }
    }

    /// Builds the same `Token::Interp { tag, body, left, right }` the scanner
    /// would produce for `input`: `tag = open..close_end`, `body` trimmed,
    /// no modifiers.
    fn token_from_input(input: &[u8]) -> Token {
        let open = input.windows(2).position(|w| w == b"{{").unwrap();
        let close = input.windows(2).rposition(|w| w == b"}}").unwrap();
        let close_end = close + 2;
        let mut body_start = open + 2;
        let mut body_end = close;
        while body_start < body_end && is_inner_ws(input[body_start]) {
            body_start += 1;
        }
        while body_end > body_start && is_inner_ws(input[body_end - 1]) {
            body_end -= 1;
        }
        Token::Interp {
            tag: open..close_end,
            body: body_start..body_end,
            left: Modifier::None,
            right: Modifier::None,
        }
    }

    #[test_case(b"{{ x }}" => expr!(var 3..4); "var")]
    #[test_case(b"{{ x_y }}" => expr!(var 3..6); "underscore_in_var")]
    #[test_case(b"{{ a1 }}" => expr!(var 3..5); "digit_in_var")]
    #[test_case(br#"{{ "str" }}"# => expr!(str 4..7); "string_literal")]
    #[test_case(br#"{{ "" }}"# => expr!(str 4..4); "empty_string_literal")]
    #[test_case(br#"{{ "a\"b" }}"# => expr!(str 4..8); "string_with_escaped_quote")]
    #[test_case(br#"{{ "a\\b" }}"# => expr!(str 4..8); "string_with_escaped_backslash")]
    #[test_case(br#"{{ "a\xb" }}"# => expr!(str 4..8); "string_with_passthrough_escape")]
    #[test_case(b"{{ 42 }}" => expr!(int 42); "int_positive")]
    #[test_case(b"{{ -7 }}" => expr!(int -7); "int_negative")]
    #[test_case(b"{{ 007 }}" => expr!(int 7); "int_leading_zeros")]
    #[test_case(b"{{ -0 }}" => expr!(int 0); "int_neg_zero")]
    #[test_case(b"{{ 9223372036854775807 }}" => expr!(int i64::MAX); "int_max_i64")]
    #[test_case(b"{{ -9223372036854775808 }}" => expr!(int i64::MIN); "int_min_i64")]
    #[test_case(b"{{ true }}" => expr!(bool true); "bool_true")]
    #[test_case(b"{{ false }}" => expr!(bool false); "bool_false")]
    fn parse_one_cases(input: &[u8]) -> Expr {
        parse_one(input)
    }

    // --- Parse errors -------------------------------------------------------

    fn err(input: &[u8]) -> (ParseError, (usize, usize)) {
        let token = token_from_input(input);
        let e = parse(vec![token], input).unwrap_err();
        let Error::Parse { err, span } = e else {
            panic!("expected parse error, got {e:?}");
        };
        (err, (span.offset(), span.len()))
    }

    #[test_case(b"{{ }}" => matches ParseError::EmptyInterpolation; "empty_body")]
    #[test_case(b"{{ 99999999999999999999999 }}" => matches ParseError::IntegerOutOfRange; "overflow_positive")]
    #[test_case(b"{{ -99999999999999999999999 }}" => matches ParseError::IntegerOutOfRange; "overflow_negative")]
    #[test_case(b"{{ \"hello }}" => matches ParseError::UnclosedString; "unclosed_string")]
    #[test_case(b"{{ @ }}" => matches ParseError::UnexpectedToken; "unexpected_byte")]
    #[test_case(b"{{ a b }}" => matches ParseError::UnexpectedTokensAfterExpr; "trailing_token")]
    #[test_case(b"{{ - }}" => matches ParseError::UnexpectedToken; "minus_alone")]
    fn kind_cases(input: &[u8]) -> ParseError {
        err(input).0
    }

    #[test_case(b"{{ }}" => (0, 5); "empty_body_span")]
    #[test_case(b"{{   }}" => (0, 7); "empty_body_span_padded")]
    #[test_case(b"{{}}" => (0, 4); "empty_body_span_tight")]
    #[test_case(b"{{ +7 }}" => (3, 1); "plus_prefixed_span")]
    #[test_case(b"{{ @ }}" => (3, 1); "unexpected_byte_span")]
    #[test_case(b"{{ a b }}" => (5, 1); "trailing_token_span")]
    #[test_case(b"{{ \"hello }}" => (3, 9); "unclosed_string_span")]
    fn span_cases(input: &[u8]) -> (usize, usize) {
        err(input).1
    }
}

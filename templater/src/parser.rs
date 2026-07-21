use std::ops::Range;

use miette::SourceSpan;

use crate::{
    ast::{Expr, Node},
    error::{Error, ParseError},
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

    let mut state = ParserState {
        source,
        tag,
        body: body.clone(),
        pos: 0,
    };
    let expr = state.parse_expr()?;
    state.skip_ws();
    if state.pos < state.len() {
        return Err(Error::parse(
            ParseError::UnexpectedTokensAfterExpr,
            state.span(state.pos..state.pos + 1),
        ));
    }
    Ok(expr)
}

struct ParserState<'s> {
    source: &'s [u8],
    tag: Range<usize>,
    body: Range<usize>,
    pos: usize,
}

impl<'s> ParserState<'s> {
    fn len(&self) -> usize {
        self.body.len()
    }

    fn bytes(&self) -> &'s [u8] {
        &self.source[self.body.clone()]
    }

    fn span(&self, rel: Range<usize>) -> SourceSpan {
        source_span(self.body.start + rel.start..self.body.start + rel.end)
    }

    fn skip_ws(&mut self) {
        let bytes = self.bytes();
        while self.pos < bytes.len() && (bytes[self.pos] == b' ' || bytes[self.pos] == b'\t') {
            self.pos += 1;
        }
    }

    fn parse_expr(&mut self) -> std::result::Result<Expr, Error> {
        let primary = self.parse_primary()?;
        self.parse_postfix(primary)
    }

    fn parse_primary(&mut self) -> std::result::Result<Expr, Error> {
        self.skip_ws();
        let bytes = self.bytes();
        match bytes[self.pos] {
            b'"' => self.parse_string_literal(),
            b'-' | b'0'..=b'9' => self.parse_integer_literal(),
            _ if is_ident_start(bytes[self.pos]) => {
                let (range, is_keyword) = self.parse_identifier()?;
                let ident =
                    &self.bytes()[range.start - self.body.start..range.end - self.body.start];
                let lit_span = body_range(
                    &self.body,
                    (range.start - self.body.start)..(range.end - self.body.start),
                );
                Ok(match ident {
                    b"true" => Expr::BoolLit(true, lit_span.clone()),
                    b"false" => Expr::BoolLit(false, lit_span),
                    _ => {
                        if is_keyword {
                            return Err(Error::parse(
                                ParseError::ReservedKeyword {
                                    keyword: std::str::from_utf8(ident)
                                        .expect("identifier is ascii")
                                        .to_owned(),
                                },
                                source_span(range.clone()),
                            ));
                        }
                        // Function calls may have whitespace between the
                        // name and `(`. Dot access may not; preserve the
                        // original position so `x .y` remains an error.
                        let after_ident = self.pos;
                        self.skip_ws();
                        match self.peek_byte() {
                            Some(b'(') => self.parse_fn_call(range)?,
                            _ => {
                                self.pos = after_ident;
                                Expr::Var(range)
                            }
                        }
                    }
                })
            }
            b'[' => self.parse_list_literal(),
            _ => Err(Error::parse(
                ParseError::UnexpectedToken,
                self.span(self.pos..self.pos + 1),
            )),
        }
    }

    fn peek_byte(&self) -> Option<u8> {
        self.bytes().get(self.pos).copied()
    }

    fn parse_postfix(&mut self, mut left: Expr) -> std::result::Result<Expr, Error> {
        loop {
            let bytes = self.bytes();
            if self.pos >= bytes.len() || bytes[self.pos] != b'.' {
                break Ok(left);
            }
            let dot_pos = self.pos;
            self.pos += 1; // consume '.'

            let bytes = self.bytes();
            if self.pos >= bytes.len() {
                return Err(Error::parse(
                    ParseError::EmptyField,
                    self.span(dot_pos..dot_pos + 1),
                ));
            }

            if is_ident_start(bytes[self.pos]) {
                let ident_start = self.pos;
                while self.pos < bytes.len() && is_ident_byte(bytes[self.pos]) {
                    self.pos += 1;
                }
                let field = body_range(&self.body, ident_start..self.pos);
                left = Expr::Dot {
                    left: Box::new(left),
                    field,
                };
            } else if bytes[self.pos] == b'-' || bytes[self.pos].is_ascii_digit() {
                // Parse integer index, which may be negative.
                let idx_start = self.pos;
                let negative = bytes[self.pos] == b'-';
                if negative {
                    self.pos += 1;
                }
                if self.pos >= bytes.len() || !bytes[self.pos].is_ascii_digit() {
                    return Err(Error::parse(
                        ParseError::EmptyField,
                        self.span(dot_pos..dot_pos + 1),
                    ));
                }
                while self.pos < bytes.len() && bytes[self.pos].is_ascii_digit() {
                    self.pos += 1;
                }
                let idx_bytes = &bytes[idx_start..self.pos];
                let idx: i64 = std::str::from_utf8(idx_bytes)
                    .expect("digits are ascii")
                    .parse()
                    .expect("digits fit within i64 for reasonable lengths");
                let idx_span = body_range(&self.body, idx_start..self.pos);
                left = Expr::Index {
                    left: Box::new(left),
                    idx,
                    idx_span,
                };
            } else {
                return Err(Error::parse(
                    ParseError::EmptyField,
                    self.span(dot_pos..dot_pos + 1),
                ));
            }
        }
    }

    fn parse_string_literal(&mut self) -> std::result::Result<Expr, Error> {
        let bytes = self.bytes();
        debug_assert_eq!(bytes[self.pos], b'"');
        let start = self.pos;
        self.pos += 1;
        while self.pos < bytes.len() {
            match bytes[self.pos] {
                b'\\' if self.pos + 1 < bytes.len() => self.pos += 2,
                b'"' => {
                    let interior = body_range(&self.body, start + 1..self.pos);
                    self.pos += 1;
                    let span = body_range(&self.body, start..self.pos);
                    return Ok(Expr::StrLit { interior, span });
                }
                _ => self.pos += 1,
            }
        }
        Err(Error::parse(
            ParseError::UnclosedString,
            source_span(self.body.start + start..self.tag.end),
        ))
    }

    fn parse_integer_literal(&mut self) -> std::result::Result<Expr, Error> {
        let bytes = self.bytes();
        let start = self.pos;
        let negative = bytes[self.pos] == b'-';
        if negative {
            self.pos += 1;
        }
        let digits_start = self.pos;
        while self.pos < bytes.len() && bytes[self.pos].is_ascii_digit() {
            self.pos += 1;
        }

        if self.pos == digits_start {
            return Err(Error::parse(
                ParseError::UnexpectedToken,
                self.span(start..start + 1),
            ));
        }

        let digits = &bytes[digits_start..self.pos];

        let mut acc: i64 = 0;
        for &d in digits {
            acc = acc
                .checked_mul(10)
                .and_then(|n| {
                    let d = (d - b'0').into();
                    if negative {
                        n.checked_sub(d)
                    } else {
                        n.checked_add(d)
                    }
                })
                .ok_or(Error::parse(
                    ParseError::IntegerOutOfRange,
                    self.span(start..self.pos),
                ))?;
        }

        let lit_span = body_range(&self.body, start..self.pos);
        Ok(Expr::IntLit(acc, lit_span))
    }

    /// Parses an `[A-Za-z_][A-Za-z0-9_]*` identifier and returns its absolute
    /// source range plus whether it is a reserved keyword (`if`, `elif`,
    /// `else`, `for`, `in`, `end`). Boolean literals are treated separately by
    /// the caller.
    fn parse_identifier(&mut self) -> std::result::Result<(Range<usize>, bool), Error> {
        let bytes = self.bytes();
        let start = self.pos;
        debug_assert!(is_ident_start(bytes[start]));
        while self.pos < bytes.len() && is_ident_byte(bytes[self.pos]) {
            self.pos += 1;
        }

        let range = body_range(&self.body, start..self.pos);
        let ident = &bytes[start..self.pos];
        let is_keyword = matches!(ident, b"if" | b"elif" | b"else" | b"for" | b"in" | b"end");
        Ok((range, is_keyword))
    }

    fn parse_fn_call(&mut self, name: Range<usize>) -> std::result::Result<Expr, Error> {
        let bytes = self.bytes();
        let lparen = self.pos;
        debug_assert_eq!(bytes[self.pos], b'(');
        self.pos += 1; // consume '('

        let mut args = Vec::new();
        loop {
            self.skip_ws();
            let bytes = self.bytes();
            if self.pos >= bytes.len() {
                return Err(Error::parse(
                    ParseError::UnclosedDelimiter,
                    self.span(self.pos.saturating_sub(1)..self.pos),
                ));
            }
            if bytes[self.pos] == b')' {
                self.pos += 1;
                break;
            }

            args.push(self.parse_expr()?);

            self.skip_ws();
            let bytes = self.bytes();
            if self.pos >= bytes.len() {
                return Err(Error::parse(
                    ParseError::UnclosedDelimiter,
                    self.span(self.pos.saturating_sub(1)..self.pos),
                ));
            }
            match bytes[self.pos] {
                b',' => {
                    let comma_pos = self.pos;
                    self.pos += 1;
                    // Trailing comma check: if next non-ws is `)`, it's a
                    // trailing comma.
                    self.skip_ws();
                    let bytes = self.bytes();
                    if self.pos < bytes.len() && bytes[self.pos] == b')' {
                        return Err(Error::parse(
                            ParseError::TrailingComma,
                            self.span(comma_pos..comma_pos + 1),
                        ));
                    }
                }
                b')' => {
                    self.pos += 1;
                    break;
                }
                _ => {
                    return Err(Error::parse(
                        ParseError::UnexpectedToken,
                        self.span(self.pos..self.pos + 1),
                    ));
                }
            }
        }

        let paren = body_range(&self.body, lparen..self.pos);
        Ok(Expr::FnCall { name, args, paren })
    }

    fn parse_list_literal(&mut self) -> std::result::Result<Expr, Error> {
        let bytes = self.bytes();
        debug_assert_eq!(bytes[self.pos], b'[');
        let start = self.pos;
        self.pos += 1; // consume '['

        let mut elements = Vec::new();
        loop {
            self.skip_ws();
            let bytes = self.bytes();
            if self.pos >= bytes.len() {
                return Err(Error::parse(
                    ParseError::UnclosedDelimiter,
                    self.span(self.pos.saturating_sub(1)..self.pos),
                ));
            }
            if bytes[self.pos] == b']' {
                self.pos += 1;
                break;
            }

            elements.push(self.parse_expr()?);

            self.skip_ws();
            let bytes = self.bytes();
            if self.pos >= bytes.len() {
                return Err(Error::parse(
                    ParseError::UnclosedDelimiter,
                    self.span(self.pos.saturating_sub(1)..self.pos),
                ));
            }
            match bytes[self.pos] {
                b',' => {
                    let comma_pos = self.pos;
                    self.pos += 1;
                    // Trailing comma check: if next non-ws is `]`, it's a
                    // trailing comma.
                    self.skip_ws();
                    let bytes = self.bytes();
                    if self.pos < bytes.len() && bytes[self.pos] == b']' {
                        return Err(Error::parse(
                            ParseError::TrailingComma,
                            self.span(comma_pos..comma_pos + 1),
                        ));
                    }
                }
                b']' => {
                    self.pos += 1;
                    break;
                }
                _ => {
                    return Err(Error::parse(
                        ParseError::UnexpectedToken,
                        self.span(self.pos..self.pos + 1),
                    ));
                }
            }
        }

        let span = body_range(&self.body, start..self.pos);
        Ok(Expr::List { elements, span })
    }
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

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use super::*;
    use crate::lex::is_inner_ws;

    macro_rules! expr {
        (var $r:expr) => {
            Expr::Var($r)
        };
        (str $r:expr) => {
            Expr::StrLit { interior: $r, span: $r }
        };
        (int $n:expr) => {
            Expr::IntLit($n, 0..0)
        };
        (bool $b:expr) => {
            Expr::BoolLit($b, 0..0)
        };
        (list $($e:expr),* $(,)?) => {
            Expr::List { elements: vec![$($e),*], span: 0..0 }
        };
        (dot $left:expr, $field:expr) => {
            Expr::Dot {
                left: Box::new($left),
                field: $field,
            }
        };
        (idx $left:expr, $idx:expr, $span:expr) => {
            Expr::Index {
                left: Box::new($left),
                idx: $idx,
                idx_span: $span,
            }
        };
        (call $name:expr, $paren:expr $(, $arg:expr)*) => {
            Expr::FnCall {
                name: $name,
                paren: $paren,
                args: vec![$($arg),*],
            }
        };
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
    #[test_case(b"{{ [] }}" => expr!(list); "empty_list")]
    #[test_case(b"{{ [1, 2] }}" => expr!(list expr!(int 1), expr!(int 2)); "list_of_ints")]
    #[test_case(br#"{{ ["a", "b"] }}"# => expr!(list expr!(str 5..6), expr!(str 10..11)); "list_of_strings")]
    #[test_case(b"{{ [1 , 2 , 3] }}" => expr!(list expr!(int 1), expr!(int 2), expr!(int 3)); "list_with_spaces")]
    #[test_case(br#"{{ [1, ["a"]] }}"# => expr!(list expr!(int 1), expr!(list expr!(str 9..10))); "nested_list")]
    #[test_case(b"{{ x.y }}" => expr!(dot expr!(var 3..4), 5..6); "simple_dot")]
    #[test_case(b"{{ x.y.z }}" => expr!(dot expr!(dot expr!(var 3..4), 5..6), 7..8); "dot_chain")]
    #[test_case(b"{{ x.0 }}" => expr!(idx expr!(var 3..4), 0, 5..6); "simple_index")]
    #[test_case(b"{{ x.0.1 }}" => expr!(idx expr!(idx expr!(var 3..4), 0, 5..6), 1, 7..8); "index_chain")]
    #[test_case(b"{{ f() }}" => expr!(call 3..4, 4..6); "zero_arg_call")]
    #[test_case(b"{{ f(1) }}" => expr!(call 3..4, 4..7, expr!(int 1)); "single_arg_call")]
    #[test_case(b"{{ f(1, 2) }}" => expr!(call 3..4, 4..10, expr!(int 1), expr!(int 2)); "two_arg_call")]
    #[test_case(b"{{ f(1 , 2) }}" => expr!(call 3..4, 4..11, expr!(int 1), expr!(int 2)); "call_with_spaces")]
    #[test_case(br#"{{ join(":", "a", "b") }}"# => expr!(call 3..7, 7..22, expr!(str 9..10), expr!(str 14..15), expr!(str 19..20)); "call_string_args")]
    #[test_case(b"{{ f(g()) }}" => expr!(call 3..4, 4..9, expr!(call 5..6, 6..8)); "nested_call")]
    #[test_case(b"{{ f(g(), h()) }}" => expr!(call 3..4, 4..14, expr!(call 5..6, 6..8), expr!(call 10..11, 11..13)); "multiple_nested_calls")]
    fn parse_cases(input: &[u8]) -> Expr {
        let token = token_from_input(input);
        let nodes = parse(vec![token], input).unwrap();
        assert_eq!(nodes.len(), 1);
        match nodes.into_iter().next().unwrap() {
            Node::Interpolate(e) => e,
            _ => unreachable!("expected Interpolate"),
        }
    }

    #[test_case(b"{{ }}" => matches (ParseError::EmptyInterpolation, (0, 5)); "empty_body")]
    #[test_case(b"{{   }}" => matches (ParseError::EmptyInterpolation, (0, 7)); "empty_body_padded")]
    #[test_case(b"{{}}" => matches (ParseError::EmptyInterpolation, (0, 4)); "empty_body_tight")]
    #[test_case(b"{{ 99999999999999999999999 }}" => matches (ParseError::IntegerOutOfRange, (_, _)); "overflow_positive")]
    #[test_case(b"{{ -99999999999999999999999 }}" => matches (ParseError::IntegerOutOfRange, (_, _)); "overflow_negative")]
    #[test_case(br#"{{ "hello }}"# => matches (ParseError::UnclosedString, (3, 9)); "unclosed_string")]
    #[test_case(b"{{ @ }}" => matches (ParseError::UnexpectedToken, (3, 1)); "unexpected_byte")]
    #[test_case(b"{{ +7 }}" => matches (ParseError::UnexpectedToken, (3, 1)); "plus_prefix")]
    #[test_case(b"{{ a b }}" => matches (ParseError::UnexpectedTokensAfterExpr, (5, 1)); "trailing_token")]
    #[test_case(b"{{ - }}" => matches (ParseError::UnexpectedToken, (_, _)); "minus_alone")]
    #[test_case(b"{{ [a, ] }}" => matches (ParseError::TrailingComma, (5, 1)); "trailing_comma_list")]
    #[test_case(b"{{ [ }}" => matches (ParseError::UnclosedDelimiter, (_, _)); "unclosed_list")]
    #[test_case(b"{{ x. }}" => matches (ParseError::EmptyField, (4, 1)); "empty_field")]
    #[test_case(b"{{ x.- }}" => matches (ParseError::EmptyField, (_, _)); "field_minus_without_digits")]
    #[test_case(b"{{ x .y }}" => matches (ParseError::UnexpectedTokensAfterExpr, (5, 1)); "space_before_dot")]
    #[test_case(b"{{ x. y }}" => matches (ParseError::EmptyField, (4, 1)); "space_after_dot")]
    #[test_case(b"{{ [a}}" => matches (ParseError::UnclosedDelimiter, (_, _)); "unclosed_list_")]
    #[test_case(b"{{ [a,}}" => matches (ParseError::UnclosedDelimiter, (_, _)); "unclosed_list_after_comma")]
    #[test_case(b"{{ [a b] }}" => matches (ParseError::UnexpectedToken, (_, _)); "unexpected_token_after_element")]
    #[test_case(b"{{ f(a, ) }}" => matches (ParseError::TrailingComma, (6, 1)); "trailing_comma_call")]
    #[test_case(b"{{ if() }}" => matches (ParseError::ReservedKeyword { keyword }, (3, 2)) if keyword == "if"; "keyword_function_name_if")]
    #[test_case(b"{{ for() }}" => matches (ParseError::ReservedKeyword { keyword }, (3, 3)) if keyword == "for"; "keyword_function_name_for")]
    #[test_case(b"{{ end() }}" => matches (ParseError::ReservedKeyword { keyword }, (3, 3)) if keyword == "end"; "keyword_function_name_end")]
    #[test_case(b"{{ 1st() }}" => matches (ParseError::UnexpectedTokensAfterExpr, (4, 1)); "digit_prefixed_call")]
    #[test_case(b"{{ kebab-fn() }}" => matches (ParseError::UnexpectedTokensAfterExpr, (8, 1)); "kebab_function_name")]
    fn parse_error_cases(input: &[u8]) -> (ParseError, (usize, usize)) {
        let token = token_from_input(input);
        let Error::Parse { err, span } = parse(vec![token], input).unwrap_err() else {
            panic!("expected parse error");
        };
        (err, (span.offset(), span.len()))
    }
}

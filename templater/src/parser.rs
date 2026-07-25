use std::ops::Range;

use miette::SourceSpan;

use crate::{
    ast::{Expr, Node},
    error::{Error, ParseError},
    scanner::Token,
    util::{is_whitespace, source_span},
};

/// Assembles already-trimmed tokens into AST nodes, recognizing
/// `{{ expr }}` interpolations.
pub(crate) fn parse(tokens: Vec<Token>, source: &[u8]) -> std::result::Result<Vec<Node>, Error> {
    let mut nodes = Vec::with_capacity(tokens.len());
    for token in tokens {
        match token {
            Token::Text(range) => nodes.push(Node::Text(range)),
            Token::Barrier => {}
            Token::Interp { tag, body, .. } => {
                nodes.push(Node::Interpolate(parse_interp_body(source, body, tag)?));
            }
            Token::Stmt { tag, .. } => {
                // TODO
                return Err(Error::parse(
                    ParseError::UnrecognizedStatement,
                    source_span(tag.clone()),
                ));
            }
        }
    }

    Ok(nodes)
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
        while self.pos < bytes.len() && is_whitespace(bytes[self.pos]) {
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
                    b"true" => Expr::BoolLit {
                        value: true,
                        span: lit_span.clone(),
                    },
                    b"false" => Expr::BoolLit {
                        value: false,
                        span: lit_span,
                    },
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
                    return Err(Error::Parse {
                        err: ParseError::UnexpectedToken,
                        span: self.span(dot_pos + 1..dot_pos + 2),
                    });
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
                return Err(Error::Parse {
                    err: ParseError::UnexpectedToken,
                    span: self.span(dot_pos + 1..dot_pos + 2),
                });
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
            source_span(self.body.start + start..self.body.start + start + 1),
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
        Ok(Expr::IntLit {
            value: acc,
            span: lit_span,
        })
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
                    ParseError::UnclosedCallParen,
                    self.span(lparen..lparen + 1),
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
                    ParseError::UnclosedCallParen,
                    self.span(lparen..lparen + 1),
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
        let lbracket = self.pos;
        debug_assert_eq!(bytes[self.pos], b'[');
        let start = self.pos;
        self.pos += 1; // consume '['

        let mut elements = Vec::new();
        loop {
            self.skip_ws();
            let bytes = self.bytes();
            if self.pos >= bytes.len() {
                return Err(Error::parse(
                    ParseError::UnclosedListBracket,
                    self.span(lbracket..lbracket + 1),
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
                    ParseError::UnclosedListBracket,
                    self.span(lbracket..lbracket + 1),
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

    use crate::scanner::scan;

    use super::*;

    macro_rules! expr {
        (var $r:expr) => {
            $crate::ast::Expr::Var($r)
        };
        (str $r:expr, $span:expr) => {
            $crate::ast::Expr::StrLit { interior: $r, span: $span }
        };
        (int $n:expr, $span:expr) => {
            $crate::ast::Expr::IntLit { value: $n, span: $span }
        };
        (bool $b:expr, $span:expr) => {
            $crate::ast::Expr::BoolLit { value: $b, span: $span }
        };
        (list $span:expr; $($e:expr),* $(,)?) => {
            $crate::ast::Expr::List { elements: vec![$($e),*], span: $span }
        };
        (dot $left:expr, $field:expr) => {
            $crate::ast::Expr::Dot {
                left: Box::new($left),
                field: $field,
            }
        };
        (idx $left:expr, $idx:expr, $span:expr) => {
            $crate::ast::Expr::Index {
                left: Box::new($left),
                idx: $idx,
                idx_span: $span,
            }
        };
        (call $name:expr, $paren:expr $(, $arg:expr)*) => {
            $crate::ast::Expr::FnCall {
                name: $name,
                paren: $paren,
                args: vec![$($arg),*],
            }
        };
    }

    // -- string literals --
    #[test_case(br#""""# => expr!(str 3..3, 2..4) ; "empty_string")]
    #[test_case(b"\"hi\"" => expr!(str 3..5, 2..6) ; "string_literal")]
    #[test_case(b"\"a\\\"b\"" => expr!(str 3..7, 2..8) ; "string_with_double_quote_escape")]
    #[test_case(b"\"a\\\\b\"" => expr!(str 3..7, 2..8) ; "string_with_backslash_escape")]
    #[test_case(b"\"\\n\"" => expr!(str 3..5, 2..6) ; "string_unknown_escape_preserved")]
    #[test_case(b"\"line1\nline2\"" => expr!(str 3..14, 2..15) ; "string_multiline")]
    #[test_case(b"\"{{x}}\"" => expr!(str 3..8, 2..9) ; "string_shields_delimiters")]
    // -- integer literals --
    #[test_case(b"42" => expr!(int 42, 2..4) ; "integer")]
    #[test_case(b" -7" => expr!(int -7, 3..5) ; "negative_integer")]
    #[test_case(b"007" => expr!(int 7, 2..5) ; "leading_zeros_decimal")]
    #[test_case(b"9223372036854775807" => expr!(int 9223372036854775807, 2..21) ; "max_i64")]
    #[test_case(b" -9223372036854775808" => expr!(int -9223372036854775808, 3..23) ; "min_i64")]
    // -- boolean literals --
    #[test_case(b"true" => expr!(bool true, 2..6) ; "bool_true")]
    #[test_case(b"false" => expr!(bool false, 2..7) ; "bool_false")]
    // -- variables / identifiers --
    #[test_case(b"x" => expr!(var 2..3) ; "variable")]
    #[test_case(b"_foo" => expr!(var 2..6) ; "variable_underscore_start")]
    #[test_case(b"var123" => expr!(var 2..8) ; "identifier_with_digits")]
    #[test_case(b"_foo_bar" => expr!(var 2..10) ; "identifier_multi_underscore")]
    // -- dot access --
    #[test_case(b"obj.field" => expr!(dot expr!(var 2..5), 6..11) ; "dot_access")]
    #[test_case(b"a.b.c" => expr!(dot expr!(dot expr!(var 2..3), 4..5), 6..7) ; "chained_dot")]
    // -- indexing --
    #[test_case(b"list.0" => expr!(idx expr!(var 2..6), 0, 7..8) ; "integer_index")]
    #[test_case(b"list.-1" => expr!(idx expr!(var 2..6), -1, 7..9) ; "negative_index")]
    #[test_case(b"a.0.b" => expr!(dot expr!(idx expr!(var 2..3), 0, 4..5), 6..7) ; "index_then_dot")]
    #[test_case(b"obj.a.b.0" => expr!(idx expr!(dot expr!(dot expr!(var 2..5), 6..7), 8..9), 0, 10..11) ; "deep_chain_ending_index")]
    // -- function calls: basic --
    #[test_case(b"fn()" => expr!(call 2..4, 4..6) ; "zero_arg_call")]
    #[test_case(b"f(a, b)" => expr!(call 2..3, 3..9, expr!(var 4..5), expr!(var 7..8)) ; "two_arg_call")]
    #[test_case(b"f(a,b)" => expr!(call 2..3, 3..8, expr!(var 4..5), expr!(var 6..7)) ; "call_no_whitespace")]
    #[test_case(b"f( a , b )" => expr!(call 2..3, 3..12, expr!(var 5..6), expr!(var 9..10)) ; "call_whitespace_inside_parens")]
    #[test_case(b"f\n()" => expr!(call 2..3, 4..6) ; "call_newline_before_paren")]
    // -- function calls: mixed arguments --
    #[test_case(b"join(\":\", a, b)" => expr!(call 2..6, 6..17, expr!(str 8..9, 7..10), expr!(var 12..13), expr!(var 15..16)) ; "call_mixed_literal_var")]
    // -- function calls: nesting --
    #[test_case(b"eq(gt(x, 3), y)" => expr!(call 2..4, 4..17, expr!(call 5..7, 7..13, expr!(var 8..9), expr!(int 3, 11..12)), expr!(var 15..16)) ; "nested_call")]
    #[test_case(b"f(g(h(x)))" => expr!(call 2..3, 3..12, expr!(call 4..5, 5..11, expr!(call 6..7, 7..10, expr!(var 8..9)))) ; "deeply_nested_calls")]
    // -- function calls: postfix --
    #[test_case(b"fn().field" => expr!(dot expr!(call 2..4, 4..6), 7..12) ; "call_then_dot")]
    #[test_case(b"fn().0" => expr!(idx expr!(call 2..4, 4..6), 0, 7..8) ; "call_then_index")]
    // -- lists: basic --
    #[test_case(b"[]" => expr!(list 2..4;) ; "empty_list")]
    #[test_case(b"[a, b]" => expr!(list 2..8; expr!(var 3..4), expr!(var 6..7)) ; "list_two_elements")]
    #[test_case(b"[\"x\", 42]" => expr!(list 2..11; expr!(str 4..5, 3..6), expr!(int 42, 8..10)) ; "list_mixed")]
    // -- lists: whitespace --
    #[test_case(b"[  ]" => expr!(list 2..6;) ; "empty_list_with_spaces")]
    #[test_case(b"[\"a\" , \"b\"]" => expr!(list 2..13; expr!(str 4..5, 3..6), expr!(str 10..11, 9..12)) ; "list_whitespace_inside")]
    #[test_case(b"[\n a,\n b\n]" => expr!(list 2..12; expr!(var 5..6), expr!(var 9..10)) ; "list_multiline")]
    // -- lists: nesting --
    #[test_case(b"[[]]" => expr!(list 2..6; expr!(list 3..5;)) ; "nested_list")]
    #[test_case(b"[a, [b, c], d]" => expr!(list 2..16; expr!(var 3..4), expr!(list 6..12; expr!(var 7..8), expr!(var 10..11)), expr!(var 14..15)) ; "list_nested_nonempty")]
    #[test_case(b"[[a], [b]]" => expr!(list 2..12; expr!(list 3..6; expr!(var 4..5)), expr!(list 8..11; expr!(var 9..10))) ; "list_of_lists")]
    // -- body trimming --
    #[test_case(b"  x  " => expr!(var 4..5) ; "trimmed_whitespace_body")]
    #[test_case(b"\n x \n" => expr!(var 4..5) ; "trimmed_multiline_body")]
    fn parse_interp(src: &[u8]) -> Expr {
        let src = [b"{{", src, b"}}"].concat();
        let Node::Interpolate(expr) = parse(scan(&src).unwrap(), &src).unwrap().pop().unwrap()
        else {
            panic!("expected Interpolate")
        };
        expr
    }

    // -- empty interpolation --
    #[test_case(b"" => (ParseError::EmptyInterpolation, (0, 4)) ; "empty_interp")]
    #[test_case(b" " => (ParseError::EmptyInterpolation, (0, 5)) ; "empty_interp_spaces")]
    #[test_case(b" \n " => (ParseError::EmptyInterpolation, (0, 7)) ; "empty_interp_newlines")]
    // -- integer errors --
    #[test_case(b"9223372036854775808" => (ParseError::IntegerOutOfRange, (2, 19)) ; "int_overflow_pos")]
    #[test_case(b" -9223372036854775809" => (ParseError::IntegerOutOfRange, (3, 20)) ; "int_overflow_neg")]
    #[test_case(b"+7" => (ParseError::UnexpectedToken, (2, 1)) ; "plus_prefixed_integer")]
    // -- string errors --
    #[test_case(b"\"unterminated" => (ParseError::UnclosedString, (2, 1)) ; "unclosed_string")]
    // -- keyword errors --
    #[test_case(b"if" => (ParseError::ReservedKeyword{ keyword: "if".into() }, (2, 2)) ; "keyword_if")]
    #[test_case(b"end" => (ParseError::ReservedKeyword{ keyword: "end".into() }, (2, 3)) ; "keyword_end")]
    #[test_case(b"if()" => (ParseError::ReservedKeyword{ keyword: "if".into() }, (2, 2)) ; "keyword_in_call_position")]
    // -- dot access errors --
    #[test_case(b"a." => (ParseError::EmptyField, (3, 1)) ; "trailing_dot")]
    #[test_case(b"a.- " => (ParseError::UnexpectedToken, (4, 1)) ; "dash_space_after_dot")]
    #[test_case(b"a.@" => (ParseError::UnexpectedToken, (4, 1)) ; "invalid_at_after_dot")]
    // -- expression errors --
    #[test_case(b"a b" => (ParseError::UnexpectedTokensAfterExpr, (4, 1)) ; "unexpected_after_expr_var")]
    #[test_case(b"42 7" => (ParseError::UnexpectedTokensAfterExpr, (5, 1)) ; "unexpected_after_expr_int")]
    // -- call errors --
    #[test_case(b"f(a,)" => (ParseError::TrailingComma, (5, 1)) ; "trailing_comma_call")]
    #[test_case(b"f(" => (ParseError::UnclosedCallParen, (3, 1)) ; "unclosed_call_empty")]
    #[test_case(b"f(a" => (ParseError::UnclosedCallParen, (3, 1)) ; "unclosed_call_paren")]
    #[test_case(b"f(a, b" => (ParseError::UnclosedCallParen, (3, 1)) ; "unclosed_call_with_args")]
    #[test_case(b"f(a@" => (ParseError::UnexpectedToken, (5, 1)) ; "unexpected_token_in_call")]
    // -- list errors --
    #[test_case(b"[a,]" => (ParseError::TrailingComma, (4, 1)) ; "trailing_comma_list")]
    #[test_case(b"[" => (ParseError::UnclosedListBracket, (2, 1)) ; "unclosed_list_empty")]
    #[test_case(b"[a" => (ParseError::UnclosedListBracket, (2, 1)) ; "unclosed_list_with_element")]
    #[test_case(b"[a, b" => (ParseError::UnclosedListBracket, (2, 1)) ; "unclosed_list_with_multiple_elements")]
    #[test_case(b"[a@" => (ParseError::UnexpectedToken, (4, 1)) ; "unexpected_token_in_list")]
    // -- generic errors --
    #[test_case(b"@" => (ParseError::UnexpectedToken, (2, 1)) ; "unexpected_token")]
    fn parse_error(src: &[u8]) -> (ParseError, (usize, usize)) {
        let src = [b"{{", src, b"}}"].concat();
        let Error::Parse { err, span } = parse(scan(&src).unwrap(), &src).unwrap_err() else {
            panic!("expected parse error");
        };
        (err, (span.offset(), span.len()))
    }
}

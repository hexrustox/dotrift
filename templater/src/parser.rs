use std::ops::Range;

use miette::SourceSpan;

use crate::{
    ast::{Branch, Expr, Node},
    error::{Error, ParseError},
    scanner::Token,
    util::{is_whitespace, source_span},
};

/// Assembles already-trimmed tokens into AST nodes, recognizing `{{ expr }}`
/// interpolations and `{% ... %}` control-flow blocks (`if`/`elif`/`else`/
/// `for`/`end`) via recursive descent over the token stream.
pub(crate) fn parse(tokens: Vec<Token>, source: &[u8]) -> std::result::Result<Vec<Node>, Error> {
    let mut parser = Parser {
        tokens,
        source,
        pos: 0,
    };
    parser.parse_nodes(Stop::None)
}

/// Which block terminators `parse_nodes` may return at (leaving them
/// unconsumed for the caller to match).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stop {
    /// Top level: no terminators are legal — `elif`/`else`/`end` are orphans.
    None,
    /// Inside an `if` block body: `elif`, `else`, and `end` all stop the body.
    IfBlock,
    /// Inside a `for` block body: only `end` stops; `elif`/`else` are errors.
    ForBlock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StmtKind {
    If,
    Elif,
    Else,
    For,
    End,
    Unrecognized,
}

struct Parser<'s> {
    tokens: Vec<Token>,
    source: &'s [u8],
    pos: usize,
}

impl<'s> Parser<'s> {
    /// Collects text/interpolation nodes and dispatches statement openers,
    /// returning when it reaches a terminator permitted by `stop` (left
    /// unconsumed) or the end of the token stream.
    fn parse_nodes(&mut self, stop: Stop) -> std::result::Result<Vec<Node>, Error> {
        let mut nodes = Vec::new();
        while self.pos < self.tokens.len() {
            match &self.tokens[self.pos] {
                Token::Text(range) => {
                    nodes.push(Node::Text(range.clone()));
                    self.pos += 1;
                }
                Token::Barrier => {
                    self.pos += 1;
                }
                Token::Interp { tag, body, .. } => {
                    let expr = parse_interp_body(self.source, body.clone(), tag.clone())?;
                    nodes.push(Node::Interpolate(expr));
                    self.pos += 1;
                }
                Token::Stmt { tag, body, .. } => {
                    let (kind, _kw, rest) = classify_stmt(self.source, tag.clone(), body.clone())?;
                    match kind {
                        StmtKind::If => {
                            let tag = tag.clone();
                            self.pos += 1;
                            nodes.push(self.parse_if(rest, tag)?);
                        }
                        StmtKind::For => {
                            let tag = tag.clone();
                            self.pos += 1;
                            nodes.push(self.parse_for(rest, tag, body.clone())?);
                        }
                        StmtKind::Elif | StmtKind::Else if stop == Stop::IfBlock => {
                            return Ok(nodes);
                        }
                        StmtKind::End if stop != Stop::None => {
                            return Ok(nodes);
                        }
                        StmtKind::Elif => {
                            return Err(Error::parse(
                                ParseError::ElifOutsideIf,
                                source_span(tag.clone()),
                            ));
                        }
                        StmtKind::Else => {
                            return Err(Error::parse(
                                ParseError::ElseOutsideIf,
                                source_span(tag.clone()),
                            ));
                        }
                        StmtKind::End => {
                            return Err(Error::parse(
                                ParseError::OrphanEnd,
                                source_span(tag.clone()),
                            ));
                        }
                        StmtKind::Unrecognized => {
                            return Err(Error::parse(
                                ParseError::UnrecognizedStatement,
                                source_span(body.clone()),
                            ));
                        }
                    }
                }
            }
        }
        Ok(nodes)
    }

    /// Parses an `{% if %}` block whose `if` statement token has already been
    /// consumed. `cond_operand` is the byte range of the `if` operand
    /// (everything after the `if` keyword, in source coordinates).
    fn parse_if(
        &mut self,
        cond_operand: Range<usize>,
        if_tag: Range<usize>,
    ) -> std::result::Result<Node, Error> {
        let cond = parse_expr_from_operand(self.source, cond_operand)?;
        let body = self.parse_nodes(Stop::IfBlock)?;
        let mut branches = vec![Branch { cond, body }];
        let mut else_body = None;

        loop {
            if self.pos >= self.tokens.len() {
                return Err(Error::parse(
                    ParseError::UnclosedBlock,
                    source_span(if_tag.clone()),
                ));
            }
            let (kind, ttag, rest) = self.peek_terminator()?;
            match kind {
                StmtKind::Elif => {
                    if else_body.is_some() {
                        return Err(Error::parse(ParseError::ElifOutsideIf, source_span(ttag)));
                    }
                    self.pos += 1;
                    let cond = parse_expr_from_operand(self.source, rest)?;
                    let body = self.parse_nodes(Stop::IfBlock)?;
                    branches.push(Branch { cond, body });
                }
                StmtKind::Else => {
                    if else_body.is_some() {
                        return Err(Error::parse(ParseError::ElseOutsideIf, source_span(ttag)));
                    }
                    self.pos += 1;
                    if has_non_ws(self.source, rest.clone()) {
                        return Err(Error::parse(
                            ParseError::UnexpectedToken,
                            operand_nonws_span(self.source, rest),
                        ));
                    }
                    let body = self.parse_nodes(Stop::IfBlock)?;
                    else_body = Some(body);
                }
                StmtKind::End => {
                    self.pos += 1;
                    if has_non_ws(self.source, rest.clone()) {
                        return Err(Error::parse(
                            ParseError::UnexpectedToken,
                            operand_nonws_span(self.source, rest),
                        ));
                    }
                    break;
                }
                // Openers / unrecognized can't reach here: parse_nodes would
                // have dispatched them or errored.
                _ => unreachable!("parse_nodes(IfBlock) stops only at terminators"),
            }
        }

        Ok(Node::If {
            branches,
            else_body,
        })
    }

    /// Parses a `{% for var in iter %}` block whose `for` token has been
    /// consumed.
    fn parse_for(
        &mut self,
        operand: Range<usize>,
        tag: Range<usize>,
        body: Range<usize>,
    ) -> std::result::Result<Node, Error> {
        let (var, iter) = parse_for_binding(self.source, operand, body)?;
        let body = self.parse_nodes(Stop::ForBlock)?;
        if self.pos >= self.tokens.len() {
            return Err(Error::parse(
                ParseError::UnclosedBlock,
                source_span(tag.clone()),
            ));
        }
        let (kind, _, rest) = self.peek_terminator()?;
        match kind {
            StmtKind::End => {
                self.pos += 1;
                if has_non_ws(self.source, rest.clone()) {
                    return Err(Error::parse(
                        ParseError::UnexpectedToken,
                        operand_nonws_span(self.source, rest),
                    ));
                }
            }
            _ => unreachable!("parse_nodes(ForBlock) stops only at terminators"),
        }
        Ok(Node::For { var, iter, body })
    }

    /// Reads the terminator token the cursor currently rests on,
    /// classifying it. Returns the statement kind, the tag span, and the
    /// operand range. Caller must ensure `pos` points at a `Stmt` token
    /// (guaranteed because `parse_nodes` only returns at a terminator or EOF,
    /// and the caller has already checked for EOF).
    fn peek_terminator(
        &self,
    ) -> std::result::Result<(StmtKind, Range<usize>, Range<usize>), Error> {
        let Token::Stmt { tag, body, .. } = &self.tokens[self.pos] else {
            unreachable!("parse_nodes stops only at a Stmt terminator or EOF");
        };
        let (kind, _kw, rest) = classify_stmt(self.source, tag.clone(), body.clone())?;
        Ok((kind, tag.clone(), rest))
    }
}

/// Classifies a `{% ... %}` statement body by reading its first whitespace-
/// delimited identifier against the reserved statement keywords. Returns the
/// statement kind, the keyword's byte span, and the operand range (everything
/// after the keyword, untrimmed).
fn classify_stmt(
    source: &[u8],
    tag: Range<usize>,
    body: Range<usize>,
) -> std::result::Result<(StmtKind, Range<usize>, Range<usize>), Error> {
    let bytes = &source[body.clone()];
    if bytes.is_empty() {
        return Err(Error::parse(ParseError::EmptyStatement, source_span(tag)));
    }
    if !is_ident_start(bytes[0]) {
        return Err(Error::parse(
            ParseError::UnrecognizedStatement,
            source_span(body),
        ));
    }
    let mut i = 0;
    while i < bytes.len() && is_ident_byte(bytes[i]) {
        i += 1;
    }
    let kw_bytes = &bytes[..i];
    let kw_range = body.start..body.start + i;
    let rest = body.start + i..body.end;
    let kind = match kw_bytes {
        b"if" => StmtKind::If,
        b"elif" => StmtKind::Elif,
        b"else" => StmtKind::Else,
        b"for" => StmtKind::For,
        b"end" => StmtKind::End,
        _ => StmtKind::Unrecognized,
    };
    Ok((kind, kw_range, rest))
}

/// Parses an expression operand occupying the full `operand` byte range,
/// rejecting empty operands or trailing tokens after the expression.
fn parse_expr_from_operand(
    source: &[u8],
    operand: Range<usize>,
) -> std::result::Result<Expr, Error> {
    if operand.start >= operand.end {
        return Err(Error::parse(
            ParseError::MissingCondition,
            source_span(operand.start..operand.start + 1),
        ));
    }
    let mut state = ParserState {
        source,
        body: operand,
        pos: 0,
    };
    let expr = state.parse_expr()?;
    let skipped = state.skip_ws();
    if state.has_remaining() {
        return Err(Error::parse(
            if skipped {
                ParseError::UnexpectedTokensAfterExpr
            } else {
                ParseError::UnexpectedToken
            },
            state.span(state.pos..state.pos + 1),
        ));
    }
    Ok(expr)
}

/// Parses `{% for <var> in <iter> %}` from the operand bytes after `for`.
fn parse_for_binding(
    source: &[u8],
    operand: Range<usize>,
    body: Range<usize>,
) -> std::result::Result<(Range<usize>, Expr), Error> {
    let mut state = ParserState {
        source,
        body: operand.clone(),
        pos: 0,
    };
    state.skip_ws();
    let bytes = state.bytes();

    let var = if state.pos >= bytes.len() {
        return Err(Error::parse(ParseError::EmptyFor, source_span(body)));
    } else if !is_ident_start(bytes[state.pos]) {
        return Err(Error::parse(
            ParseError::InvalidVariable,
            state.span(state.pos..state.pos + 1),
        ));
    } else {
        let (range, is_keyword) = state.parse_identifier()?;
        if is_keyword {
            let keyword = std::str::from_utf8(&state.source[range.clone()])
                .expect("identifier is ascii")
                .to_owned();
            return Err(Error::parse(
                ParseError::ReservedKeyword { keyword },
                source_span(range),
            ));
        }
        range
    };

    if state.pos < bytes.len() && !is_whitespace(bytes[state.pos]) {
        return Err(Error::Parse {
            err: ParseError::UnexpectedToken,
            span: state.span(state.pos..state.pos + 1),
        });
    }

    state.skip_ws();
    let start = state.pos;
    while state.pos < bytes.len() && !is_whitespace(bytes[state.pos]) {
        state.pos += 1;
    }
    let range = body_range(&state.body, start..state.pos);
    let in_bytes = &state.source[range.clone()];
    if in_bytes != b"in" {
        return Err(Error::parse(
            ParseError::MissingIn,
            source_span(if range.is_empty() {
                range.start..range.start + 1
            } else {
                range
            }),
        ));
    }

    state.skip_ws();
    if state.pos >= bytes.len() {
        return Err(Error::parse(
            ParseError::MissingIterable,
            state.span(state.pos..state.pos + 1),
        ));
    }
    let iter = state.parse_expr()?;

    let skipped = state.skip_ws();
    if state.has_remaining() {
        return Err(Error::parse(
            if skipped {
                ParseError::UnexpectedTokensAfterExpr
            } else {
                ParseError::UnexpectedToken
            },
            state.span(state.pos..state.pos + 1),
        ));
    }
    Ok((var, iter))
}

/// True when the byte range contains any non-whitespace byte.
fn has_non_ws(source: &[u8], range: Range<usize>) -> bool {
    source[range].iter().any(|&b| !is_whitespace(b))
}

/// Span of the first non-whitespace byte in `range`, for error attribution on
/// `else` operands that illegally take a condition. Falls back to an empty
/// span at the operand start when the range is all whitespace (where the
/// caller only calls this when there *is* a non-ws byte).
fn operand_nonws_span(source: &[u8], range: Range<usize>) -> SourceSpan {
    let bytes = &source[range.clone()];
    for (i, &b) in bytes.iter().enumerate() {
        if !is_whitespace(b) {
            return source_span(range.start + i..range.start + i + 1);
        }
    }
    source_span(range.start..range.start)
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
    let skipped = state.skip_ws();
    if state.pos < state.len() {
        return Err(Error::parse(
            if skipped {
                ParseError::UnexpectedTokensAfterExpr
            } else {
                ParseError::UnexpectedToken
            },
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

    /// True when there are still bytes to consume after the current position.
    fn has_remaining(&self) -> bool {
        self.pos < self.len()
    }

    fn bytes(&self) -> &'s [u8] {
        &self.source[self.body.clone()]
    }

    fn span(&self, rel: Range<usize>) -> SourceSpan {
        source_span(self.body.start + rel.start..self.body.start + rel.end)
    }

    fn skip_ws(&mut self) -> bool {
        let bytes = self.bytes();
        let start = self.pos;
        while self.pos < bytes.len() && is_whitespace(bytes[self.pos]) {
            self.pos += 1;
        }
        start != self.pos
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
        let is_keyword = matches!(
            ident,
            b"true" | b"false" | b"if" | b"elif" | b"else" | b"for" | b"in" | b"end"
        );
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
                    ParseError::UnclosedFunction,
                    self.span(lparen..lparen + 1),
                ));
            }
            if bytes[self.pos] == b')' {
                self.pos += 1;
                break;
            }

            args.push(self.parse_expr()?);

            let skipped = self.skip_ws();
            let bytes = self.bytes();
            if self.pos >= bytes.len() {
                return Err(Error::parse(
                    ParseError::UnclosedFunction,
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
                        if skipped {
                            ParseError::UnexpectedTokensAfterExpr
                        } else {
                            ParseError::UnexpectedToken
                        },
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
                    ParseError::UnclosedList,
                    self.span(lbracket..lbracket + 1),
                ));
            }
            if bytes[self.pos] == b']' {
                self.pos += 1;
                break;
            }

            elements.push(self.parse_expr()?);

            let skipped = self.skip_ws();
            let bytes = self.bytes();
            if self.pos >= bytes.len() {
                return Err(Error::parse(
                    ParseError::UnclosedList,
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
                        if skipped {
                            ParseError::UnexpectedTokensAfterExpr
                        } else {
                            ParseError::UnexpectedToken
                        },
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
    #[test_case(b"0" => expr!(int 0, 2..3) ; "integer_zero")]
    #[test_case(b"42" => expr!(int 42, 2..4) ; "integer")]
    #[test_case(b" -0" => expr!(int 0, 3..5) ; "negative_zero")]
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
    #[test_case(b"_foo123" => expr!(var 2..9) ; "identifier_with_trailing_digits")]
    #[test_case(b"true0" => expr!(var 2..7) ; "identifier_that_is_keyword_prefix")]
    // -- dot access --
    #[test_case(b"obj.field" => expr!(dot expr!(var 2..5), 6..11) ; "dot_access")]
    #[test_case(b"a.b.c" => expr!(dot expr!(dot expr!(var 2..3), 4..5), 6..7) ; "chained_dot")]
    #[test_case(b"\"s\".length" => expr!(dot expr!(str 3..4, 2..5), 6..12) ; "dot_access_on_string_literal")]
    // -- indexing --
    #[test_case(b"list.0" => expr!(idx expr!(var 2..6), 0, 7..8) ; "integer_index")]
    #[test_case(b"list.-1" => expr!(idx expr!(var 2..6), -1, 7..9) ; "negative_index")]
    #[test_case(b"a.0.b" => expr!(dot expr!(idx expr!(var 2..3), 0, 4..5), 6..7) ; "index_then_dot")]
    #[test_case(b"obj.a.b.0" => expr!(idx expr!(dot expr!(dot expr!(var 2..5), 6..7), 8..9), 0, 10..11) ; "deep_chain_ending_index")]
    // -- function calls: basic --
    #[test_case(b"fn()" => expr!(call 2..4, 4..6) ; "zero_arg_call")]
    #[test_case(b"f(a,b)" => expr!(call 2..3, 3..8, expr!(var 4..5), expr!(var 6..7)) ; "call_no_whitespace")]
    #[test_case(b"f(a, b)" => expr!(call 2..3, 3..9, expr!(var 4..5), expr!(var 7..8)) ; "two_arg_call")]
    #[test_case(b"f( a , b )" => expr!(call 2..3, 3..12, expr!(var 5..6), expr!(var 9..10)) ; "call_whitespace_inside_parens")]
    #[test_case(b"f\n()" => expr!(call 2..3, 4..6) ; "call_newline_before_paren")]
    #[test_case(b"fn ( )" => expr!(call 2..4, 5..8) ; "call_whitespace_between_name_and_parens")]
    #[test_case(b"f(-1)" => expr!(call 2..3, 3..7, expr!(int -1, 4..6)) ; "call_negative_argument")]
    #[test_case(b"join(\":\", a, b)" => expr!(call 2..6, 6..17, expr!(str 8..9, 7..10), expr!(var 12..13), expr!(var 15..16)) ; "call_mixed_literal_var")]
    // -- function calls: nesting --
    #[test_case(b"eq(gt(x, 3), y)" => expr!(call 2..4, 4..17, expr!(call 5..7, 7..13, expr!(var 8..9), expr!(int 3, 11..12)), expr!(var 15..16)) ; "nested_call")]
    #[test_case(b"f(g(h(x)))" => expr!(call 2..3, 3..12, expr!(call 4..5, 5..11, expr!(call 6..7, 7..10, expr!(var 8..9)))) ; "deeply_nested_calls")]
    // -- function calls: postfix --
    #[test_case(b"fn().field" => expr!(dot expr!(call 2..4, 4..6), 7..12) ; "call_then_dot")]
    #[test_case(b"fn().0" => expr!(idx expr!(call 2..4, 4..6), 0, 7..8) ; "call_then_index")]
    // -- lists: basic --
    #[test_case(b"[]" => expr!(list 2..4;) ; "empty_list")]
    #[test_case(b"[x]" => expr!(list 2..5; expr!(var 3..4)) ; "list_single_element")]
    #[test_case(b"[a, b]" => expr!(list 2..8; expr!(var 3..4), expr!(var 6..7)) ; "list_two_elements")]
    #[test_case(b"[\"x\", 42]" => expr!(list 2..11; expr!(str 4..5, 3..6), expr!(int 42, 8..10)) ; "list_mixed")]
    #[test_case(b"[true, false]" => expr!(list 2..15; expr!(bool true, 3..7), expr!(bool false, 9..14)) ; "list_boolean_literals")]
    #[test_case(b"[-1, 2]" => expr!(list 2..9; expr!(int -1, 3..5), expr!(int 2, 7..8)) ; "list_negative_integers")]
    // -- lists: whitespace --
    #[test_case(b"[  ]" => expr!(list 2..6;) ; "empty_list_with_spaces")]
    #[test_case(b"[\"a\" , \"b\"]" => expr!(list 2..13; expr!(str 4..5, 3..6), expr!(str 10..11, 9..12)) ; "list_whitespace_inside")]
    #[test_case(b"[\n a,\n b\n]" => expr!(list 2..12; expr!(var 5..6), expr!(var 9..10)) ; "list_multiline")]
    // -- lists: nesting --
    #[test_case(b"[[]]" => expr!(list 2..6; expr!(list 3..5;)) ; "nested_list")]
    #[test_case(b"[a, [b, c], d]" => expr!(list 2..16; expr!(var 3..4), expr!(list 6..12; expr!(var 7..8), expr!(var 10..11)), expr!(var 14..15)) ; "list_nested_nonempty")]
    #[test_case(b"[[a], [b]]" => expr!(list 2..12; expr!(list 3..6; expr!(var 4..5)), expr!(list 8..11; expr!(var 9..10))) ; "list_of_lists")]
    #[test_case(b"[[],[]]" => expr!(list 2..9; expr!(list 3..5;), expr!(list 6..8;)) ; "list_of_two_empty_lists")]
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

    #[test_case(b"{% if yes %}Y{% end %}" => Node::If {
        branches: vec![Branch { cond: expr!(var 6..9), body: vec![Node::Text(12..13)] }],
        else_body: None,
    } ; "if_simple")]
    #[test_case(b"{% if yes %}Y{% else %}N{% end %}" => Node::If {
        branches: vec![Branch { cond: expr!(var 6..9), body: vec![Node::Text(12..13)] }],
        else_body: Some(vec![Node::Text(23..24)]),
    } ; "if_with_else")]
    #[test_case(b"{% if true %}{% else %}{% end %}" => Node::If {
        branches: vec![Branch { cond: expr!(bool true, 6..10), body: vec![] }],
        else_body: Some(vec![]),
    } ; "if_empty_else")]
    #[test_case(b"{% for x in list %}{% end %}" => Node::For {
        var: 7..8,
        iter: expr!(var 12..16),
        body: vec![],
    } ; "for_empty_body")]
    #[test_case(b"{% for x in list %}hello{% end %}" => Node::For {
        var: 7..8,
        iter: expr!(var 12..16),
        body: vec![Node::Text(19..24)],
    } ; "for_text_body")]
    #[test_case(b"{% for x in f() %}{% end %}" => Node::For {
        var: 7..8,
        iter: expr!(call 12..13, 13..15),
        body: vec![],
    } ; "for_funcall_iterable")]
    #[test_case(b"{% for x in [1, 2] %}{{x}}{% end %}" => Node::For {
        var: 7..8,
        iter: expr!(list 12..18; expr!(int 1, 13..14), expr!(int 2, 16..17)),
        body: vec![Node::Interpolate(expr!(var 23..24))],
    } ; "for_list_literal_iterable")]
    #[test_case(b"{% if true %}{% end %}" => Node::If {
        branches: vec![Branch { cond: expr!(bool true, 6..10), body: vec![] }],
        else_body: None,
    } ; "if_only_bool_true")]
    #[test_case(b"{% if true %}{{x}}{% end %}" => Node::If {
        branches: vec![Branch { cond: expr!(bool true, 6..10), body: vec![Node::Interpolate(expr!(var 15..16))] }],
        else_body: None,
    } ; "if_only_with_interpolation")]
    #[test_case(b"{% if no %}A{% elif yes %}B{% end %}" => Node::If {
        branches: vec![
            Branch { cond: expr!(var 6..8), body: vec![Node::Text(11..12)] },
            Branch { cond: expr!(var 20..23), body: vec![Node::Text(26..27)] },
        ],
        else_body: None,
    } ; "if_elif")]
    #[test_case(b"{% if a %}{% elif b %}{% elif c %}{% end %}" => Node::If {
        branches: vec![
            Branch { cond: expr!(var 6..7), body: vec![] },
            Branch { cond: expr!(var 18..19), body: vec![] },
            Branch { cond: expr!(var 30..31), body: vec![] },
        ],
        else_body: None,
    } ; "if_multiple_elif_empty_bodies")]
    #[test_case(b"{% if a %}A{% elif b %}B{% elif c %}C{% else %}D{% end %}" => Node::If {
        branches: vec![
            Branch { cond: expr!(var 6..7), body: vec![Node::Text(10..11)] },
            Branch { cond: expr!(var 19..20), body: vec![Node::Text(23..24)] },
            Branch { cond: expr!(var 32..33), body: vec![Node::Text(36..37)] }
        ],
        else_body: Some(vec![Node::Text(47..48)]),
    } ; "if_multi_elif_with_else")]
    #[test_case(b"{% if a %}{% if b %}{% end %}{% end %}" => Node::If {
        branches: vec![Branch { cond: expr!(var 6..7), body: vec![
            Node::If {
                branches: vec![Branch { cond: expr!(var 16..17), body: vec![]}],
                else_body: None
            }
        ] }],
        else_body: None,
    } ; "if_nested_if")]
    #[test_case(b"{% for x in list %}{% if true %}{{ x }}{% end %}{% end %}" => Node::For {
        var: 7..8,
        iter: expr!(var 12..16),
        body: vec![Node::If {
            branches: vec![Branch {
                cond: expr!(bool true, 25..29),
                body: vec![Node::Interpolate(expr!(var 35..36))],
            }],
            else_body: None,
        }],
    } ; "for_with_nested_if")]
    fn parse_stmt_node(src: &[u8]) -> Node {
        parse(scan(src).unwrap(), src).unwrap().pop().unwrap()
    }

    // -- empty / whitespace --
    #[test_case(b"" => (ParseError::EmptyInterpolation, (0, 4)) ; "empty_interp")]
    #[test_case(b" " => (ParseError::EmptyInterpolation, (0, 5)) ; "empty_interp_spaces")]
    #[test_case(b" \n " => (ParseError::EmptyInterpolation, (0, 7)) ; "empty_interp_newlines")]
    // -- integer errors --
    #[test_case(b"9223372036854775808" => (ParseError::IntegerOutOfRange, (2, 19)) ; "int_overflow_pos")]
    #[test_case(b" -9223372036854775809" => (ParseError::IntegerOutOfRange, (3, 20)) ; "int_overflow_neg")]
    #[test_case(b"+7" => (ParseError::UnexpectedToken, (2, 1)) ; "plus_prefixed_integer")]
    #[test_case(b" - 7" => (ParseError::UnexpectedTokensAfterExpr, (5, 1)) ; "minus_sign_separated_from_digits")]
    #[test_case(b"42x" => (ParseError::UnexpectedToken, (4, 1)) ; "int_immediately_followed_by_ident")]
    // -- string errors --
    #[test_case(b"\"unterminated" => (ParseError::UnclosedString, (2, 1)) ; "unclosed_string")]
    // -- reserved keywords --
    #[test_case(b"if" => (ParseError::ReservedKeyword{ keyword: "if".into() }, (2, 2)) ; "keyword_if")]
    #[test_case(b"end" => (ParseError::ReservedKeyword{ keyword: "end".into() }, (2, 3)) ; "keyword_end")]
    #[test_case(b"in" => (ParseError::ReservedKeyword { keyword: "in".into() }, (2, 2)) ; "keyword_in_expr")]
    #[test_case(b"elif" => (ParseError::ReservedKeyword { keyword: "elif".into() }, (2, 4)) ; "keyword_elif_expr")]
    #[test_case(b"else" => (ParseError::ReservedKeyword { keyword: "else".into() }, (2, 4)) ; "keyword_else_expr")]
    #[test_case(b"for" => (ParseError::ReservedKeyword { keyword: "for".into() }, (2, 3)) ; "keyword_for_expr")]
    #[test_case(b"if()" => (ParseError::ReservedKeyword{ keyword: "if".into() }, (2, 2)) ; "keyword_in_call_position")]
    #[test_case(b"if(x)" => (ParseError::ReservedKeyword { keyword: "if".into() }, (2, 2)) ; "keyword_function_name")]
    #[test_case(b"f(in)" => (ParseError::ReservedKeyword { keyword: "in".into() }, (4, 2)) ; "keyword_as_call_arg")]
    #[test_case(b"[if]" => (ParseError::ReservedKeyword { keyword: "if".into() }, (3, 2)) ; "keyword_as_list_element")]
    // -- dot / index errors --
    #[test_case(b"a." => (ParseError::EmptyField, (3, 1)) ; "trailing_dot")]
    #[test_case(b"a.- " => (ParseError::UnexpectedToken, (4, 1)) ; "dash_space_after_dot")]
    #[test_case(b"a.@" => (ParseError::UnexpectedToken, (4, 1)) ; "invalid_at_after_dot")]
    // -- unexpected tokens after expression --
    #[test_case(b"a b" => (ParseError::UnexpectedTokensAfterExpr, (4, 1)) ; "unexpected_after_expr_var")]
    #[test_case(b"42 7" => (ParseError::UnexpectedTokensAfterExpr, (5, 1)) ; "unexpected_after_expr_int")]
    #[test_case(b"@" => (ParseError::UnexpectedToken, (2, 1)) ; "unexpected_token")]
    #[test_case(b"var@" => (ParseError::UnexpectedToken, (5, 1)) ; "unexpected_token_var_at")]
    // -- unclosed function calls --
    #[test_case(b"f(" => (ParseError::UnclosedFunction, (3, 1)) ; "unclosed_call_empty")]
    #[test_case(b"f(a" => (ParseError::UnclosedFunction, (3, 1)) ; "unclosed_call_paren")]
    #[test_case(b"f(a, b" => (ParseError::UnclosedFunction, (3, 1)) ; "unclosed_call_with_args")]
    #[test_case(b"f(a@" => (ParseError::UnexpectedToken, (5, 1)) ; "unexpected_token_in_call")]
    #[test_case(b"f(,)" => (ParseError::UnexpectedToken, (4, 1)) ; "call_comma_with_no_arg")]
    #[test_case(b"f(a b" => (ParseError::UnexpectedTokensAfterExpr, (6, 1)) ; "call_missing_paren_with_extra_token")]
    #[test_case(b"f(a,)" => (ParseError::TrailingComma, (5, 1)) ; "trailing_comma_call")]
    // -- unclosed lists --
    #[test_case(b"[" => (ParseError::UnclosedList, (2, 1)) ; "unclosed_list_empty")]
    #[test_case(b"[a" => (ParseError::UnclosedList, (2, 1)) ; "unclosed_list_with_element")]
    #[test_case(b"[a, b" => (ParseError::UnclosedList, (2, 1)) ; "unclosed_list_with_multiple_elements")]
    #[test_case(b"[a@" => (ParseError::UnexpectedToken, (4, 1)) ; "unexpected_token_in_list")]
    #[test_case(b"[a,]" => (ParseError::TrailingComma, (4, 1)) ; "trailing_comma_list")]
    #[test_case(b"[a b]" => (ParseError::UnexpectedTokensAfterExpr, (5, 1)) ; "list_missing_comma")]
    fn parse_interp_error(src: &[u8]) -> (ParseError, (usize, usize)) {
        let src = [b"{{", src, b"}}"].concat();
        let Error::Parse { err, span } = parse(scan(&src).unwrap(), &src).unwrap_err() else {
            panic!("expected parse error");
        };
        (err, (span.offset(), span.len()))
    }

    // -- empty / invalid statements --
    #[test_case(b"" => (ParseError::EmptyStatement, (0, 4)) ; "empty_statement")]
    #[test_case(b" " => (ParseError::EmptyStatement, (0, 5)) ; "whitespace_statement")]
    #[test_case(b"@" => (ParseError::UnrecognizedStatement, (2, 1)) ; "unrecognized_statement_at")]
    #[test_case(b"ifx" => (ParseError::UnrecognizedStatement, (2, 3)) ; "unrecognized_statement_ifx")]
    #[test_case(b"forx in y" => (ParseError::UnrecognizedStatement, (2, 9)) ; "unrecognized_statement_forx")]
    #[test_case(b"123" => (ParseError::UnrecognizedStatement, (2, 3)) ; "statement_starts_with_digit")]
    #[test_case(b"IF x" => (ParseError::UnrecognizedStatement, (2, 4)) ; "statement_uppercase_keyword")]
    // -- if errors --
    #[test_case(b"if" => (ParseError::MissingCondition, (4, 1)) ; "if_missing_condition")]
    #[test_case(b"if!" => (ParseError::UnexpectedToken, (4, 1)) ; "if_unexpected_token_bang")]
    #[test_case(b"if x y" => (ParseError::UnexpectedTokensAfterExpr, (7, 1)) ; "if_unexpected_tokens_after_condition")]
    #[test_case(b"if \"unclosed" => (ParseError::UnclosedString, (5, 1)) ; "if_unclosed_string_condition")]
    #[test_case(b"if 9223372036854775808" => (ParseError::IntegerOutOfRange, (5, 19)) ; "if_int_overflow_condition")]
    #[test_case(b"if in" => (ParseError::ReservedKeyword { keyword: "in".into() }, (5, 2)) ; "if_reserved_keyword_condition")]
    #[test_case(b"if a." => (ParseError::EmptyField, (6, 1)) ; "if_trailing_dot_in_condition")]
    #[test_case(b"if a@" => (ParseError::UnexpectedToken, (6, 1)) ; "if_unexpected_token_at")]
    // -- for errors --
    #[test_case(b"for" => (ParseError::EmptyFor, (2, 3)) ; "for_missing_binding")]
    #[test_case(b"for $" => (ParseError::InvalidVariable, (6, 1)) ; "for_invalid_variable_dollar")]
    #[test_case(b"for x$" => (ParseError::UnexpectedToken, (7, 1)) ; "for_invalid_variable_trailing")]
    #[test_case(b"for x" => (ParseError::MissingIn, (7, 1)) ; "for_missing_in")]
    #[test_case(b"for x x" => (ParseError::MissingIn, (8, 1)) ; "for_missing_in_second_word")]
    #[test_case(b"for x on list" => (ParseError::MissingIn, (8, 2)) ; "for_wrong_word_instead_of_in")]
    #[test_case(b"for x in" => (ParseError::MissingIterable, (10, 1)) ; "for_missing_iterable")]
    #[test_case(b"for x in   " => (ParseError::MissingIterable, (10, 1)) ; "for_missing_iterable_with_trailing_ws")]
    #[test_case(b"for x in y x" => (ParseError::UnexpectedTokensAfterExpr, (13, 1)) ; "for_unexpected_tokens_after_iterable")]
    #[test_case(b"for x in y$" => (ParseError::UnexpectedToken, (12, 1)) ; "for_unexpected_token_in_iterable")]
    #[test_case(b"for if in list" => (ParseError::ReservedKeyword { keyword: "if".into() }, (6, 2)) ; "for_reserved_keyword_var")]
    #[test_case(b"for true in list" => (ParseError::ReservedKeyword { keyword: "true".into() }, (6, 4)) ; "for_bool_literal_var")]
    #[test_case(b"for 1 in list" => (ParseError::InvalidVariable, (6, 1)) ; "for_digit_start_var")]
    #[test_case(b"for x in in" => (ParseError::ReservedKeyword { keyword: "in".into() }, (11, 2)) ; "for_reserved_keyword_iterable")]
    fn parse_stmt_error(src: &[u8]) -> (ParseError, (usize, usize)) {
        let src = [b"{%", src, b"%}"].concat();
        let Error::Parse { err, span } = parse(scan(&src).unwrap(), &src).unwrap_err() else {
            panic!("expected parse error");
        };
        (err, (span.offset(), span.len()))
    }

    // -- orphan terminators --
    #[test_case(b"{%end%}" => (ParseError::OrphanEnd, (0, 7)) ; "orphan_end")]
    #[test_case(b"{%elif%}" => (ParseError::ElifOutsideIf, (0, 8)) ; "orphan_elif")]
    #[test_case(b"{%else%}" => (ParseError::ElseOutsideIf, (0, 8)) ; "orphan_else")]
    #[test_case(b"{% if a %}{% end %}{% elif b %}{% end %}" => (ParseError::ElifOutsideIf, (19, 12)) ; "elif_after_closed_if")]
    #[test_case(b"{% if a %}A{% else %}B{% end %}{% end %}" => (ParseError::OrphanEnd, (31, 9)) ; "extra_end_after_closed_if")]
    // -- elif / else in wrong blocks --
    #[test_case(b"{% if a %}A{% else %}B{% elif c %}C{% end %}" => (ParseError::ElifOutsideIf, (22, 12)) ; "elif_after_else")]
    #[test_case(b"{% if a %}A{% else %}B{% else %}C{% end %}" => (ParseError::ElseOutsideIf, (22, 10)) ; "else_after_else")]
    #[test_case(b"{% for x in list %}{% elif y %}{% end %}" => (ParseError::ElifOutsideIf, (19, 12)) ; "elif_in_for")]
    #[test_case(b"{% for x in list %}{% else %}{% end %}" => (ParseError::ElseOutsideIf, (19, 10)) ; "else_in_for")]
    // -- unexpected tokens on terminators --
    #[test_case(b"{%if true%}{%end x%}" => (ParseError::UnexpectedToken, (17, 1)) ; "end_with_operand")]
    #[test_case(b"{% for x in list %}{% end x %}" => (ParseError::UnexpectedToken, (26, 1)) ; "for_end_with_operand")]
    #[test_case(b"{%if true%}{%else x%}" => (ParseError::UnexpectedToken, (18, 1)) ; "else_with_operand")]
    #[test_case(b"{%if true%}{%else a b c%}" => (ParseError::UnexpectedToken, (18, 1)) ; "else_with_multiple_operands")]
    // -- unclosed blocks --
    #[test_case(b"{% if true %}text" => (ParseError::UnclosedBlock, (0, 13)) ; "unclosed_if")]
    #[test_case(b"{% for x in list %}text" => (ParseError::UnclosedBlock, (0, 19)) ; "unclosed_for")]
    #[test_case(b"{% if true %}{% else %}" => (ParseError::UnclosedBlock, (0, 13)) ; "if_else_missing_end")]
    #[test_case(b"{% for x in list %}{% if true %}text{% end %}" => (ParseError::UnclosedBlock, (0, 19)) ; "for_body_with_unclosed_inner_if")]
    fn parse_stmt_nodes_error(src: &[u8]) -> (ParseError, (usize, usize)) {
        let Error::Parse { err, span } = parse(scan(src).unwrap(), src).unwrap_err() else {
            panic!("expected parse error");
        };
        (err, (span.offset(), span.len()))
    }
}

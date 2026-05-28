use std::ops::Range;

use crate::ast::{Expr, ExprKind, Node};
use crate::error::{ParseError, ParseErrorKind};
use crate::scanner::RawToken;

#[derive(Debug, Clone, PartialEq)]
enum TokenKind {
    Ident(String),
    Str(String),
    Int(i64),
    True,
    False,
    Dot,
    Comma,
    LParen,
    RParen,
    LBracket,
    RBracket,
    If,
    Elif,
    Else,
    For,
    End,
    In,
}

#[derive(Debug, Clone, PartialEq)]
struct Token {
    kind: TokenKind,
    range: Range<usize>,
}

fn tag_error(kind: ParseErrorKind, range: &Range<usize>, source: &[u8]) -> ParseError {
    ParseError::new(kind, range.start.saturating_sub(2), range.len() + 4, source)
}

pub fn parse(tokens: &[RawToken], source: &[u8]) -> Result<Vec<Node>, ParseError> {
    let mut nodes = Vec::new();
    let mut i = 0;

    while i < tokens.len() {
        match &tokens[i] {
            RawToken::Text(range) => {
                nodes.push(Node::Text(range.clone()));
                i += 1;
            }
            RawToken::Interpolate(range) => {
                let expr = parse_interpolate(source, range)?;
                nodes.push(Node::Interpolate(expr));
                i += 1;
            }
            RawToken::Statement(range) => {
                let opening = range.clone();
                let toks = tokenize(source, range.clone())?;
                let kw = toks
                    .first()
                    .ok_or_else(|| tag_error(ParseErrorKind::EmptyStatement, &opening, source))?;
                let (stmt_nodes, consumed) = match &kw.kind {
                    TokenKind::If => parse_if_block(tokens, source, i)?,
                    TokenKind::For => parse_for_block(tokens, source, i)?,
                    TokenKind::Elif => {
                        return Err(tag_error(ParseErrorKind::StrayElif, &opening, source));
                    }
                    TokenKind::Else => {
                        return Err(tag_error(ParseErrorKind::StrayElse, &opening, source));
                    }
                    TokenKind::End => {
                        return Err(tag_error(ParseErrorKind::StrayEnd, &opening, source));
                    }
                    _ => {
                        return Err(ParseError::new(
                            ParseErrorKind::UnexpectedKeyword,
                            kw.range.start,
                            kw.range.len(),
                            source,
                        ));
                    }
                };
                nodes.extend(stmt_nodes);
                i += consumed;
            }
        }
    }

    Ok(nodes)
}

fn parse_interpolate(source: &[u8], range: &Range<usize>) -> Result<Expr, ParseError> {
    let toks = tokenize(source, range.clone())?;
    let mut pos = 0;
    let expr = parse_postfix(&toks, &mut pos, source, range.start, range.end)?;
    if pos != toks.len() {
        return Err(ParseError::new(
            ParseErrorKind::UnexpectedTokensAfterExpr,
            toks[pos].range.start,
            1,
            source,
        ));
    }
    Ok(expr)
}

fn parse_trailing_expr(
    toks: &[Token],
    start_pos: usize,
    source: &[u8],
    base: usize,
    end: usize,
) -> Result<Expr, ParseError> {
    let mut pos = start_pos;
    let expr = parse_postfix(toks, &mut pos, source, base, end)?;
    if pos != toks.len() {
        return Err(ParseError::new(
            ParseErrorKind::UnexpectedTokensAfterExpr,
            toks[pos].range.start,
            1,
            source,
        ));
    }
    Ok(expr)
}

fn parse_if_block(
    tokens: &[RawToken],
    source: &[u8],
    start: usize,
) -> Result<(Vec<Node>, usize), ParseError> {
    let mut branches: Vec<(Expr, Vec<Node>)> = Vec::new();
    let mut else_branch: Option<Vec<Node>> = None;
    let mut i = start;
    let mut consumed = 0;

    let opening = match tokens.get(start) {
        Some(RawToken::Statement(r)) => r.clone(),
        _ => return Err(ParseError::new(ParseErrorKind::UnclosedBlock, 0, 1, source)),
    };

    loop {
        let raw = match tokens.get(i) {
            Some(RawToken::Statement(range)) => range.clone(),
            None => {
                return Err(tag_error(ParseErrorKind::UnclosedBlock, &opening, source));
            }
            _ => {
                unreachable!()
            }
        };

        let toks = tokenize(source, raw.clone())?;
        let first = toks.first().cloned();

        match first {
            Some(Token {
                kind: TokenKind::If,
                ..
            })
            | Some(Token {
                kind: TokenKind::Elif,
                ..
            }) => {
                let cond = parse_trailing_expr(&toks, 1, source, toks[0].range.end, raw.end)?;
                let (body, body_consumed) =
                    collect_body_until(tokens, source, i + 1, &[TokenKind::Elif, TokenKind::Else])?;
                branches.push((cond, body));
                i += body_consumed + 1;
                consumed += body_consumed + 1;
            }
            Some(Token {
                kind: TokenKind::Else,
                ..
            }) => {
                let (body, body_consumed) = collect_body_until(tokens, source, i + 1, &[])?;
                else_branch = Some(body);
                consumed += body_consumed + 2;
                break;
            }
            Some(Token {
                kind: TokenKind::End,
                ..
            }) => {
                consumed += 1;
                break;
            }
            Some(tok) => {
                return Err(ParseError::new(
                    ParseErrorKind::UnexpectedKeyword,
                    tok.range.start,
                    tok.range.len(),
                    source,
                ));
            }
            None => {
                unreachable!()
            }
        }
    }

    Ok((
        vec![Node::If {
            branches,
            else_branch,
        }],
        consumed,
    ))
}

fn parse_for_block(
    tokens: &[RawToken],
    source: &[u8],
    start: usize,
) -> Result<(Vec<Node>, usize), ParseError> {
    let range = match tokens.get(start) {
        Some(RawToken::Statement(r)) => r.clone(),
        _ => unreachable!(),
    };

    let toks = tokenize(source, range.clone())?;

    if toks.len() < 4 {
        return Err(ParseError::new(
            ParseErrorKind::ExpectedForSyntax,
            toks[0].range.end,
            1,
            source,
        ));
    }

    let var = match &toks[1].kind {
        TokenKind::Ident(name) => name.clone(),
        _ => {
            return Err(ParseError::new(
                ParseErrorKind::ExpectedForVar,
                toks[1].range.start,
                toks[1].range.len(),
                source,
            ));
        }
    };

    if toks[2].kind != TokenKind::In {
        return Err(ParseError::new(
            ParseErrorKind::ExpectedForIn,
            toks[2].range.start,
            toks[2].range.len(),
            source,
        ));
    }

    let collection = parse_trailing_expr(&toks, 3, source, toks[0].range.end, range.end)?;

    let (body, body_consumed) = collect_body_until(tokens, source, start + 1, &[])?;

    let end_pos = start + body_consumed + 1;
    match tokens.get(end_pos) {
        Some(RawToken::Statement(r))
            if tokenize(source, r.clone())?.first().map(|t| &t.kind) == Some(&TokenKind::End) => {}
        _ => {
            return Err(tag_error(ParseErrorKind::UnclosedFor, &range, source));
        }
    }

    Ok((
        vec![Node::For {
            var,
            collection,
            body,
        }],
        body_consumed + 2,
    ))
}

fn collect_body_until(
    tokens: &[RawToken],
    source: &[u8],
    start: usize,
    stop_at: &[TokenKind],
) -> Result<(Vec<Node>, usize), ParseError> {
    let mut body = Vec::new();
    let mut i = start;

    while i < tokens.len() {
        match &tokens[i] {
            RawToken::Statement(range) => {
                let toks = tokenize(source, range.clone())?;
                match toks.first().map(|t| &t.kind) {
                    Some(TokenKind::If) => {
                        let (nodes, consumed) = parse_if_block(tokens, source, i)?;
                        body.extend(nodes);
                        i += consumed - 1;
                    }
                    Some(TokenKind::For) => {
                        let (nodes, consumed) = parse_for_block(tokens, source, i)?;
                        body.extend(nodes);
                        i += consumed - 1;
                    }
                    Some(TokenKind::End) => {
                        return Ok((body, i - start));
                    }
                    Some(tok) if stop_at.contains(tok) => {
                        return Ok((body, i - start));
                    }
                    _ => {
                        unreachable!()
                    }
                }
            }
            RawToken::Interpolate(range) => {
                let expr = parse_interpolate(source, range)?;
                body.push(Node::Interpolate(expr));
            }
            RawToken::Text(range) => {
                body.push(Node::Text(range.clone()));
            }
        }
        i += 1;
    }

    Ok((body, tokens.len() - start))
}

fn tokenize(source: &[u8], range: Range<usize>) -> Result<Vec<Token>, ParseError> {
    macro_rules! push_token {
        ($kind:expr, $r:expr, $p:expr, $tokens:expr) => {{
            $tokens.push(Token {
                kind: $kind,
                range: $r.start + $p..$r.start + $p + 1,
            });
            $p += 1;
        }};
    }

    let bytes = &source[range.clone()];
    let mut tokens = Vec::new();
    let mut pos = 0;
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';

    while pos < bytes.len() {
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }

        match bytes[pos] {
            b'"' => {
                let quote_start = range.start + pos;
                pos += 1;
                let mut s = String::new();
                loop {
                    if pos >= bytes.len() {
                        let mut end = range.start + pos;
                        while end > quote_start && source[end - 1].is_ascii_whitespace() {
                            end -= 1;
                        }
                        return Err(ParseError::new(
                            ParseErrorKind::UnclosedString,
                            quote_start,
                            end - quote_start,
                            source,
                        ));
                    }
                    match bytes[pos] {
                        b'"' => {
                            pos += 1;
                            break;
                        }
                        b'\\' if pos + 1 < bytes.len() => {
                            pos += 1;
                            match bytes[pos] {
                                b'"' => s.push('"'),
                                b'\\' => s.push('\\'),
                                c => {
                                    s.push('\\');
                                    s.push(c as char);
                                }
                            }
                            pos += 1;
                        }
                        b => {
                            s.push(b as char);
                            pos += 1;
                        }
                    }
                }
                tokens.push(Token {
                    kind: TokenKind::Str(s),
                    range: quote_start..range.start + pos,
                });
            }
            b'(' => {
                push_token!(TokenKind::LParen, range, pos, tokens);
            }
            b')' => {
                push_token!(TokenKind::RParen, range, pos, tokens);
            }
            b',' => {
                push_token!(TokenKind::Comma, range, pos, tokens);
            }
            b'.' => {
                push_token!(TokenKind::Dot, range, pos, tokens);
            }
            b'[' => {
                push_token!(TokenKind::LBracket, range, pos, tokens);
            }
            b']' => {
                push_token!(TokenKind::RBracket, range, pos, tokens);
            }
            b'-' | b'0'..=b'9' => {
                let start = range.start + pos;
                let negative = bytes[pos] == b'-';
                if negative {
                    pos += 1;
                }
                let mut n: i64 = 0;
                while pos < bytes.len() && bytes[pos].is_ascii_digit() {
                    n = n * 10 + (bytes[pos] - b'0') as i64;
                    pos += 1;
                }
                tokens.push(Token {
                    kind: TokenKind::Int(if negative { -n } else { n }),
                    range: start..range.start + pos,
                });
            }
            _ if is_ident(bytes[pos]) => {
                let ident_start = range.start + pos;
                let start = pos;
                pos += 1;
                while pos < bytes.len() && is_ident(bytes[pos]) {
                    pos += 1;
                }
                let ident = &bytes[start..pos];
                let kind = match ident {
                    b"true" => TokenKind::True,
                    b"false" => TokenKind::False,
                    b"if" => TokenKind::If,
                    b"elif" => TokenKind::Elif,
                    b"else" => TokenKind::Else,
                    b"for" => TokenKind::For,
                    b"end" => TokenKind::End,
                    b"in" => TokenKind::In,
                    _ => TokenKind::Ident(String::from_utf8_lossy(ident).into_owned()),
                };
                tokens.push(Token {
                    kind,
                    range: ident_start..range.start + pos,
                });
            }
            _ => {
                return Err(ParseError::new(
                    ParseErrorKind::UnexpectedToken,
                    range.start + pos,
                    1,
                    source,
                ));
            }
        }
    }

    Ok(tokens)
}

fn parse_postfix(
    tokens: &[Token],
    pos: &mut usize,
    source: &[u8],
    base: usize,
    end: usize,
) -> Result<Expr, ParseError> {
    let mut left = parse_primary(tokens, pos, source, base, end)?;

    while *pos < tokens.len() && tokens[*pos].kind == TokenKind::Dot {
        *pos += 1;
        let field = match tokens.get(*pos) {
            Some(Token {
                kind: TokenKind::Ident(name),
                ..
            }) => name.clone(),
            Some(Token {
                kind: TokenKind::Int(n),
                ..
            }) => n.to_string(),
            tok => {
                let at = tok.map_or(base, |t| t.range.start);
                return Err(ParseError::new(
                    ParseErrorKind::ExpectedFieldName,
                    at,
                    1,
                    source,
                ));
            }
        };
        *pos += 1;
        let start = left.range.start;
        let end = tokens[*pos - 1].range.end;
        left = Expr {
            kind: ExprKind::Dot {
                left: Box::new(left),
                field,
            },
            range: start..end,
        };
    }

    Ok(left)
}

fn parse_primary(
    tokens: &[Token],
    pos: &mut usize,
    source: &[u8],
    base: usize,
    end: usize,
) -> Result<Expr, ParseError> {
    struct DelimConfig {
        close_kind: TokenKind,
        unclosed_kind: ParseErrorKind,
        comma_kind: ParseErrorKind,
        open_at: usize,
        unclosed_span_len: usize,
    }

    impl DelimConfig {
        fn list(open_at: usize, span_len: usize) -> Self {
            Self {
                close_kind: TokenKind::RBracket,
                unclosed_kind: ParseErrorKind::UnclosedList,
                comma_kind: ParseErrorKind::ExpectedCommaInList,
                open_at,
                unclosed_span_len: span_len,
            }
        }
        fn group(open_at: usize, span_len: usize) -> Self {
            Self {
                close_kind: TokenKind::RParen,
                unclosed_kind: ParseErrorKind::UnclosedGroup,
                comma_kind: ParseErrorKind::ExpectedCommaBetweenArgs,
                open_at,
                unclosed_span_len: span_len,
            }
        }
    }

    fn parse_delimited_items(
        tokens: &[Token],
        pos: &mut usize,
        source: &[u8],
        base: usize,
        end: usize,
        dc: &DelimConfig,
    ) -> Result<Vec<Expr>, ParseError> {
        let mut items = Vec::new();
        loop {
            if *pos >= tokens.len() {
                return Err(ParseError::new(
                    dc.unclosed_kind.clone(),
                    dc.open_at,
                    dc.unclosed_span_len,
                    source,
                ));
            }
            if tokens[*pos].kind == dc.close_kind {
                *pos += 1;
                break;
            }
            let item = parse_postfix(tokens, pos, source, base, end)?;
            items.push(item);
            if *pos >= tokens.len() {
                return Err(ParseError::new(
                    dc.unclosed_kind.clone(),
                    dc.open_at,
                    dc.unclosed_span_len,
                    source,
                ));
            }
            match &tokens[*pos].kind {
                TokenKind::Comma => {
                    *pos += 1;
                }
                k if *k == dc.close_kind => {
                    *pos += 1;
                    break;
                }
                _ => {
                    return Err(ParseError::new(
                        dc.comma_kind.clone(),
                        tokens[*pos].range.start,
                        1,
                        source,
                    ));
                }
            }
        }
        Ok(items)
    }

    if *pos >= tokens.len() {
        return Err(ParseError::new(
            ParseErrorKind::UnexpectedEndOfExpr,
            base,
            1,
            source,
        ));
    }

    match &tokens[*pos].kind {
        TokenKind::Str(s) => {
            let range = tokens[*pos].range.clone();
            *pos += 1;
            Ok(Expr {
                kind: ExprKind::Str(s.clone()),
                range,
            })
        }
        TokenKind::Int(n) => {
            let range = tokens[*pos].range.clone();
            *pos += 1;
            Ok(Expr {
                kind: ExprKind::Int(*n),
                range,
            })
        }
        TokenKind::True => {
            let range = tokens[*pos].range.clone();
            *pos += 1;
            Ok(Expr {
                kind: ExprKind::Bool(true),
                range,
            })
        }
        TokenKind::False => {
            let range = tokens[*pos].range.clone();
            *pos += 1;
            Ok(Expr {
                kind: ExprKind::Bool(false),
                range,
            })
        }
        TokenKind::LBracket => {
            let start = tokens[*pos].range.start;
            *pos += 1;
            let stop = tokens.last().map_or(start + 1, |t| t.range.end);
            let items = parse_delimited_items(
                tokens,
                pos,
                source,
                base,
                end,
                &DelimConfig::list(start, stop - start),
            )?;
            let end = tokens[*pos - 1].range.end;
            Ok(Expr {
                kind: ExprKind::List(items),
                range: start..end,
            })
        }
        TokenKind::Ident(name) => {
            let name = name.clone();
            let ident_range = tokens[*pos].range.clone();
            *pos += 1;
            if *pos < tokens.len() && tokens[*pos].kind == TokenKind::LParen {
                let lparen_start = tokens[*pos].range.start;
                *pos += 1;
                let args = parse_delimited_items(
                    tokens,
                    pos,
                    source,
                    base,
                    end,
                    &DelimConfig::group(lparen_start, end.saturating_sub(lparen_start) + 1),
                )?;
                Ok(Expr {
                    kind: ExprKind::FnCall { name, args },
                    range: ident_range.start..tokens[*pos - 1].range.end,
                })
            } else {
                Ok(Expr {
                    kind: ExprKind::Var(name),
                    range: ident_range,
                })
            }
        }
        _ => {
            let at = tokens[*pos].range.start;
            Err(ParseError::new(
                ParseErrorKind::UnexpectedToken,
                at,
                1,
                source,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::scanner::scan;

    use super::*;
    use rand::RngExt;
    use test_case::test_case;

    macro_rules! expr {
        (var $name:expr) => {
            Expr {
                kind: ExprKind::Var($name.into()),
                range: 0..0,
            }
        };
        (str $s:expr) => {
            Expr {
                kind: ExprKind::Str($s.into()),
                range: 0..0,
            }
        };
        (int $n:expr) => {
            Expr {
                kind: ExprKind::Int($n),
                range: 0..0,
            }
        };
        (bool $b:expr) => {
            Expr {
                kind: ExprKind::Bool($b),
                range: 0..0,
            }
        };
        (list [$($e:expr),* $(,)?]) => {
            Expr {
                kind: ExprKind::List(vec![$(($e)),*]),
                range: 0..0,
            }
        };
        (call $name:expr, [$($arg:expr),* $(,)?]) => {
            Expr {
                kind: ExprKind::FnCall { name: $name.into(), args: vec![$($arg),*] },
                range: 0..0,
            }
        };
        (dot $e:expr, $field:expr) => {
            Expr {
                kind: ExprKind::Dot { left: Box::new($e), field: $field.into() },
                range: 0..0,
            }
        };
    }

    macro_rules! node {
        (text $range:expr) => { Node::Text($range) };
        (interp $($tok:tt)+) => { Node::Interpolate(expr!($($tok)+)) };
        (if [
            $([$cond:expr => [$($body:expr),* $(,)?]]),* $(,)?
        ]
        $(else [$($else_body:expr),* $(,)?])? $(,)?) => {
            Node::If {
                branches: vec![$(($cond, vec![$($body),*])),*],
                else_branch: None $(.or(Some(vec![$($else_body),*])))?,
            }
        };
        (for $var:literal in $coll:expr => [$($body:expr),* $(,)?]) => {
            Node::For {
                var: $var.into(),
                collection: $coll,
                body: vec![$($body),*],
            }
        };
    }

    #[test_case("" => Vec::<Node>::new(); "empty_input")]
    #[test_case("hello" => vec![node!(text 0..5)]; "plain_text")]
    #[test_case("{{ a }}" => vec![node!(interp var "a")]; "interpolate_var")]
    #[test_case("{{ \"str\" }}" => vec![node!(interp str "str")]; "interpolate_string")]
    #[test_case("{{ 42 }}" => vec![node!(interp int 42)]; "interpolate_int")]
    #[test_case("{{ -1 }}" => vec![node!(interp int -1)]; "interpolate_negative_int")]
    #[test_case("{{ true }}" => vec![node!(interp bool true)]; "interpolate_true")]
    #[test_case("{{ false }}" => vec![node!(interp bool false)]; "interpolate_false")]
    #[test_case("{{ [] }}" => vec![node!(interp list [])]; "interpolate_empty_list")]
    #[test_case("{{ [1,  \"2\",[  3]  ] }}" => vec![node!(interp list [expr!(int 1), expr!(str "2"), expr!(list [expr!(int 3)])])]; "interpolate_nested_list")]
    #[test_case("{{ fn() }}" => vec![node!(interp call "fn", [])]; "interpolate_fn_call_no_args")]
    #[test_case("{{ fn1(a, 2  ,fn2(  \"\"   )) }}" => vec![node!(interp call "fn1", [expr!(var "a"), expr!(int 2), expr!(call "fn2", [expr!(str "")])])]; "interpolate_nested_fn_call")]
    #[test_case("{{ a.b }}" => vec![node!(interp dot expr!(var "a"), "b")]; "interpolate_dot_field")]
    #[test_case("{{ a.b.c }}" => vec![node!(interp dot expr!(dot expr!(var "a"), "b"), "c")]; "interpolate_dot_chain")]
    #[test_case("{{ list.0 }}" => vec![node!(interp dot expr!(var "list"), "0")]; "interpolate_dot_int_index")]
    #[test_case(r#"{{ "hello \"world\" \\ " }}"# => vec![node!(interp str r#"hello "world" \ "#)]; "interpolate_string_escapes")]
    #[test_case("a {{ x }} b" => vec![
        node!(text 0..2),
        node!(interp var "x"),
        node!(text 9..11),
    ]; "mixed_text_and_interpolate")]
    #[test_case("{% if true %}{% end %}" => vec![node!(if [[expr!(bool true) => []]])]; "if_empty_body")]
    #[test_case("{% if a %}{% else %}{% end %}" => vec![node!(
        if [[expr!(var "a") => []]]
        else [],
    )]; "if_else_empty")]
    #[test_case("{% if a %}{% elif b %}{% else %}{% end %}" => vec![node!(
        if [
            [expr!(var "a") => []],
            [expr!(var "b") => []],
        ]
        else [],
    )]; "if_elif_else_empty")]
    #[test_case("{% for x in items %}{% end %}" => vec![node!(for "x" in expr!(var "items") => [])]; "for_empty_body")]
    #[test_case("{% for x in [1, 2] %}{% end %}" => vec![node!(for "x" in expr!(list [expr!(int 1), expr!(int 2)]) => [])]; "for_in_list_empty_body")]
    #[test_case("{% if a %}A{% end %}" => vec![node!(if [[expr!(var "a") => [node!(text 10..11)]]])]; "if_with_text_body")]
    #[test_case("{% for x in items %}X{% end %}" => vec![node!(for "x" in expr!(var "items") => [node!(text 20..21)])]; "for_with_text_body")]
    #[test_case("{% if l1 %}{% if l2 %}{% elif l2 %}{% else %}{% end %}{% elif l1 %}{% for x in items %}{% if l2 %}{% end %}{% end %}{% else %}{% end %}" =>
        vec![
            node!(if [
                [expr!(var "l1") => [
                    node!(if [
                        [expr!(var "l2") => []],
                        [expr!(var "l2") => []],
                    ]
                    else [],
                    ),
                ]],
                [expr!(var "l1") => [
                    node!(for "x" in expr!(var "items") => [
                        node!(if [
                            [expr!(var "l2") => []],
                        ]),
                    ]),
                ]],
            ]
            else [],
        )]; "nested_blocks")]
    fn test_parse(input: &str) -> Vec<Node> {
        let input = input.as_bytes();
        parse(&scan(input).unwrap(), input).unwrap()
    }

    #[test_case("{{ }}" => (ParseErrorKind::UnexpectedEndOfExpr, 2, 1); "unexpected_end_of_expr")]
    #[test_case("{{ @ }}" => (ParseErrorKind::UnexpectedToken, 3, 1); "unexpected_token_at_sign")]
    #[test_case("{{ p% }}" => (ParseErrorKind::UnexpectedToken, 4, 1); "unexpected_token_percent")]
    #[test_case("{{ a b }}" => (ParseErrorKind::UnexpectedTokensAfterExpr, 5, 1); "unexpected_tokens_after_expr")]
    #[test_case("{{ () }}" => (ParseErrorKind::UnexpectedToken, 3, 1); "unexpected_token_empty_paren")]
    #[test_case("{{ a..b }}" => (ParseErrorKind::ExpectedFieldName, 5, 1); "expected_field_name_after_dot")]
    #[test_case("{{ list., }}" => (ParseErrorKind::ExpectedFieldName, 8, 1); "expected_field_name_trailing_dot")]
    #[test_case("{{ \"hello }}" => (ParseErrorKind::UnclosedString, 3, 6); "unclosed_string")]
    #[test_case("{{ [1, 2 }}" => (ParseErrorKind::UnclosedList, 3, 5); "unclosed_list")]
    #[test_case("{{ fn(a, b }}" => (ParseErrorKind::UnclosedGroup, 5, 7); "unclosed_group")]
    #[test_case("{{ [1  2] }}" => (ParseErrorKind::ExpectedCommaInList, 7, 1); "expected_comma_in_list")]
    #[test_case("{{ fn(a b) }}" => (ParseErrorKind::ExpectedCommaBetweenArgs, 8, 1); "expected_comma_between_args")]
    #[test_case("{%%}" => (ParseErrorKind::EmptyStatement, 0, 4); "empty_statement")]
    #[test_case("{% foobar %}" => (ParseErrorKind::UnexpectedKeyword, 3, 6); "unexpected_keyword")]
    #[test_case("{% if %}" => (ParseErrorKind::UnexpectedEndOfExpr, 5, 1); "if_no_condition")]
    #[test_case("{% if true %}" => (ParseErrorKind::UnclosedBlock, 0, 13); "unclosed_if_block")]
    #[test_case("{% elif true %}" => (ParseErrorKind::StrayElif, 0, 15); "stray_elif")]
    #[test_case("{% else true %}" => (ParseErrorKind::StrayElse, 0, 15); "stray_else")]
    #[test_case("{% end %}" => (ParseErrorKind::StrayEnd, 0, 9); "stray_end")]
    #[test_case("{% for %}" => (ParseErrorKind::ExpectedForSyntax, 6, 1); "for_no_var")]
    #[test_case("{% for 123 in items %}" => (ParseErrorKind::ExpectedForVar, 7, 3); "for_non_ident_var")]
    #[test_case("{% for x of items %}" => (ParseErrorKind::ExpectedForIn, 9, 2); "for_missing_in")]
    #[test_case("{% for x in items %}" => (ParseErrorKind::UnclosedFor, 0, 20); "unclosed_for")]
    fn test_error(input: &str) -> (ParseErrorKind, usize, usize) {
        let mut rng = rand::rng();
        let prefix_len = rng.random::<u8>() as usize;
        let aug_input = String::from(" ").repeat(prefix_len) + input;
        let input = aug_input.as_bytes();
        let e = parse(&scan(input).unwrap(), input).unwrap_err();
        let mut e = e.destruct();
        e.1 = e.1.saturating_sub(prefix_len);
        e
    }
}

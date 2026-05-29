use std::ops::Range;

use miette::bail;

use crate::ast::{Expr, ExprKind, Node};
use crate::error::{Error, ErrorKind};
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

fn tag_error(kind: ErrorKind, range: &Range<usize>) -> Error {
    Error::new(kind, range.start.saturating_sub(2), range.len() + 4)
}

pub fn parse(tokens: &[RawToken], source: &[u8]) -> miette::Result<Vec<Node>> {
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
                    .ok_or_else(|| tag_error(ErrorKind::EmptyStatement, &opening))?;
                let (stmt_nodes, consumed) = match &kw.kind {
                    TokenKind::If => parse_if_block(tokens, source, i)?,
                    TokenKind::For => parse_for_block(tokens, source, i)?,
                    TokenKind::Elif => {
                        bail!(tag_error(ErrorKind::StrayElif, &opening));
                    }
                    TokenKind::Else => {
                        bail!(tag_error(ErrorKind::StrayElse, &opening));
                    }
                    TokenKind::End => {
                        bail!(tag_error(ErrorKind::StrayEnd, &opening));
                    }
                    _ => {
                        bail!(Error::new(
                            ErrorKind::UnexpectedKeyword,
                            kw.range.start,
                            kw.range.len(),
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

fn parse_interpolate(source: &[u8], range: &Range<usize>) -> miette::Result<Expr> {
    let toks = tokenize(source, range.clone())?;
    parse_expr_from(&toks, 0, range.start, range.end)
}

fn parse_expr_from(
    toks: &[Token],
    start_pos: usize,
    base: usize,
    end: usize,
) -> miette::Result<Expr> {
    let mut pos = start_pos;
    let expr = parse_postfix(toks, &mut pos, base, end)?;
    if pos != toks.len() {
        bail!(Error::new(
            ErrorKind::UnexpectedTokensAfterExpr,
            toks[pos].range.start,
            1,
        ));
    }
    Ok(expr)
}

fn parse_if_block(
    tokens: &[RawToken],
    source: &[u8],
    start: usize,
) -> miette::Result<(Vec<Node>, usize)> {
    let mut branches: Vec<(Expr, Vec<Node>)> = Vec::new();
    let mut else_branch: Option<Vec<Node>> = None;
    let mut i = start;
    let mut consumed = 0;

    let opening = match tokens.get(start) {
        Some(RawToken::Statement(r)) => r.clone(),
        _ => bail!(Error::new(ErrorKind::UnclosedBlock, 0, 1)),
    };

    loop {
        let raw = match tokens.get(i) {
            Some(RawToken::Statement(range)) => range.clone(),
            None => {
                bail!(tag_error(ErrorKind::UnclosedBlock, &opening));
            }
            _ => bail!("expected statement tag in if-block, got text or interpolate"),
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
                let cond = parse_expr_from(&toks, 1, toks[0].range.end, raw.end)?;
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
                bail!(Error::new(
                    ErrorKind::UnexpectedKeyword,
                    tok.range.start,
                    tok.range.len(),
                ));
            }
            None => bail!("empty statement tag"),
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
) -> miette::Result<(Vec<Node>, usize)> {
    let range = match tokens.get(start) {
        Some(RawToken::Statement(r)) => r.clone(),
        _ => bail!("expected statement tag in for-block, got text or interpolate"),
    };

    let toks = tokenize(source, range.clone())?;

    if toks.len() < 4 {
        bail!(Error::new(
            ErrorKind::ExpectedForSyntax,
            toks[0].range.end,
            1,
        ));
    }

    let var = match &toks[1].kind {
        TokenKind::Ident(name) => name.clone(),
        _ => {
            bail!(Error::new(
                ErrorKind::ExpectedForVar,
                toks[1].range.start,
                toks[1].range.len(),
            ));
        }
    };

    if toks[2].kind != TokenKind::In {
        bail!(Error::new(
            ErrorKind::ExpectedForIn,
            toks[2].range.start,
            toks[2].range.len(),
        ));
    }

    let collection = parse_expr_from(&toks, 3, toks[0].range.end, range.end)?;

    let (body, body_consumed) = collect_body_until(tokens, source, start + 1, &[])?;

    let end_pos = start + body_consumed + 1;
    match tokens.get(end_pos) {
        Some(RawToken::Statement(r))
            if tokenize(source, r.clone())?.first().map(|t| &t.kind) == Some(&TokenKind::End) => {}
        _ => {
            bail!(tag_error(ErrorKind::UnclosedFor, &range));
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
) -> miette::Result<(Vec<Node>, usize)> {
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
                    _ => bail!("unrecognized statement keyword"),
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

fn tokenize(source: &[u8], range: Range<usize>) -> miette::Result<Vec<Token>> {
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
                        bail!(Error::new(
                            ErrorKind::UnclosedString,
                            quote_start,
                            end - quote_start,
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
                bail!(Error::new(ErrorKind::UnexpectedToken, range.start + pos, 1));
            }
        }
    }

    Ok(tokens)
}

fn parse_postfix(
    tokens: &[Token],
    pos: &mut usize,
    base: usize,
    end: usize,
) -> miette::Result<Expr> {
    let mut left = parse_primary(tokens, pos, base, end)?;

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
                bail!(Error::new(ErrorKind::ExpectedFieldName, at, 1));
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
    base: usize,
    end: usize,
) -> miette::Result<Expr> {
    struct DelimConfig {
        close_kind: TokenKind,
        unclosed_kind: ErrorKind,
        comma_kind: ErrorKind,
        open_at: usize,
        unclosed_span_len: usize,
    }

    impl DelimConfig {
        fn list(open_at: usize, span_len: usize) -> Self {
            Self {
                close_kind: TokenKind::RBracket,
                unclosed_kind: ErrorKind::UnclosedList,
                comma_kind: ErrorKind::ExpectedCommaInList,
                open_at,
                unclosed_span_len: span_len,
            }
        }
        fn group(open_at: usize, span_len: usize) -> Self {
            Self {
                close_kind: TokenKind::RParen,
                unclosed_kind: ErrorKind::UnclosedGroup,
                comma_kind: ErrorKind::ExpectedCommaBetweenArgs,
                open_at,
                unclosed_span_len: span_len,
            }
        }
    }

    fn parse_delimited_items(
        tokens: &[Token],
        pos: &mut usize,
        base: usize,
        end: usize,
        dc: &DelimConfig,
    ) -> miette::Result<Vec<Expr>> {
        let mut items = Vec::new();
        loop {
            if *pos >= tokens.len() {
                bail!(Error::new(
                    dc.unclosed_kind.clone(),
                    dc.open_at,
                    dc.unclosed_span_len,
                ));
            }
            if tokens[*pos].kind == dc.close_kind {
                *pos += 1;
                break;
            }
            let item = parse_postfix(tokens, pos, base, end)?;
            items.push(item);
            if *pos >= tokens.len() {
                bail!(Error::new(
                    dc.unclosed_kind.clone(),
                    dc.open_at,
                    dc.unclosed_span_len,
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
                    bail!(Error::new(
                        dc.comma_kind.clone(),
                        tokens[*pos].range.start,
                        1,
                    ));
                }
            }
        }
        Ok(items)
    }

    macro_rules! literal {
        ($kind:expr) => {{
            let range = tokens[*pos].range.clone();
            *pos += 1;
            Ok(Expr { kind: $kind, range })
        }};
    }

    if *pos >= tokens.len() {
        bail!(Error::new(ErrorKind::UnexpectedEndOfExpr, base, 1));
    }

    match &tokens[*pos].kind {
        TokenKind::Str(s) => literal!(ExprKind::Str(s.clone())),
        TokenKind::Int(n) => literal!(ExprKind::Int(*n)),
        TokenKind::True => literal!(ExprKind::Bool(true)),
        TokenKind::False => literal!(ExprKind::Bool(false)),
        TokenKind::LBracket => {
            let start = tokens[*pos].range.start;
            *pos += 1;
            let stop = tokens.last().map_or(start + 1, |t| t.range.end);
            let items = parse_delimited_items(
                tokens,
                pos,
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
            bail!(Error::new(ErrorKind::UnexpectedToken, at, 1))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::scanner::scan;

    use super::*;
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

    #[test_case("{{ }}" => (ErrorKind::UnexpectedEndOfExpr, 2, 1); "unexpected_end_of_expr")]
    #[test_case("{{ @ }}" => (ErrorKind::UnexpectedToken, 3, 1); "unexpected_token_at_sign")]
    #[test_case("{{ p% }}" => (ErrorKind::UnexpectedToken, 4, 1); "unexpected_token_percent")]
    #[test_case("{{ a b }}" => (ErrorKind::UnexpectedTokensAfterExpr, 5, 1); "unexpected_tokens_after_expr")]
    #[test_case("{{ () }}" => (ErrorKind::UnexpectedToken, 3, 1); "unexpected_token_empty_paren")]
    #[test_case("{{ a..b }}" => (ErrorKind::ExpectedFieldName, 5, 1); "expected_field_name_after_dot")]
    #[test_case("{{ list., }}" => (ErrorKind::ExpectedFieldName, 8, 1); "expected_field_name_trailing_dot")]
    #[test_case("{{ \"hello }}" => (ErrorKind::UnclosedString, 3, 6); "unclosed_string")]
    #[test_case("{{ [1, 2 }}" => (ErrorKind::UnclosedList, 3, 5); "unclosed_list")]
    #[test_case("{{ fn(a, b }}" => (ErrorKind::UnclosedGroup, 5, 7); "unclosed_group")]
    #[test_case("{{ [1  2] }}" => (ErrorKind::ExpectedCommaInList, 7, 1); "expected_comma_in_list")]
    #[test_case("{{ fn(a b) }}" => (ErrorKind::ExpectedCommaBetweenArgs, 8, 1); "expected_comma_between_args")]
    #[test_case("{%%}" => (ErrorKind::EmptyStatement, 0, 4); "empty_statement")]
    #[test_case("{% foobar %}" => (ErrorKind::UnexpectedKeyword, 3, 6); "unexpected_keyword")]
    #[test_case("{% if %}" => (ErrorKind::UnexpectedEndOfExpr, 5, 1); "if_no_condition")]
    #[test_case("{% if true %}" => (ErrorKind::UnclosedBlock, 0, 13); "unclosed_if_block")]
    #[test_case("{% elif true %}" => (ErrorKind::StrayElif, 0, 15); "stray_elif")]
    #[test_case("{% else true %}" => (ErrorKind::StrayElse, 0, 15); "stray_else")]
    #[test_case("{% end %}" => (ErrorKind::StrayEnd, 0, 9); "stray_end")]
    #[test_case("{% for %}" => (ErrorKind::ExpectedForSyntax, 6, 1); "for_no_var")]
    #[test_case("{% for 123 in items %}" => (ErrorKind::ExpectedForVar, 7, 3); "for_non_ident_var")]
    #[test_case("{% for x of items %}" => (ErrorKind::ExpectedForIn, 9, 2); "for_missing_in")]
    #[test_case("{% for x in items %}" => (ErrorKind::UnclosedFor, 0, 20); "unclosed_for")]
    fn test_error(input: &str) -> (ErrorKind, usize, usize) {
        let input = input.as_bytes();
        let report = parse(&scan(input).unwrap(), input).unwrap_err();
        let error = report.downcast_ref::<Error>().unwrap().clone();
        error.destruct()
    }
}

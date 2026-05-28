use std::ops::Range;

use crate::ast::{Expr, Node};
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

pub fn parse(tokens: &[RawToken], source: &[u8]) -> Result<Vec<Node>, Error> {
    let mut nodes = Vec::new();
    let mut i = 0;

    while i < tokens.len() {
        match &tokens[i] {
            RawToken::Text(range) => {
                nodes.push(Node::Text(range.clone()));
                i += 1;
            }
            RawToken::Interpolate(range) => {
                let toks = tokenize(source, range.clone())?;
                let mut pos = 0;
                let expr = parse_postfix(&toks, &mut pos, source, range.start, range.end)?;
                if pos != toks.len() {
                    return Err(Error::new(
                        ErrorKind::UnexpectedTokensAfterExpr,
                        toks[pos].range.start,
                        1,
                        source,
                    ));
                }
                nodes.push(Node::Interpolate(expr));
                i += 1;
            }
            RawToken::Statement(range) => {
                let opening = range.clone();
                let toks = tokenize(source, range.clone())?;
                let kw = toks.first().ok_or_else(|| {
                    Error::new(ErrorKind::EmptyStatement, opening.start - 2, opening.len() + 4, source)
                })?;
                let (stmt_nodes, consumed) = match &kw.kind {
                    TokenKind::If => parse_if_block(tokens, source, i)?,
                    TokenKind::For => parse_for_block(tokens, source, i)?,
                    TokenKind::Elif => {
                        return Err(Error::new(
                            ErrorKind::StrayElif,
                            opening.start - 2,
                            opening.len() + 4,
                            source,
                        ));
                    }
                    TokenKind::Else => {
                        return Err(Error::new(
                            ErrorKind::StrayElse,
                            opening.start - 2,
                            opening.len() + 4,
                            source,
                        ));
                    }
                    TokenKind::End => {
                        return Err(Error::new(
                            ErrorKind::StrayEnd,
                            opening.start - 2,
                            opening.len() + 4,
                            source,
                        ));
                    }
                    _ => {
                        return Err(Error::new(
                            ErrorKind::UnexpectedKeyword,
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

fn parse_if_block(
    tokens: &[RawToken],
    source: &[u8],
    start: usize,
) -> Result<(Vec<Node>, usize), Error> {
    let mut branches: Vec<(Expr, Vec<Node>)> = Vec::new();
    let mut else_branch: Option<Vec<Node>> = None;
    let mut i = start;
    let mut consumed = 0;

    let opening = match tokens.get(start) {
        Some(RawToken::Statement(r)) => r.clone(),
        _ => return Err(Error::new(ErrorKind::UnclosedBlock, 0, 1, source)),
    };

    loop {
        let raw = match tokens.get(i) {
            Some(RawToken::Statement(range)) => range.clone(),
            None => {
                return Err(Error::new(
                    ErrorKind::UnclosedBlock,
                    opening.start - 2,
                    opening.len() + 4,
                    source,
                ));
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
            }) => {
                let mut pos = 1;
                let cond = parse_postfix(&toks, &mut pos, source, toks[0].range.end, raw.end)?;
                if pos != toks.len() {
                    return Err(Error::new(
                        ErrorKind::UnexpectedTokensAfterExpr,
                        toks[pos].range.start,
                        1,
                        source,
                    ));
                }
                let (body, body_consumed) =
                    collect_body_until(tokens, source, i + 1, &[TokenKind::Elif, TokenKind::Else])?;
                branches.push((cond, body));
                i += body_consumed + 1;
                consumed += body_consumed + 1;
            }
            Some(Token {
                kind: TokenKind::Elif,
                ..
            }) => {
                let mut pos = 1;
                let cond = parse_postfix(&toks, &mut pos, source, toks[0].range.end, raw.end)?;
                if pos != toks.len() {
                    return Err(Error::new(
                        ErrorKind::UnexpectedTokensAfterExpr,
                        toks[pos].range.start,
                        1,
                        source,
                    ));
                }
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
                return Err(Error::new(
                    ErrorKind::UnexpectedKeyword,
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
) -> Result<(Vec<Node>, usize), Error> {
    let range = match tokens.get(start) {
        Some(RawToken::Statement(r)) => r.clone(),
        _ => unreachable!(),
    };

    let toks = tokenize(source, range.clone())?;

    if toks.len() < 4 {
        return Err(Error::new(
            ErrorKind::ExpectedForSyntax,
            toks[0].range.end,
            1,
            source,
        ));
    }

    let var = match &toks[1].kind {
        TokenKind::Ident(name) => name.clone(),
        _ => {
            return Err(Error::new(
                ErrorKind::ExpectedForVar,
                toks[1].range.start,
                toks[1].range.len(),
                source,
            ));
        }
    };

    if toks[2].kind != TokenKind::In {
        return Err(Error::new(
            ErrorKind::ExpectedForIn,
            toks[2].range.start,
            toks[2].range.len(),
            source,
        ));
    }

    let mut pos = 3;
    let collection = parse_postfix(&toks, &mut pos, source, toks[0].range.end, range.end)?;
    if pos != toks.len() {
        return Err(Error::new(
            ErrorKind::UnexpectedTokensAfterExpr,
            toks[pos].range.start,
            1,
            source,
        ));
    }

    let (body, body_consumed) = collect_body_until(tokens, source, start + 1, &[])?;

    let end_pos = start + body_consumed + 1;
    match tokens.get(end_pos) {
        Some(RawToken::Statement(r))
            if tokenize(source, r.clone())?.first().map(|t| &t.kind) == Some(&TokenKind::End) => {}
        _ => {
            return Err(Error::new(
                ErrorKind::UnclosedFor,
                range.start - 2,
                range.len() + 4,
                source,
            ));
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
) -> Result<(Vec<Node>, usize), Error> {
    let mut body = Vec::new();
    let mut i = start;

    while i < tokens.len() {
        match &tokens[i] {
            RawToken::Statement(range) => {
                let toks = tokenize(source, range.clone())?;
                match toks.first() {
                    Some(Token {
                        kind: TokenKind::If,
                        ..
                    }) => {
                        let (nodes, consumed) = parse_if_block(tokens, source, i)?;
                        body.extend(nodes);
                        i += consumed - 1;
                    }
                    Some(Token {
                        kind: TokenKind::For,
                        ..
                    }) => {
                        let (nodes, consumed) = parse_for_block(tokens, source, i)?;
                        body.extend(nodes);
                        i += consumed - 1;
                    }
                    Some(Token {
                        kind: TokenKind::End,
                        ..
                    }) => {
                        return Ok((body, i - start));
                    }
                    Some(tok) if stop_at.contains(&tok.kind) => {
                        return Ok((body, i - start));
                    }
                    _ => {
                        body.push(Node::Text(range.clone()));
                    }
                }
            }
            RawToken::Interpolate(range) => {
                let base = range.start;
                let toks = tokenize(source, range.clone())?;
                let mut pos = 0;
                let expr = parse_postfix(&toks, &mut pos, source, base, range.end)?;
                if pos != toks.len() {
                    return Err(Error::new(
                        ErrorKind::UnexpectedTokensAfterExpr,
                        toks[pos].range.start,
                        1,
                        source,
                    ));
                }
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

fn tokenize(source: &[u8], range: Range<usize>) -> Result<Vec<Token>, Error> {
    let bytes = &source[range.clone()];
    let mut tokens = Vec::new();
    let mut pos = 0;

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
                        return Err(Error::new(
                            ErrorKind::UnclosedString,
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
                tokens.push(Token {
                    kind: TokenKind::LParen,
                    range: range.start + pos..range.start + pos + 1,
                });
                pos += 1;
            }
            b')' => {
                tokens.push(Token {
                    kind: TokenKind::RParen,
                    range: range.start + pos..range.start + pos + 1,
                });
                pos += 1;
            }
            b',' => {
                tokens.push(Token {
                    kind: TokenKind::Comma,
                    range: range.start + pos..range.start + pos + 1,
                });
                pos += 1;
            }
            b'.' => {
                tokens.push(Token {
                    kind: TokenKind::Dot,
                    range: range.start + pos..range.start + pos + 1,
                });
                pos += 1;
            }
            b'[' => {
                tokens.push(Token {
                    kind: TokenKind::LBracket,
                    range: range.start + pos..range.start + pos + 1,
                });
                pos += 1;
            }
            b']' => {
                tokens.push(Token {
                    kind: TokenKind::RBracket,
                    range: range.start + pos..range.start + pos + 1,
                });
                pos += 1;
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
            _ if bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_' => {
                let ident_start = range.start + pos;
                let start = pos;
                while pos < bytes.len()
                    && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_')
                {
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
                return Err(Error::new(
                    ErrorKind::UnexpectedToken,
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
) -> Result<Expr, Error> {
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
                return Err(Error::new(ErrorKind::ExpectedFieldName, at, 1, source));
            }
        };
        *pos += 1;
        left = Expr::Dot {
            left: Box::new(left),
            field,
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
) -> Result<Expr, Error> {
    if *pos >= tokens.len() {
        return Err(Error::new(ErrorKind::UnexpectedEndOfExpr, base, 1, source));
    }

    match &tokens[*pos].kind {
        TokenKind::Str(s) => {
            *pos += 1;
            Ok(Expr::Str(s.clone()))
        }
        TokenKind::Int(n) => {
            *pos += 1;
            Ok(Expr::Int(*n))
        }
        TokenKind::True => {
            *pos += 1;
            Ok(Expr::Bool(true))
        }
        TokenKind::False => {
            *pos += 1;
            Ok(Expr::Bool(false))
        }
        TokenKind::LBracket => {
            let lbracket_range = tokens[*pos].range.start;
            *pos += 1;
            let mut items = Vec::new();
            loop {
                if *pos >= tokens.len() {
                    let stop = tokens.last().map_or(lbracket_range + 1, |t| t.range.end);
                    return Err(Error::new(
                        ErrorKind::UnclosedList,
                        lbracket_range,
                        stop - lbracket_range,
                        source,
                    ));
                }
                if tokens[*pos].kind == TokenKind::RBracket {
                    *pos += 1;
                    break;
                }
                let item = parse_postfix(tokens, pos, source, base, end)?;
                items.push(item);
                if *pos >= tokens.len() {
                    let stop = tokens.last().map_or(lbracket_range + 1, |t| t.range.end);
                    return Err(Error::new(
                        ErrorKind::UnclosedList,
                        lbracket_range,
                        stop - lbracket_range,
                        source,
                    ));
                }
                match tokens[*pos].kind {
                    TokenKind::Comma => {
                        *pos += 1;
                    }
                    TokenKind::RBracket => {
                        *pos += 1;
                        break;
                    }
                    _ => {
                        return Err(Error::new(
                            ErrorKind::ExpectedCommaInList,
                            tokens[*pos].range.start,
                            1,
                            source,
                        ));
                    }
                }
            }
            Ok(Expr::List(items))
        }
        TokenKind::Ident(name) => {
            let name = name.clone();
            *pos += 1;
            if *pos < tokens.len() && tokens[*pos].kind == TokenKind::LParen {
                let lparen_range = tokens[*pos].range.start;
                *pos += 1;
                let mut args = Vec::new();
                loop {
                    if *pos >= tokens.len() {
                        return Err(Error::new(
                            ErrorKind::UnclosedGroup,
                            lparen_range,
                            end.saturating_sub(lparen_range) + 1,
                            source,
                        ));
                    }
                    if tokens[*pos].kind == TokenKind::RParen {
                        *pos += 1;
                        break;
                    }
                    let arg = parse_postfix(tokens, pos, source, base, end)?;
                    args.push(arg);
                    if *pos >= tokens.len() {
                        return Err(Error::new(
                            ErrorKind::UnclosedGroup,
                            lparen_range,
                            end.saturating_sub(lparen_range) + 1,
                            source,
                        ));
                    }
                    match tokens[*pos].kind {
                        TokenKind::Comma => {
                            *pos += 1;
                        }
                        TokenKind::RParen => {
                            *pos += 1;
                            break;
                        }
                        _ => {
                            return Err(Error::new(
                                ErrorKind::ExpectedCommaBetweenArgs,
                                tokens[*pos].range.start,
                                1,
                                source,
                            ));
                        }
                    }
                }
                Ok(Expr::FnCall { name, args })
            } else {
                Ok(Expr::Var(name))
            }
        }
        _ => {
            let at = tokens[*pos].range.start;
            Err(Error::new(ErrorKind::UnexpectedToken, at, 1, source))
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
        (var $name:expr) => { Expr::Var($name.into()) };
        (str $s:expr) => { Expr::Str($s.into()) };
        (int $n:expr) => { Expr::Int($n) };
        (bool $b:expr) => { Expr::Bool($b) };
        (list [$($e:expr),* $(,)?]) => { Expr::List(vec![$(($e)),*]) };
        (call $name:expr, [$($arg:expr),* $(,)?]) => {
            Expr::FnCall { name: $name.into(), args: vec![$($arg),*] }
        };
        (dot $e:expr, $field:expr) => {
            Expr::Dot { left: Box::new($e), field: $field.into() }
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

    #[test_case("" => Vec::<Node>::new())]
    #[test_case("hello" => vec![node!(text 0..5)])]
    #[test_case("{{ a }}" => vec![node!(interp var "a")])]
    #[test_case("{{ \"str\" }}" => vec![node!(interp str "str")])]
    #[test_case("{{ 42 }}" => vec![node!(interp int 42)])]
    #[test_case("{{ -1 }}" => vec![node!(interp int -1)])]
    #[test_case("{{ true }}" => vec![node!(interp bool true)])]
    #[test_case("{{ false }}" => vec![node!(interp bool false)])]
    #[test_case("{{ [] }}" => vec![node!(interp list [])])]
    #[test_case("{{ [1,  \"2\",[  3]  ] }}" => vec![node!(interp list [expr!(int 1), expr!(str "2"), expr!(list [expr!(int 3)])])])]
    #[test_case("{{ fn() }}" => vec![node!(interp call "fn", [])])]
    #[test_case("{{ fn1(a, 2  ,fn2(  \"\"   )) }}" => vec![node!(interp call "fn1", [expr!(var "a"), expr!(int 2), expr!(call "fn2", [expr!(str "")])])])]
    #[test_case("{{ a.b }}" => vec![node!(interp dot expr!(var "a"), "b")])]
    #[test_case("{{ a.b.c }}" => vec![node!(interp dot expr!(dot expr!(var "a"), "b"), "c")])]
    #[test_case("{{ list.0 }}" => vec![node!(interp dot expr!(var "list"), "0")])]
    #[test_case(r#"{{ "hello \"world\" \\ " }}"# => vec![node!(interp str r#"hello "world" \ "#)])]
    #[test_case("a {{ x }} b" => vec![
        node!(text 0..2),
        node!(interp var "x"),
        node!(text 9..11),
    ])]
    #[test_case("{% if true %}{% end %}" => vec![node!(if [[expr!(bool true) => []]])])]
    #[test_case("{% if a %}{% else %}{% end %}" => vec![node!(
        if [[expr!(var "a") => []]]
        else [],
    )])]
    #[test_case("{% if a %}{% elif b %}{% else %}{% end %}" => vec![node!(
        if [
            [expr!(var "a") => []],
            [expr!(var "b") => []],
        ]
        else [],
    )])]
    #[test_case("{% for x in items %}{% end %}" => vec![node!(for "x" in expr!(var "items") => [])])]
    #[test_case("{% for x in [1, 2] %}{% end %}" => vec![node!(for "x" in expr!(list [expr!(int 1), expr!(int 2)]) => [])])]
    #[test_case("{% if a %}A{% end %}" => vec![node!(if [[expr!(var "a") => [node!(text 10..11)]]])])]
    #[test_case("{% for x in items %}X{% end %}" => vec![node!(for "x" in expr!(var "items") => [node!(text 20..21)])])]
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
        )]
    )]
    fn test_parse(input: &str) -> Vec<Node> {
        let input = input.as_bytes();
        parse(&scan(input).unwrap(), input).unwrap()
    }

    #[test_case("{{ }}" => (ErrorKind::UnexpectedEndOfExpr, 2, 1))]
    #[test_case("{{ @ }}" => (ErrorKind::UnexpectedToken, 3, 1))]
    #[test_case("{{ p% }}" => (ErrorKind::UnexpectedToken, 4, 1))]
    #[test_case("{{ a b }}" => (ErrorKind::UnexpectedTokensAfterExpr, 5, 1))]
    #[test_case("{{ () }}" => (ErrorKind::UnexpectedToken, 3, 1); "todo1")]
    #[test_case("{{ a..b }}" => (ErrorKind::ExpectedFieldName, 5, 1))]
    #[test_case("{{ list., }}" => (ErrorKind::ExpectedFieldName, 8, 1))]
    #[test_case("{{ \"hello }}" => (ErrorKind::UnclosedString, 3, 6))]
    #[test_case("{{ [1, 2 }}" => (ErrorKind::UnclosedList, 3, 5))]
    #[test_case("{{ fn(a, b }}" => (ErrorKind::UnclosedGroup, 5, 7))]
    #[test_case("{{ [1  2] }}" => (ErrorKind::ExpectedCommaInList, 7, 1))]
    #[test_case("{{ fn(a b) }}" => (ErrorKind::ExpectedCommaBetweenArgs, 8, 1))]
    #[test_case("{%%}" => (ErrorKind::EmptyStatement, 0, 4))]
    #[test_case("{% foobar %}" => (ErrorKind::UnexpectedKeyword, 3, 6))]
    #[test_case("{% if %}" => (ErrorKind::UnexpectedEndOfExpr, 5, 1))]
    #[test_case("{% if true %}" => (ErrorKind::UnclosedBlock, 0, 13))]
    #[test_case("{% elif true %}" => (ErrorKind::StrayElif, 0, 15))]
    #[test_case("{% else true %}" => (ErrorKind::StrayElse, 0, 15))]
    #[test_case("{% end %}" => (ErrorKind::StrayEnd, 0, 9))]
    #[test_case("{% for %}" => (ErrorKind::ExpectedForSyntax, 6, 1))]
    #[test_case("{% for 123 in items %}" => (ErrorKind::ExpectedForVar, 7, 3))]
    #[test_case("{% for x of items %}" => (ErrorKind::ExpectedForIn, 9, 2))]
    #[test_case("{% for x in items %}" => (ErrorKind::UnclosedFor, 0, 20))]
    fn test_error(input: &str) -> (ErrorKind, usize, usize) {
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

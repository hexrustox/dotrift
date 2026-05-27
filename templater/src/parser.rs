use std::ops::Range;

use crate::ast::{Expr, Node};
use crate::error::Error;
use crate::scanner::RawToken;

#[derive(Debug, Clone, PartialEq)]
enum Token {
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
                let toks = tokenize(source, range.clone());
                let mut pos = 0;
                let expr = parse_postfix(&toks, &mut pos, source, range.start)?;
                if pos != toks.len() {
                    return Err(Error::new("unexpected tokens after expression", 0, 0));
                }
                nodes.push(Node::Interpolate(expr));
                i += 1;
            }
            RawToken::Statement(range) => {
                let toks = tokenize(source, range.clone());
                let kw = toks
                    .first()
                    .ok_or_else(|| Error::new("empty statement", 0, 0))?;
                let (stmt_nodes, consumed) = match kw {
                    Token::If => parse_if_block(tokens, source, i)?,
                    Token::For => parse_for_block(tokens, source, i)?,
                    Token::End => {
                        return Err(Error::new("{% end %} without matching opening block", 0, 0));
                    }
                    _ => {
                        return Err(Error::new(
                            format!("unexpected keyword in statement: {:?}", kw),
                            0,
                            0,
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

    loop {
        let raw = match tokens.get(i) {
            Some(RawToken::Statement(range)) => range.clone(),
            Some(RawToken::Text(_) | RawToken::Interpolate(_)) => {
                i += 1;
                consumed += 1;
                continue;
            }
            None => {
                return Err(Error::new("unclosed {% if %} block", 0, 0));
            }
        };

        let toks = tokenize(source, raw.clone());
        let first = toks.first().cloned();

        match first {
            Some(Token::If) => {
                let mut pos = 1;
                let cond = parse_postfix(&toks, &mut pos, source, raw.start)?;
                if pos != toks.len() {
                    return Err(Error::new("unexpected tokens after expression", 0, 0));
                }
                let (body, body_consumed) =
                    collect_body_until(tokens, source, i + 1, &[Token::Elif, Token::Else])?;
                branches.push((cond, body));
                i += body_consumed + 1;
                consumed += body_consumed + 1;
            }
            Some(Token::Elif) => {
                let mut pos = 1;
                let cond = parse_postfix(&toks, &mut pos, source, raw.start)?;
                if pos != toks.len() {
                    return Err(Error::new("unexpected tokens after expression", 0, 0));
                }
                let (body, body_consumed) =
                    collect_body_until(tokens, source, i + 1, &[Token::Elif, Token::Else])?;
                branches.push((cond, body));
                i += body_consumed + 1;
                consumed += body_consumed + 1;
            }
            Some(Token::Else) => {
                let (body, body_consumed) = collect_body_until(tokens, source, i + 1, &[])?;
                else_branch = Some(body);
                consumed += body_consumed + 2;
                break;
            }
            Some(Token::End) => {
                consumed += 1;
                break;
            }
            _ => {
                return Err(Error::new(
                    "expected {% if %}, {% elif %}, {% else %}, or {% end %}",
                    raw.start,
                    raw.len(),
                ));
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
        _ => return Err(Error::new("expected statement", 0, 0)),
    };

    let toks = tokenize(source, range.clone());

    if toks.len() < 4 {
        return Err(Error::new(
            "expected {% for var in expr %}",
            range.start,
            range.len(),
        ));
    }

    let var = match &toks[1] {
        Token::Ident(name) => name.clone(),
        _ => {
            return Err(Error::new(
                "expected variable name after for",
                range.start,
                range.len(),
            ));
        }
    };

    if toks[2] != Token::In {
        return Err(Error::new(
            "expected 'in' after for variable",
            range.start,
            range.len(),
        ));
    }

    let mut pos = 3;
    let collection = parse_postfix(&toks, &mut pos, source, range.start)?;
    if pos != toks.len() {
        return Err(Error::new("unexpected tokens after expression", 0, 0));
    }

    let (body, body_consumed) = collect_body_until(tokens, source, start + 1, &[])?;

    let end_pos = start + body_consumed + 1;
    match tokens.get(end_pos) {
        Some(RawToken::Statement(r)) => {
            let end_toks = tokenize(source, r.clone());
            if end_toks.first() != Some(&Token::End) {
                return Err(Error::new("expected {% end %}", r.start, r.len()));
            }
        }
        _ => {
            return Err(Error::new(
                "unclosed {% for %} block",
                range.start,
                range.len(),
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
    stop_at: &[Token],
) -> Result<(Vec<Node>, usize), Error> {
    let mut body = Vec::new();
    let mut i = start;

    while i < tokens.len() {
        match &tokens[i] {
            RawToken::Statement(range) => {
                let toks = tokenize(source, range.clone());
                match toks.first() {
                    Some(Token::If) => {
                        let (nodes, consumed) = parse_if_block(tokens, source, i)?;
                        body.extend(nodes);
                        i += consumed - 1;
                    }
                    Some(Token::For) => {
                        let (nodes, consumed) = parse_for_block(tokens, source, i)?;
                        body.extend(nodes);
                        i += consumed - 1;
                    }
                    Some(Token::End) => {
                        return Ok((body, i - start));
                    }
                    Some(t) if stop_at.contains(t) => {
                        return Ok((body, i - start));
                    }
                    _ => {
                        body.push(Node::Text(range.clone()));
                    }
                }
            }
            RawToken::Interpolate(range) => {
                let base = range.start;
                let toks = tokenize(source, range.clone());
                let mut pos = 0;
                let expr = parse_postfix(&toks, &mut pos, source, base)?;
                if pos != toks.len() {
                    return Err(Error::new("unexpected tokens after expression", 0, 0));
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

fn tokenize(source: &[u8], range: Range<usize>) -> Vec<Token> {
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
                pos += 1;
                let mut s = String::new();
                while pos < bytes.len() {
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
                tokens.push(Token::Str(s));
            }
            b'(' => {
                tokens.push(Token::LParen);
                pos += 1;
            }
            b')' => {
                tokens.push(Token::RParen);
                pos += 1;
            }
            b',' => {
                tokens.push(Token::Comma);
                pos += 1;
            }
            b'.' => {
                tokens.push(Token::Dot);
                pos += 1;
            }
            b'[' => {
                tokens.push(Token::LBracket);
                pos += 1;
            }
            b']' => {
                tokens.push(Token::RBracket);
                pos += 1;
            }
            b'-' | b'0'..=b'9' => {
                let negative = bytes[pos] == b'-';
                if negative {
                    pos += 1;
                }
                let mut n: i64 = 0;
                while pos < bytes.len() && bytes[pos].is_ascii_digit() {
                    n = n * 10 + (bytes[pos] - b'0') as i64;
                    pos += 1;
                }
                tokens.push(Token::Int(if negative { -n } else { n }));
            }
            _ => {
                let start = pos;
                while pos < bytes.len()
                    && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_')
                {
                    pos += 1;
                }
                let ident = &bytes[start..pos];
                tokens.push(match ident {
                    b"true" => Token::True,
                    b"false" => Token::False,
                    b"if" => Token::If,
                    b"elif" => Token::Elif,
                    b"else" => Token::Else,
                    b"for" => Token::For,
                    b"end" => Token::End,
                    b"in" => Token::In,
                    _ => Token::Ident(String::from_utf8_lossy(ident).into_owned()),
                });
            }
        }
    }

    tokens
}

fn parse_postfix(
    tokens: &[Token],
    pos: &mut usize,
    source: &[u8],
    base: usize,
) -> Result<Expr, Error> {
    let mut left = parse_primary(tokens, pos, source, base)?;

    while *pos < tokens.len() && matches!(tokens[*pos], Token::Dot) {
        *pos += 1;
        let field = match tokens.get(*pos) {
            Some(Token::Ident(name)) => name.clone(),
            Some(Token::Int(n)) => n.to_string(),
            Some(_) => {
                return Err(Error::new("expected field name after '.'", base, 1));
            }
            None => {
                return Err(Error::new("expected field name after '.'", base, 1));
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
) -> Result<Expr, Error> {
    if *pos >= tokens.len() {
        return Err(Error::new("unexpected end of expression", base, 1));
    }

    match &tokens[*pos] {
        Token::Str(s) => {
            *pos += 1;
            Ok(Expr::Str(s.clone()))
        }
        Token::Int(n) => {
            *pos += 1;
            Ok(Expr::Int(*n))
        }
        Token::True => {
            *pos += 1;
            Ok(Expr::Bool(true))
        }
        Token::False => {
            *pos += 1;
            Ok(Expr::Bool(false))
        }
        Token::LBracket => {
            *pos += 1;
            let mut items = Vec::new();
            while *pos < tokens.len() && tokens[*pos] != Token::RBracket {
                if !items.is_empty() {
                    if tokens[*pos] != Token::Comma {
                        return Err(Error::new("expected ',' in list", base, 1));
                    }
                    *pos += 1;
                }
                let item = parse_postfix(tokens, pos, source, base)?;
                items.push(item);
                if *pos < tokens.len() && tokens[*pos] == Token::Comma {
                    continue;
                }
                break;
            }
            if *pos < tokens.len() && tokens[*pos] == Token::RBracket {
                *pos += 1;
            }
            Ok(Expr::List(items))
        }
        Token::LParen => {
            *pos += 1;
            let expr = parse_postfix(tokens, pos, source, base)?;
            if *pos < tokens.len() && tokens[*pos] == Token::RParen {
                *pos += 1;
            }
            Ok(expr)
        }
        Token::Ident(name) => {
            let name = name.clone();
            *pos += 1;
            if *pos < tokens.len() && tokens[*pos] == Token::LParen {
                *pos += 1;
                let mut args = Vec::new();
                while *pos < tokens.len() && tokens[*pos] != Token::RParen {
                    if !args.is_empty() {
                        if tokens[*pos] != Token::Comma {
                            return Err(Error::new("expected ',' between arguments", base, 1));
                        }
                        *pos += 1;
                    }
                    let arg = parse_postfix(tokens, pos, source, base)?;
                    args.push(arg);
                    if *pos < tokens.len() && tokens[*pos] == Token::Comma {
                        continue;
                    }
                    break;
                }
                if *pos < tokens.len() && tokens[*pos] == Token::RParen {
                    *pos += 1;
                }
                Ok(Expr::FnCall { name, args })
            } else {
                Ok(Expr::Var(name))
            }
        }
        _ => Err(Error::new(
            format!("unexpected token: {:?}", tokens[*pos]),
            base,
            1,
        )),
    }
}

#[cfg(test)]
mod tests {
    use crate::scanner::scan;

    use super::*;
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
    #[test_case("{{ [1, \"2\", [3]] }}" => vec![node!(interp list [expr!(int 1), expr!(str "2"), expr!(list [expr!(int 3)])])])]
    #[test_case("{{ fn() }}" => vec![node!(interp call "fn", [])])]
    #[test_case("{{ fn(a) }}" => vec![node!(interp call "fn", [expr!(var "a")])])]
    #[test_case("{{ fn1(a, 2, fn2(\"\")) }}" => vec![node!(interp call "fn1", [expr!(var "a"), expr!(int 2), expr!(call "fn2", [expr!(str "")])])])]
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
}

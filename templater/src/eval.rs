use std::{borrow::Cow, collections::HashMap, io};

use miette::SourceSpan;

use crate::{
    Template, Value,
    ast::{Expr, Node},
    error::{Error, RenderError, Result},
};

/// A binding layer in the scope stack walked by variable resolution. This
/// slice has only the base Var frame; `for` will add a loop frame later
/// (ticket 06) and the borrowed/owned distinction will pay off then.
pub(crate) enum Frame<'a> {
    /// The host-provided top-level variable scope (`Template::render`'s
    /// `variables` argument). Borrowed for the lifetime of the render.
    Var(&'a HashMap<String, Value>),
}

impl Template {
    /// Renders a sequence of nodes to the writer.
    pub(crate) fn eval_body<W: io::Write>(
        &self,
        nodes: &[Node],
        writer: &mut W,
        frame: &Frame<'_>,
    ) -> Result<()> {
        for node in nodes {
            match node {
                Node::Text(range) => writer.write_all(&self.src.bytes()[range.clone()])?,
                Node::Interpolate(expr) => {
                    // String literals escape-walk directly into the writer
                    // (zero allocation, byte-preserved). All other exprs are
                    // evaluated to an owned `Value` and written via
                    // `write_top`. When function calls (ticket 04) can pass
                    // a string literal as an argument, `eval` will need to
                    // grow a `Value::Str` path for StrLit; this slice's
                    // shapes don't exercise that.
                    match expr {
                        Expr::StrLit(range) => {
                            write_string_literal(self.src.bytes(), range.clone(), writer)?;
                        }
                        _ => {
                            let value = eval(expr, self.src.bytes(), frame)?;
                            value.write_top(writer)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// Looks up `name` against the active scope stack. Returns a borrowed
/// `Cow::Borrowed` for the Var frame (the common case — no clone until
/// the caller asks for `into_owned`).
fn lookup<'v>(name: &str, frame: &'v Frame<'v>) -> Option<Cow<'v, Value>> {
    match frame {
        Frame::Var(map) => map.get(name).map(Cow::Borrowed),
    }
}

/// Evaluates one non-string-literal expression to an owned `Value`. Per
/// decision E1 the lookup path clones the underlying value out of the
/// borrowed scope (`StrLit` is handled separately in `eval_body`).
fn eval(expr: &Expr, src: &[u8], frame: &Frame<'_>) -> Result<Value> {
    Ok(match expr {
        Expr::IntLit(n) => Value::Int(*n),
        Expr::BoolLit(b) => Value::Bool(*b),
        Expr::StrLit(_) => unreachable!(
            "StrLit is handled by `eval_body`'s direct-to-writer path; \
             `eval` only fires for Var/IntLit/BoolLit in this slice"
        ),
        Expr::Var(range) => {
            let name_bytes = &src[range.clone()];
            // Variable names are restricted to `[A-Za-z_][A-Za-z0-9_]*` by
            // the parser, so the byte slice is ASCII (valid UTF-8).
            let name = std::str::from_utf8(name_bytes).expect("identifier is ascii");
            match lookup(name, frame) {
                Some(v) => v.into_owned(),
                None => {
                    return Err(Error::render(
                        RenderError::UndefinedVariable,
                        SourceSpan::from((range.start, range.end - range.start)),
                    ));
                }
            }
        }
    })
}

/// Walks the interior of a `"..."` literal (the byte range between the
/// opening and closing quotes, exclusive) and writes the decoded result
/// directly into `writer`:
///
/// - `\"` → `"`, `\\` → `\`.
/// - Any other `\X` → both bytes verbatim (no interpretation).
/// - Raw newlines and other bytes pass through unchanged, byte-for-byte
///   (no `char`-cast — non-ASCII bytes such as those inside `{{ "café" }}`
///   survive intact even though they aren't valid standalone UTF-8).
///
/// The range is guaranteed to be the interior of a *closed* string literal
/// (the parser rejects unclosed strings at parse time), so the loop is
/// infallible.
fn write_string_literal<W: io::Write>(
    src: &[u8],
    interior: std::ops::Range<usize>,
    writer: &mut W,
) -> io::Result<()> {
    let bytes = &src[interior];
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            match next {
                b'"' => {
                    writer.write_all(b"\"")?;
                    i += 2;
                    continue;
                }
                b'\\' => {
                    writer.write_all(b"\\")?;
                    i += 2;
                    continue;
                }
                _ => {
                    // Pass both bytes through verbatim.
                    writer.write_all(&bytes[i..i + 2])?;
                    i += 2;
                    continue;
                }
            }
        }
        writer.write_all(&bytes[i..i + 1])?;
        i += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;

    use test_case::test_case;

    fn decode(interior: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        write_string_literal(interior, 0..interior.len(), &mut out).unwrap();
        out
    }

    #[test_case(b"" => b"".to_vec(); "empty_interior")]
    #[test_case(b"plain" => b"plain".to_vec(); "plain_text")]
    #[test_case(b"a\\\"b" => b"a\"b".to_vec(); "escaped_quote")]
    #[test_case(b"a\\\\b" => b"a\\b".to_vec(); "escaped_backslash")]
    #[test_case(b"a\\nb" => b"a\\nb".to_vec(); "passthrough_escape")]
    #[test_case(b"line1\nline2" => b"line1\nline2".to_vec(); "raw_newline")]
    #[test_case(b"caf\xc3\xa9" => b"caf\xc3\xa9".to_vec(); "non_ascii_bytes_preserved")]
    #[test_case(b"\xff\xfe" => b"\xff\xfe".to_vec(); "invalid_utf8_preserved")]
    fn decode_cases(input: &[u8]) -> Vec<u8> {
        decode(input)
    }

    #[test]
    fn lookup_borrows_from_var_frame() {
        let mut map = HashMap::new();
        map.insert("x".to_string(), Value::Int(1));
        let frame = Frame::Var(&map);
        match lookup("x", &frame) {
            Some(Cow::Borrowed(Value::Int(1))) => {}
            other => panic!("expected Cow::Borrowed(Int(1)), got {other:?}"),
        }
        assert!(lookup("missing", &frame).is_none());
    }
}

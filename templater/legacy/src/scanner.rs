use std::ops::Range;

use crate::error::{Error, ErrorKind};

#[derive(Debug, Clone, Copy, PartialEq)]
enum TagKind {
    Interp,
    Stmt,
    Comment,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
enum Modifier {
    #[default]
    None,
    TrimWhitespace,
    TrimAll,
}

impl Modifier {
    fn byte_len(self) -> usize {
        match self {
            Self::None => 0,
            Self::TrimWhitespace | Self::TrimAll => 1,
        }
    }
}

#[derive(Debug, PartialEq)]
struct Tag {
    start: usize,
    end: usize,
    kind: TagKind,
    interior_start: usize,
    interior_end: usize,
    modifier_left: Modifier,
    modifier_right: Modifier,
    backslash_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct EscapeSite {
    pos: usize,
    n: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RawToken {
    Text(Range<usize>),
    Interpolate(Range<usize>),
    Statement(Range<usize>),
}

pub fn scan(source: &[u8]) -> Result<Vec<RawToken>, Error> {
    let (tags, escapes) = find_tags(source)?;
    let tokens = build_tokens(source, &tags, &escapes);
    Ok(tokens)
}

fn count_backslashes(source: &[u8], pos: usize) -> usize {
    let mut n = 0;
    while n < pos && source[pos - 1 - n] == b'\\' {
        n += 1;
    }
    n
}

fn find_tags(source: &[u8]) -> Result<(Vec<Tag>, Vec<EscapeSite>), Error> {
    let mut tags = Vec::new();
    let mut escapes = Vec::new();
    let mut pos = 0;

    while pos < source.len() {
        let next = match source[pos..]
            .iter()
            .position(|&b| b == b'{' || b == b'}' || b == b'%' || b == b'#')
            .map(|p| pos + p)
        {
            Some(o) => o,
            None => break,
        };

        let b = source[next];
        let b2 = source.get(next + 1).copied();

        if (b == b'}' && b2 == Some(b'}'))
            || (b == b'%' && b2 == Some(b'}'))
            || (b == b'#' && b2 == Some(b'}'))
        {
            let n = count_backslashes(source, next);
            if n % 2 == 1 {
                escapes.push(EscapeSite { pos: next, n });
                pos = next + 2;
                continue;
            }
            return Err(Error::new(ErrorKind::StrayDelimiter, next, 2));
        }
        if b != b'{' {
            pos = next + 1;
            continue;
        }

        let kind = match b2 {
            Some(b'{') => TagKind::Interp,
            Some(b'%') => TagKind::Stmt,
            Some(b'#') => TagKind::Comment,
            _ => {
                pos = next + 1;
                continue;
            }
        };

        let n = count_backslashes(source, next);
        if n % 2 == 1 {
            escapes.push(EscapeSite { pos: next, n });
            pos = next + 2;
            continue;
        }

        let close_delim = match kind {
            TagKind::Interp => b"}}",
            TagKind::Stmt => b"%}",
            TagKind::Comment => b"#}",
        };

        let interior_start = next + 2;

        let close = find_close(source, interior_start, close_delim)
            .ok_or_else(|| Error::new(ErrorKind::UnclosedDelimiter, next, 2))?;

        let interior_end = close;

        let (modifier_left, modifier_right) = parse_modifiers(source, interior_start, interior_end);

        let interior_content_start = interior_start + modifier_left.byte_len();
        let interior_content_end = interior_end - modifier_right.byte_len();

        tags.push(Tag {
            start: next,
            end: close + close_delim.len(),
            kind,
            interior_start: interior_content_start,
            interior_end: interior_content_end,
            modifier_left,
            modifier_right,
            backslash_count: n,
        });

        pos = close + close_delim.len();
    }

    Ok((tags, escapes))
}

fn find_close(source: &[u8], start: usize, delim: &[u8]) -> Option<usize> {
    let mut i = start;
    while i + delim.len() <= source.len() {
        if source[i] == b'"' {
            let mut j = i + 1;
            let closed = loop {
                if j >= source.len() {
                    break false;
                }
                if source[j] == b'\\' && j + 1 < source.len() {
                    j += 2;
                    continue;
                }
                if source[j] == b'"' {
                    break true;
                }
                j += 1;
            };
            if closed {
                i = j + 1;
                continue;
            }
        }
        if &source[i..i + delim.len()] == delim {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn modifier_from_byte(b: u8) -> Modifier {
    match b {
        b'-' => Modifier::TrimWhitespace,
        b'=' => Modifier::TrimAll,
        _ => Modifier::None,
    }
}

fn parse_modifiers(
    source: &[u8],
    interior_start: usize,
    interior_end: usize,
) -> (Modifier, Modifier) {
    let left = if interior_start < interior_end {
        modifier_from_byte(source[interior_start])
    } else {
        Default::default()
    };
    let right = if interior_end > interior_start {
        modifier_from_byte(source[interior_end - 1])
    } else {
        Default::default()
    };
    (left, right)
}

fn push_text_with_escapes(
    start: usize,
    end: usize,
    escapes: &[EscapeSite],
    tokens: &mut Vec<RawToken>,
) {
    if start >= end {
        return;
    }

    let mut pos = start;

    for esc in escapes {
        if esc.pos < start {
            continue;
        }
        if esc.pos + 2 > end {
            break;
        }

        let before_end = esc.pos - esc.n;
        if before_end > pos {
            tokens.push(RawToken::Text(pos..before_end));
        }

        let kept_start = esc.pos - esc.n;
        let kept_end = esc.pos - esc.n.div_ceil(2);
        if kept_start < kept_end {
            tokens.push(RawToken::Text(kept_start..kept_end));
        }

        tokens.push(RawToken::Text(esc.pos..esc.pos + 2));

        pos = esc.pos + 2;
    }

    if pos < end {
        tokens.push(RawToken::Text(pos..end));
    }
}

fn build_tokens(source: &[u8], tags: &[Tag], escapes: &[EscapeSite]) -> Vec<RawToken> {
    let mut tokens = Vec::new();
    let mut prev_end = 0usize;

    for (i, tag) in tags.iter().enumerate() {
        let mut text_start = prev_end;
        let mut text_end = tag.start;

        if i > 0 {
            let prev_tag = &tags[i - 1];
            apply_right_modifier(source, &mut text_start, prev_tag, tag.start);
        }
        apply_left_modifier(source, &mut text_end, tag, prev_end);

        if text_end > tag.backslash_count / 2 {
            text_end -= tag.backslash_count / 2;
        } else {
            text_end = 0;
        }

        push_text_with_escapes(text_start, text_end, escapes, &mut tokens);

        match tag.kind {
            TagKind::Interp => {
                tokens.push(RawToken::Interpolate(tag.interior_start..tag.interior_end));
            }
            TagKind::Stmt => {
                tokens.push(RawToken::Statement(tag.interior_start..tag.interior_end));
            }
            TagKind::Comment => {}
        }

        prev_end = tag.end;
    }

    if let Some(last) = tags.last() {
        apply_right_modifier(source, &mut prev_end, last, source.len());
    }
    push_text_with_escapes(prev_end, source.len(), escapes, &mut tokens);

    tokens
}

fn trim_all_boundary(nl_pos: Option<usize>, fallback: usize, source: &[u8]) -> usize {
    let stop = nl_pos.unwrap_or(fallback);
    if stop < source.len() && source[stop] == b'\n' {
        stop + 1
    } else {
        stop
    }
}

fn apply_right_modifier(source: &[u8], text_start: &mut usize, tag: &Tag, next_tag_start: usize) {
    match tag.modifier_right {
        Modifier::TrimWhitespace => {
            while *text_start < source.len() && is_whitespace(source[*text_start]) {
                *text_start += 1;
            }
        }
        Modifier::TrimAll => {
            let nl = find_byte(source, tag.end, b'\n').filter(|&n| n < next_tag_start);
            *text_start = trim_all_boundary(nl, next_tag_start, source);
        }
        Modifier::None => {}
    }
}

fn apply_left_modifier(source: &[u8], text_end: &mut usize, tag: &Tag, prev_end: usize) {
    match tag.modifier_left {
        Modifier::TrimWhitespace => {
            while *text_end > 0 && is_whitespace(source[*text_end - 1]) {
                *text_end -= 1;
            }
        }
        Modifier::TrimAll => {
            let nl = rfind_byte(source, *text_end, b'\n').filter(|&n| n > prev_end);
            *text_end = trim_all_boundary(nl, prev_end, source);
        }
        Modifier::None => {}
    }
}

fn find_byte(source: &[u8], start: usize, byte: u8) -> Option<usize> {
    source[start..]
        .iter()
        .position(|&b| b == byte)
        .map(|p| start + p)
}

fn rfind_byte(source: &[u8], end: usize, byte: u8) -> Option<usize> {
    source[..end].iter().rposition(|&b| b == byte)
}

fn is_whitespace(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_case::test_case;

    macro_rules! tag {
        ($kind:ident, $start:expr, $end:expr, $istart:expr, $iend:expr) => {
            Tag {
                kind: TagKind::$kind,
                start: $start,
                end: $end,
                interior_start: $istart,
                interior_end: $iend,
                modifier_left: Modifier::None,
                modifier_right: Modifier::None,
                backslash_count: 0,
            }
        };
        ($kind:ident, $start:expr, $end:expr, $istart:expr, $iend:expr, $ml:ident, $mr:ident) => {
            Tag {
                kind: TagKind::$kind,
                start: $start,
                end: $end,
                interior_start: $istart,
                interior_end: $iend,
                modifier_left: Modifier::$ml,
                modifier_right: Modifier::$mr,
                backslash_count: 0,
            }
        };
    }

    macro_rules! esc {
        ($pos:expr, $n:expr) => {
            EscapeSite { pos: $pos, n: $n }
        };
    }

    #[test_case("" => Vec::<Tag>::new(); "empty_input")]
    #[test_case("hello" => Vec::<Tag>::new(); "plain_text_only")]
    #[test_case("{{ var }}" => vec![tag!(Interp, 0, 9, 2, 7)]; "simple_interpolation")]
    #[test_case("{% if true %}" => vec![tag!(Stmt, 0, 13, 2, 11)]; "simple_statement")]
    #[test_case("{# note #}" => vec![tag!(Comment, 0, 10, 2, 8)]; "comment")]
    #[test_case("{{var}}" => vec![tag!(Interp, 0, 7, 2, 5)]; "interpolation_no_padding")]
    #[test_case("{{ var   }}" => vec![tag!(Interp, 0, 11, 2, 9)]; "interpolation_arbitrary_padding")]
    #[test_case("before {{ a }} after" => vec![tag!(Interp, 7, 14, 9, 12)]; "interpolation_between_text")]
    #[test_case("{{- var }}" => vec![tag!(Interp, 0, 10, 3, 8, TrimWhitespace, None)]; "left_trim_whitespace")]
    #[test_case("{{ var -}}" => vec![tag!(Interp, 0, 10, 2, 7, None, TrimWhitespace)]; "right_trim_whitespace")]
    #[test_case("{%= if true %}" => vec![tag!(Stmt, 0, 14, 3, 12, TrimAll, None)]; "left_trim_all")]
    #[test_case("{% if true =%}" => vec![tag!(Stmt, 0, 14, 2, 11, None, TrimAll)]; "right_trim_all")]
    #[test_case("{{- var =}}" => vec![tag!(Interp, 0, 11, 3, 8, TrimWhitespace, TrimAll)]; "both_sides_modifiers")]
    #[test_case("text { not a tag {{ real }}" => vec![tag!(Interp, 17, 27, 19, 25)]; "lone_brace_skipped")]
    #[test_case("{% if %}{% end %}" => vec![
        tag!(Stmt, 0, 8, 2, 6),
        tag!(Stmt, 8, 17, 10, 15),
    ]; "multiple_tags")]
    fn test_find_tags(input: &str) -> Vec<Tag> {
        find_tags(input.as_bytes()).unwrap().0
    }

    #[test_case("" => Vec::<EscapeSite>::new(); "empty_no_escapes")]
    #[test_case("hello" => Vec::<EscapeSite>::new(); "plain_text_no_escapes")]
    #[test_case("{{ var }}" => Vec::<EscapeSite>::new(); "normal_tag_no_escapes")]
    #[test_case(r#"\{{ name \}}"# => vec![esc!(1, 1), esc!(10, 1)]; "escaped_delimiters")]
    #[test_case(r#"\\\{{ name \}}"# => vec![esc!(3, 3), esc!(12, 1)]; "three_backslashes_escaped")]
    #[test_case(r#"\{% stmt \%}"# => vec![esc!(1, 1), esc!(10, 1)]; "escaped_stmt")]
    fn test_find_escapes(input: &str) -> Vec<EscapeSite> {
        find_tags(input.as_bytes()).unwrap().1
    }

    macro_rules! text {
        ($r:expr) => {
            RawToken::Text($r)
        };
    }
    macro_rules! interp {
        ($r:expr) => {
            RawToken::Interpolate($r)
        };
    }
    macro_rules! stmt {
        ($r:expr) => {
            RawToken::Statement($r)
        };
    }

    #[test_case("" => Vec::<RawToken>::new(); "empty_input")]
    #[test_case("hello" => vec![text!(0..5)]; "plain_text_only")]
    #[test_case("{{ var }}" => vec![interp!(2..7)]; "simple_interpolation")]
    #[test_case("{% if true %}" => vec![stmt!(2..11)]; "simple_statement")]
    #[test_case("before {{ x }} after" => vec![
        text!(0..7),
        interp!(9..12),
        text!(14..20),
    ]; "interpolation_between_text")]
    #[test_case("{# note #}after" => vec![text!(10..15)]; "comment_stripped,_trailing_text_kept")]
    #[test_case("hi  {{- var }}" => vec![
        text!(0..2),
        interp!(7..12),
    ]; "left_trim_whitespace")]
    #[test_case("{{ var -}}  world" => vec![
        interp!(2..7),
        text!(12..17),
    ]; "right_trim_whitespace")]
    #[test_case("  {{- var -}}  " => vec![
        interp!(5..10),
    ]; "both_sides_trim_whitespace")]
    #[test_case("{{ var =}} junk\nnext" => vec![
        interp!(2..7),
        text!(16..20),
    ]; "right_trim_all_eats_past_newline")]
    #[test_case("line1\njunk{%= stmt %}" => vec![
        text!(0..6),
        stmt!(13..19),
    ]; "left_trim_all_eats_to_previous_newline")]
    #[test_case("{{ a =}} text {{ b }}" => vec![
        interp!(2..5),
        interp!(16..19),
    ]; "right_trim_all_stops_at_next_tag")]
    #[test_case("{{ a }}{%= stmt %}" => vec![
        interp!(2..5),
        stmt!(10..16),
    ]; "left_trim_all_stops_at_previous_tag")]
    #[test_case("keep\n{%= if true =%}drop\nkeep" => vec![
        text!(0..5),
        stmt!(8..17),
        text!(25..29),
    ]; "both_sides_trim_all_multi-line")]
    #[test_case("hi {{ var =}} junk" => vec![
        text!(0..3),
        interp!(5..10),
    ]; "last_tag_right_modifier_applied")]
    #[test_case("{% if %} X {{ y }} {% end %}" => vec![
        stmt!(2..6),
        text!(8..11),
        interp!(13..16),
        text!(18..19),
        stmt!(21..26),
    ]; "mixed_statements_and_interpolations")]
    #[test_case("{{= a =}}" => vec![
        interp!(3..6),
    ]; "both_sides_trim_all_single_interpolation")]
    #[test_case("{{ a =}} text {{= b }}" => vec![
        interp!(2..5),
        interp!(17..20),
    ]; "right_trim_all_stops_at_next_trim_all_tag")]
    #[test_case("{{ a =}} foo\nbar {{= b }}" => vec![
        interp!(2..5),
        interp!(20..23),
    ]; "trim_all_converging_at_newline_between_tags")]
    #[test_case(r#"\{{ literal \}}"# => vec![
        text!(1..3),
        text!(3..12),
        text!(13..15),
    ]; "escaped_interpolation_output")]
    #[test_case(r#"before \{{ literal \}} after"# => vec![
        text!(0..7),
        text!(8..10),
        text!(10..19),
        text!(20..22),
        text!(22..28),
    ]; "escaped_interpolation_between_text")]
    #[test_case(r#"\\{{ name }}"# => vec![
        text!(0..1),
        interp!(4..10),
    ]; "even_backslashes_text_and_tag")]
    #[test_case(r#"\\\\{{ name }}"# => vec![
        text!(0..2),
        interp!(6..12),
    ]; "four_backslashes_text_and_tag")]
    #[test_case(r#"\\\{{ literal \}}"# => vec![
        text!(0..1),
        text!(3..5),
        text!(5..14),
        text!(15..17),
    ]; "three_backslashes_escaped")]
    #[test_case(r#"\{% stmt \%}"# => vec![
        text!(1..3),
        text!(3..9),
        text!(10..12),
    ]; "escaped_statement_output")]
    #[test_case(r#"\{# comment \#}"# => vec![
        text!(1..3),
        text!(3..12),
        text!(13..15),
    ]; "escaped_comment_output")]
    #[test_case(r#"{{ "}}" }}"# => vec![
        interp!(2..8),
    ]; "string_contains_closing_delim")]
    #[test_case(r#"{% "%}" %}"# => vec![
        stmt!(2..8),
    ]; "stmt_contains_closing_delim")]
    #[test_case(r##"{# "#}" #}"## => Vec::<RawToken>::new(); "comment_contains_closing_delim")]
    fn test_scan(input: &str) -> Vec<RawToken> {
        scan(input.as_bytes()).unwrap()
    }

    #[test_case("{{ unclosed" => (ErrorKind::UnclosedDelimiter, 0, 2); "unclosed_interpolation")]
    #[test_case("{% unclosed" => (ErrorKind::UnclosedDelimiter, 0, 2); "unclosed_stmt")]
    #[test_case("{# unclosed" => (ErrorKind::UnclosedDelimiter, 0, 2); "unclosed_comment")]
    #[test_case("stray }}" => (ErrorKind::StrayDelimiter, 6, 2); "stray_interpolation")]
    #[test_case("stray %}" => (ErrorKind::StrayDelimiter, 6, 2); "stray_stmt")]
    #[test_case("stray #}" => (ErrorKind::StrayDelimiter, 6, 2); "stray_comment")]
    fn test_error(input: &str) -> (ErrorKind, usize, usize) {
        let e = find_tags(input.as_bytes()).unwrap_err();
        e.destruct()
    }
}

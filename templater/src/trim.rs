use std::ops::Range;

use crate::scanner::{Modifier, Token};

/// Applies whitespace-control modifiers to scanned text ranges in place.
///
/// `source` is the original byte buffer the `tokens` ranges point into.
/// After this call, adjacent `Text` tokens reflect the `-` and `=` trimming
/// semantics of their neighboring interpolation and statement tags. Fully
/// trimmed-away text tokens are removed, leaving the stream ready for the
/// parser.
pub(crate) fn trim_tokens(tokens: &mut Vec<Token>, source: &[u8]) {
    if tokens.is_empty() {
        return;
    }

    // Resolve, for every token index, the modifier of the nearest tag to its
    // left and right that is not separated by a Barrier. We build these lookups
    // in two linear passes so trimming is O(n).
    let n = tokens.len();
    let mut left_mod = vec![Modifier::None; n];
    let mut right_mod = vec![Modifier::None; n];

    let mut last_right = Modifier::None;
    for (i, token) in tokens.iter().enumerate() {
        left_mod[i] = last_right;
        match token {
            Token::Interp { right, .. } | Token::Stmt { right, .. } => {
                last_right = *right;
            }
            Token::Barrier => last_right = Modifier::None,
            Token::Text(_) => {}
        }
    }

    let mut last_left = Modifier::None;
    for (i, token) in tokens.iter().enumerate().rev() {
        right_mod[i] = last_left;
        match token {
            Token::Interp { left, .. } | Token::Stmt { left, .. } => {
                last_left = *left;
            }
            Token::Barrier => last_left = Modifier::None,
            Token::Text(_) => {}
        }
    }

    // Mutate text tokens in place. If a text range collapses we mark it for
    // removal and drop it afterwards.
    let mut remove = vec![false; n];
    for (i, token) in tokens.iter_mut().enumerate() {
        let Token::Text(range) = token else {
            continue;
        };

        if left_mod[i] != Modifier::None {
            range.start = trim_left(source, range, left_mod[i]);
        }
        if right_mod[i] != Modifier::None {
            range.end = trim_right(source, range, right_mod[i]);
        }
        if range.start >= range.end {
            remove[i] = true;
        }
    }

    // Drop collapsed text tokens while preserving the order of everything else.
    let mut keep = Vec::with_capacity(n);
    for (i, token) in tokens.drain(..).enumerate() {
        if !remove[i] {
            keep.push(token);
        }
    }
    *tokens = keep;
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

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use test_case::test_case;

    use crate::{interp, scanner::scan, stmt, text};

    use super::*;

    #[test_case(b"{{--}}" => vec![interp!(0..6, 3..3, Dash, Dash)]; "interp_dash_empty")]
    #[test_case(b"{{==}}" => vec![interp!(0..6, 3..3, Equal, Equal)]; "interp_equal_empty")]
    #[test_case(b"hello" => vec![text!(0..5)]; "plain_text_no_tags")]
    #[test_case(b"a {{ x }}b" => vec![text!(0..2), interp!(2..9, 5..6), text!(9..10)]; "no_modifiers_with_text")]
    #[test_case(b"{%- -%}" => vec![stmt!(0..7, 4..4, Dash, Dash)]; "stmt_dash_empty")]
    #[test_case(b"{%= =%}" => vec![stmt!(0..7, 4..4, Equal, Equal)]; "stmt_equal_empty")]
    #[test_case(b"  {{- x -}}  " => vec![interp!(2..11, 6..7, Dash, Dash)]; "spaces_fully_consumed")]
    #[test_case(b"\n{{= x =}}\n" => vec![text!(0..1), interp!(1..10, 5..6, Equal, Equal)]; "equal_trims_to_newline")]
    #[test_case(b"a {{- x }}b" => vec![text!(0..1), interp!(2..10, 6..7, Dash, None), text!(10..11)]; "left_dash_trims_preceding")]
    #[test_case(b"a {{ x -}}  b" => vec![text!(0..2), interp!(2..10, 5..6, None, Dash), text!(12..13)]; "right_dash_trims_following")]
    #[test_case(b"a\n{{= x =}}\nb" => vec![text!(0..2), interp!(2..11, 6..7, Equal, Equal), text!(12..13)]; "equal_with_mixed_content")]
    fn trim_cases(source: &[u8]) -> Vec<Token> {
        let mut tokens = scan(source).unwrap();
        trim_tokens(&mut tokens, source);
        tokens
    }

    fn legal_bytes() -> impl Strategy<Value = Vec<u8>> {
        prop::collection::vec(
            any::<u8>().prop_filter("illegal char", |b| !matches!(*b, b'{' | b'}' | b'\\')),
            0..20,
        )
    }

    fn contains_newline() -> impl Strategy<Value = Vec<u8>> {
        legal_bytes().prop_filter("contains newline", |v| v.contains(&b'\n'))
    }

    fn spaces_tabs() -> impl Strategy<Value = Vec<u8>> {
        prop::collection::vec(prop::sample::select(vec![b' ', b'\t']), 1..10)
    }

    fn prefix_strategy(left_mod: Modifier) -> BoxedStrategy<Vec<u8>> {
        match left_mod {
            Modifier::Dash => (legal_bytes(), spaces_tabs())
                .prop_map(|(a, b)| [a, b].concat())
                .boxed(),
            Modifier::Equal => contains_newline().boxed(),
            _ => panic!("None excluded"),
        }
    }

    fn suffix_strategy(right_mod: Modifier) -> BoxedStrategy<Vec<u8>> {
        match right_mod {
            Modifier::Dash => (spaces_tabs(), legal_bytes())
                .prop_map(|(a, b)| [a, b].concat())
                .boxed(),
            Modifier::Equal => contains_newline().boxed(),
            _ => panic!("None excluded"),
        }
    }

    fn modifier_strategy() -> impl Strategy<Value = Modifier> {
        prop_oneof![Just(Modifier::Dash), Just(Modifier::Equal)]
    }

    proptest! {
        #[test]
        fn trim_property((left_mod, right_mod, is_stmt, prefix, suffix) in (
            modifier_strategy(),
            modifier_strategy(),
            any::<bool>(),
            modifier_strategy().prop_flat_map(prefix_strategy),
            modifier_strategy().prop_flat_map(suffix_strategy),
        )) {
            let (open, close): (&[u8], &[u8]) = if is_stmt {
                (b"{%", b"%}")
            } else {
                (b"{{", b"}}")
            };
            let left_sigil: &[u8] = match left_mod {
                Modifier::Dash => b"-",
                Modifier::Equal => b"=",
                _ => b"",
            };
            let right_sigil: &[u8] = match right_mod {
                Modifier::Dash => b"-",
                Modifier::Equal => b"=",
                _ => b"",
            };

            let mut source = Vec::new();
            source.extend_from_slice(&prefix);
            source.extend_from_slice(open);
            source.extend_from_slice(left_sigil);
            source.extend_from_slice(right_sigil);
            source.extend_from_slice(close);
            source.extend_from_slice(&suffix);

            let mut tokens = scan(&source).unwrap();
            trim_tokens(&mut tokens, &source);

            prop_assert!((1usize..=3).contains(&tokens.len()));

            let tag_idx = tokens
                .iter()
                .position(|t| matches!(t, Token::Interp { .. } | Token::Stmt { .. }))
                .expect("tag token present");

            match &tokens[tag_idx] {
                Token::Interp { left, right, .. } => {
                    prop_assert_eq!(*left, left_mod);
                    prop_assert_eq!(*right, right_mod);
                }
                Token::Stmt { left, right, .. } => {
                    prop_assert_eq!(*left, left_mod);
                    prop_assert_eq!(*right, right_mod);
                }
                _ => prop_assert!(false, "expected tag token"),
            }

            if tag_idx > 0
                && let Token::Text(range) = &tokens[tag_idx - 1] {
                    let tag_start = match &tokens[tag_idx] {
                        Token::Interp { tag, .. } | Token::Stmt { tag, .. } => tag.start,
                        _ => unreachable!(),
                    };
                    match left_mod {
                        Modifier::Dash => {
                            for &b in &source[range.end..tag_start] {
                                prop_assert!(b == b' ' || b == b'\t');
                            }
                        }
                        Modifier::Equal => {
                            prop_assert_eq!(source[range.end - 1], b'\n');
                            for &b in &source[range.end..tag_start] {
                                prop_assert_ne!(b, b'\n');
                            }
                        }
                        _ => {}
                    }
                }

            if tag_idx + 1 < tokens.len()
                && let Token::Text(range) = &tokens[tag_idx + 1] {
                    let tag_end = match &tokens[tag_idx] {
                        Token::Interp { tag, .. } | Token::Stmt { tag, .. } => tag.end,
                        _ => unreachable!(),
                    };
                    match right_mod {
                        Modifier::Dash => {
                            for &b in &source[tag_end..range.start] {
                                prop_assert!(b == b' ' || b == b'\t');
                            }
                        }
                        Modifier::Equal => {
                            prop_assert_eq!(source[range.start - 1], b'\n');
                            for &b in &source[tag_end..range.start - 1] {
                                prop_assert_ne!(b, b'\n');
                            }
                        }
                        _ => {}
                    }
                }
        }
    }
}

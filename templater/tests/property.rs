mod common;

use common::{MockRegistry, var_scope};
use proptest::prelude::*;
use templater::Template;

/// All six delimiter sequences recognized by the scanner.
const DELIMS: &[&[u8]] = &[b"{{", b"}}", b"{%", b"%}", b"{#", b"#}"];

/// Generated interpolation expressions and their expected rendered output.
///
/// Covers string literals, integer literals, boolean literals, and variables.
const INTERPS: &[(&[u8], &[u8])] = &[
    (br#""hello""#, b"hello"),
    (b"42", b"42"),
    (b"-5", b"-5"),
    (b"true", b"true"),
    (b"false", b"false"),
    (b"name", b"world"),
    (b"count", b"42"),
    (b"neg", b"-5"),
    (b"flag", b"true"),
    (b"off", b"false"),
];

/// Bytes that are safe for a plain-text segment: they cannot form or hide a
/// delimiter, nor can they be interpreted as an escape prefix.
fn safe_plain_bytes() -> Vec<u8> {
    // Printable ASCII except `{` `}` `%` `#` `\`.
    (32u8..=126)
        .filter(|&b| !matches!(b, b'{' | b'}' | b'%' | b'#' | b'\\'))
        .collect()
}

/// A generated template segment. It carries both the source bytes it
/// contributes and the expected rendered output for those bytes.
#[derive(Debug, Clone)]
enum Segment {
    /// Plain text containing no delimiter or backslash bytes.
    Plain(Vec<u8>),
    /// An odd run of backslashes followed by a delimiter. Renders as the
    /// literal delimiter bytes.
    Escaped {
        backslashes: usize,
        delim: &'static [u8],
    },
    /// A `{{ ... }}` interpolation expression.
    Interp {
        expr: &'static [u8],
        rendered: &'static [u8],
    },
    /// A `{# ... #}` comment; produces no output.
    Comment(Vec<u8>),
}

impl Segment {
    fn source_and_rendered(self) -> (Vec<u8>, Vec<u8>) {
        match self {
            Segment::Plain(bytes) => (bytes.clone(), bytes),
            Segment::Escaped { backslashes, delim } => {
                let mut src = Vec::new();
                src.extend(std::iter::repeat_n(b'\\', backslashes));
                src.extend_from_slice(delim);

                let mut out = Vec::new();
                out.extend(std::iter::repeat_n(b'\\', (backslashes - 1) / 2));
                out.extend_from_slice(delim);
                (src, out)
            }
            Segment::Interp { expr, rendered } => {
                let mut src = Vec::new();
                src.extend_from_slice(b"{{ ");
                src.extend(expr);
                src.extend_from_slice(b" }}");
                (src, rendered.to_vec())
            }
            Segment::Comment(bytes) => {
                let mut src = Vec::new();
                src.extend_from_slice(b"{# ");
                src.extend(&bytes);
                src.extend_from_slice(b" #}");
                (src, Vec::new())
            }
        }
    }
}

fn plain_strategy() -> impl Strategy<Value = Segment> {
    prop::collection::vec(prop::sample::select(safe_plain_bytes()), 0..=32).prop_map(Segment::Plain)
}

fn escaped_strategy() -> impl Strategy<Value = Segment> {
    (1usize..=7, prop::sample::select(DELIMS))
        .prop_filter("odd backslash count", |(n, _)| n % 2 == 1)
        .prop_map(|(backslashes, delim)| Segment::Escaped { backslashes, delim })
}

fn interp_strategy() -> impl Strategy<Value = Segment> {
    prop::sample::select(INTERPS).prop_map(|(expr, rendered)| Segment::Interp { expr, rendered })
}

fn comment_strategy() -> impl Strategy<Value = Segment> {
    prop::collection::vec(prop::sample::select(safe_plain_bytes()), 0..=32)
        .prop_map(Segment::Comment)
}

fn segment_strategy() -> impl Strategy<Value = Segment> {
    prop_oneof![
        plain_strategy().boxed(),
        escaped_strategy().boxed(),
        interp_strategy().boxed(),
        comment_strategy().boxed(),
    ]
}

proptest! {
    #[test]
    fn generated_templates_render_predictably(
        segments in prop::collection::vec(segment_strategy(), 0..=32)
    ) {
        let variables = var_scope();
        let mut source = Vec::new();
        let mut expected = Vec::new();

        for seg in segments {
            let (src, out) = seg.source_and_rendered();
            source.extend(src);
            expected.extend(out);
        }

        let template = Template::from_bytes(source.clone())
            .expect("generated template must parse");
        let mut actual = Vec::new();
        template
            .render(&mut actual, &variables, &MockRegistry)
            .expect("generated template must render");

        prop_assert_eq!(actual, expected);
    }
}

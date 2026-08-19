mod common;

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;

use proptest::prelude::*;
use templater::Template;
use templater::error::RegistryError;
use templater::function::FunctionRegistry;
use templater::util::RESERVED_KEYWORDS;
use templater::value::Value;

use common::MockRegistry;

const MAX_NESTING_DEPTH: usize = 2;
const MAX_LIST_LEN: usize = 8;
const MAX_STRING_BYTES: usize = 64;
const MAX_CALL_ARGS: usize = 4;
const MAX_CHAIN_CONDITIONS: usize = 6;
const MAX_BUFFER_BYTES: usize = 1024;
const MAX_NEST_BLOCK_DEPTH: usize = 3;
const MAX_FOR_ITERS: u8 = 4;
const MAX_IDENT_LEN: usize = 10;

/// Escapes a byte payload so it can be placed inside a template string literal.
///
/// Each `"` becomes `\"` and each `\\` becomes `\\\\`. All other bytes pass
/// through unchanged. Rendering the resulting literal should reproduce the
/// original `bytes` exactly.
fn string_literal_safe_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut escaped = Vec::with_capacity(bytes.len());
    for &b in bytes {
        if b == b'"' || b == b'\\' {
            escaped.push(b'\\');
        }
        escaped.push(b);
    }
    escaped
}

/// Serializes a primitive `Value` to its template source-syntax literal form —
/// the inverse of parse+eval: parsing this form yields the same `Value`. Used
/// to embed generated args back into the function-call source.
///
/// - `Str(s)` → `"s"` with `"` and `\` byte-escaped via `string_literal_safe_bytes`
/// - `Int(n)` → decimal i64 (matching `IntLit` parse)
/// - `Bool(b)` → `true` / `false` (matching `BoolLit` parse)
fn value_literal_bytes(value: &Value) -> Vec<u8> {
    match value {
        Value::Str(s) => {
            let mut bytes = vec![b'"'];
            bytes.extend(string_literal_safe_bytes(s.as_bytes()));
            bytes.push(b'"');
            bytes
        }
        Value::Int(n) => n.to_string().into_bytes(),
        Value::Bool(b) => b.to_string().into_bytes(),
        // The strategy only ever generates primitive values; lists/maps never
        // reach here.
        _ => unreachable!(),
    }
}

/// Wraps an interpolation body in `{{ ` ... ` }}` delimiters.
fn wrap_interp(body: &[u8]) -> Vec<u8> {
    let mut src = Vec::with_capacity(6 + body.len());
    src.extend_from_slice(b"{{ ");
    src.extend_from_slice(body);
    src.extend_from_slice(b" }}");
    src
}

/// Registry exposing one variadic function named `name` that joins the
/// canonical render of each argument with a single ASCII space and returns
/// `Value::Str(joined)`.
///
/// Stringification mirrors `Value::write_top` byte-for-byte (`Str` verbatim,
/// `Int` decimal, `Bool` lowercase keyword) so the property test can predict
/// the rendered output without reaching into the engine's `pub(crate)`
/// renderer. The top-level render of `Value::Str(joined)` is `joined`
/// verbatim (`value.rs:61`), so `output == joined.as_bytes()`.
struct JoinRegistry {
    name: String,
}

impl FunctionRegistry for JoinRegistry {
    fn call(&self, name: &str, args: &[Value]) -> Result<Value, RegistryError> {
        debug_assert_eq!(name, self.name);
        let mut joined = String::new();
        for (i, arg) in args.iter().enumerate() {
            if i > 0 {
                joined.push(' ');
            }
            match arg {
                Value::Str(s) => joined.push_str(s),
                Value::Int(n) => {
                    let _ = write!(joined, "{n}");
                }
                Value::Bool(b) => {
                    let _ = write!(joined, "{b}");
                }
                // Strategy only ever generates primitives; lists/maps never
                // reach the registry.
                _ => unreachable!(),
            }
        }
        Ok(Value::Str(joined))
    }
}

/// Builds a strategy for a list literal whose source bytes are identical to its
/// rendered output, with element nesting up to `max_depth` levels deep.
///
/// - depth 0 elements: string, integer, or bool leaf
/// - depth > 0 elements: any leaf OR a nested list at depth - 1
///
/// Each list picks `n` in `0..MAX_LIST_LEN` elements, joins them with `", "`,
/// and wraps in `[` ... `]`. Since the canonical render form matches the source
/// syntax
/// byte-for-byte, the same payload serves as template body and expected output.
fn list_literal(max_depth: usize) -> BoxedStrategy<Vec<u8>> {
    fn element(depth: usize) -> BoxedStrategy<Vec<u8>> {
        let str_leaf =
            proptest::collection::vec(any::<u8>(), 0..MAX_STRING_BYTES).prop_map(|bytes| {
                let mut s = Vec::with_capacity(bytes.len() + 2);
                s.push(b'"');
                s.extend(string_literal_safe_bytes(&bytes));
                s.push(b'"');
                s
            });
        let int_leaf = any::<i64>().prop_map(|n| value_literal_bytes(&Value::Int(n)));
        let bool_leaf = any::<bool>().prop_map(|b| value_literal_bytes(&Value::Bool(b)));

        if depth == 0 {
            prop_oneof![str_leaf, int_leaf, bool_leaf].boxed()
        } else {
            prop_oneof![str_leaf, int_leaf, bool_leaf, list_at_depth(depth - 1)].boxed()
        }
    }

    fn list_at_depth(depth: usize) -> BoxedStrategy<Vec<u8>> {
        proptest::collection::vec(element(depth), 0..MAX_LIST_LEN)
            .prop_map(|elems| {
                let mut s = Vec::new();
                s.push(b'[');
                for (i, e) in elems.iter().enumerate() {
                    if i > 0 {
                        s.extend_from_slice(b", ");
                    }
                    s.extend_from_slice(e);
                }
                s.push(b']');
                s
            })
            .boxed()
    }

    list_at_depth(max_depth)
}

/// Reserved keywords that cannot appear as identifiers (variables or map keys)
/// because the parser would reject them. Re-exported from `templater::util`
/// so the test filters against the same source of truth the parser enforces.
///
/// Generates a valid identifier `[A-Za-z_][A-Za-z0-9_]*` of length
/// 1..=`MAX_IDENT_LEN`, excluding reserved keywords. Returns the identifier
/// bytes.
fn ident_bytes() -> BoxedStrategy<Vec<u8>> {
    // ASCII alphabetic first char, ASCII alphanumeric subsequent chars.
    // Pattern guarantees a non-empty identifier (1..=MAX_IDENT_LEN chars).
    proptest::string::string_regex(&format!("[A-Za-z_][A-Za-z0-9_]{{0,{}}}", MAX_IDENT_LEN - 1))
        .unwrap()
        .prop_map(|s| {
            let bytes = s.into_bytes();
            // Reject reserved keywords; regenerated automatically by the
            // `prop_filter_map` wrapper below.
            bytes
        })
        .prop_filter("reject reserved keywords", |b| {
            !RESERVED_KEYWORDS.iter().any(|kw| b == kw.as_bytes())
        })
        .boxed()
}

/// Generates a leaf primitive `Value` alongside its canonical top-level render
/// bytes: unquoted string, decimal int, or lowercase bool. Top-level string
/// values are emitted verbatim by the engine, so the expected output for a
/// `Str` leaf is the raw payload bytes (no quoting, no escaping).
fn primitive_value() -> BoxedStrategy<(Value, Vec<u8>)> {
    let str_leaf = proptest::string::string_regex(&format!("[A-Za-z0-9_]{{0,{MAX_STRING_BYTES}}}"))
        .unwrap()
        .prop_map(|s| (Value::Str(s.clone()), s.into_bytes()));
    let int_leaf = any::<i64>().prop_map(|n| (Value::Int(n), n.to_string().into_bytes()));
    let bool_leaf = any::<bool>().prop_map(|b| (Value::Bool(b), b.to_string().into_bytes()));

    prop_oneof![str_leaf, int_leaf, bool_leaf].boxed()
}

/// Generates a nested `Value` paired with the path suffix leading from this
/// value down to a primitive leaf, and the primitive's expected render bytes.
///
/// Recurses up to `max_depth` levels of wrapping; the actual depth is drawn
/// uniformly from `[0, max_depth]`, so `max_depth = 2` may yield a bare leaf
/// (suffix `""`), a single wrap (`foo.bar` / `foo.0`), or a double wrap
/// (`foo.bar.0.baz`). Each wrap is either a single-element `List` (target at
/// index 0, suffix `.0`) or a single-key `Map` (target at the key, suffix
/// `.<ident>`). The composition guarantees no `MapKeyNotFound`,
/// `ListIndexOutOfBounds`, `NegativeListIndex`, or `TypeMismatch` errors at
/// render time.
fn nested_value_at_depth(max_depth: usize) -> BoxedStrategy<(Value, Vec<u8>, Vec<u8>)> {
    fn element(depth: usize) -> BoxedStrategy<(Value, Vec<u8>, Vec<u8>)> {
        let leaf = primitive_value().prop_map(|(v, expected)| (v, Vec::new(), expected));

        if depth == 0 {
            leaf.boxed()
        } else {
            let inner = element(depth - 1);
            let list_wrap =
                (0..MAX_LIST_LEN, inner.clone()).prop_map(|(index, (v, mut suffix, expected))| {
                    let mut list = vec![Value::Str("".to_string()); MAX_LIST_LEN];
                    list[index] = v;

                    let mut new_suffix = Vec::with_capacity(2 + suffix.len());
                    new_suffix.extend_from_slice(b".");
                    new_suffix.extend_from_slice(index.to_string().as_bytes());
                    new_suffix.append(&mut suffix);
                    (Value::List(list), new_suffix, expected)
                });
            let map_wrap = (ident_bytes(), inner).prop_map(|(key, (v, mut suffix, expected))| {
                let mut new_suffix = Vec::with_capacity(1 + key.len() + suffix.len());
                new_suffix.push(b'.');
                new_suffix.extend_from_slice(&key);
                new_suffix.append(&mut suffix);
                let map = BTreeMap::from([(String::from_utf8(key).unwrap(), v)]);
                (Value::Map(map), new_suffix, expected)
            });

            prop_oneof![leaf, list_wrap, map_wrap].boxed()
        }
    }

    element(max_depth)
}

type AccessPathFixture = (HashMap<String, Value>, Vec<u8>, Vec<u8>);

/// Generates the full fixture for a variable-access property test:
/// `(scope, path_bytes, expected_render_bytes)`.
///
/// The `scope` is a one-entry `HashMap` whose single `Value` is a nested
/// structure; `path_bytes` is the access expression (e.g. `foo.bar.0.baz`)
/// that navigates down to a primitive leaf; `expected_render_bytes` is the
/// canonical top-level render of that leaf (unquoted strings allowed).
///
/// The dot-only access form (no `[]`) matches the templater grammar in
/// `templater/spec/syntax.md`: `postfix = primary ( ("." identifier) | ("." integer) )*`.
fn access_path(max_depth: usize) -> BoxedStrategy<AccessPathFixture> {
    (ident_bytes(), nested_value_at_depth(max_depth))
        .prop_map(|(root, (value, suffix, expected))| {
            let mut path = Vec::with_capacity(root.len() + suffix.len());
            path.extend_from_slice(&root);
            path.extend_from_slice(&suffix);
            let scope = HashMap::from([(String::from_utf8(root).unwrap(), value)]);
            (scope, path, expected)
        })
        .boxed()
}

/// Generates the fixture for the function-call property test:
/// `(function_name, args, expected_render_bytes)`.
///
/// `function_name` is a valid identifier `[A-Za-z_][A-Za-z0-9_]*` (excluding
/// reserved keywords) — used both as the name in the source call and as the
/// name the `JoinRegistry` matches against. `args` is 0..=`max_args`
/// primitive `Value`s. `expected_render_bytes` is the args' canonical renders
/// joined by `" "` — identical to what the `JoinRegistry` produces and what
/// `Value::write_top` then renders verbatim at top level.
fn join_call(max_args: usize) -> BoxedStrategy<(String, Vec<Value>, Vec<u8>)> {
    (
        ident_bytes().prop_map(String::from_utf8),
        proptest::collection::vec(primitive_value(), 0..=max_args),
    )
        .prop_map(|(name, args): (Result<String, _>, Vec<(Value, Vec<u8>)>)| {
            let mut expected = Vec::new();
            for (i, (_, render)) in args.iter().enumerate() {
                if i > 0 {
                    expected.push(b' ');
                }
                expected.extend_from_slice(render);
            }
            let args: Vec<Value> = args.into_iter().map(|(v, _)| v).collect();
            (name.unwrap(), args, expected)
        })
        .boxed()
}

/// Generates a fixture for the `if`/`elif`/`else` property test:
/// `(n, has_else, t)` where `n ∈ [1, MAX_CHAIN_CONDITIONS]` is the number of
/// `if`/`elif` conditions, `has_else` ∈ {false, true} governs whether an
/// `{% else %}` arm exists, and `t ∈ [0, n + has_else as usize)` is the index
/// of the branch that should fire.
///
/// `t < n` means the `t`-th `if`/`elif` condition is `true` (and conditions
/// after it are arbitrary — never evaluated by the engine, which short-circuits
/// at the first `Bool(true)`). `t == n` (only when `has_else`) means all
/// conditions are `false` and the `else` arm fires.
fn if_chain_fixture() -> BoxedStrategy<(usize, bool, usize)> {
    (1..=MAX_CHAIN_CONDITIONS, any::<bool>())
        .prop_flat_map(|(n, has_else)| {
            let max_t = if has_else { n } else { n - 1 };
            (0..=max_t).prop_map(move |t| (n, has_else, t))
        })
        .boxed()
}

/// Generates an arbitrary byte buffer up to `MAX_BUFFER_BYTES` with every `{`
/// and `}` removed, so the payload can never contain a tag delimiter. Used by
/// the brace-free property tests to build inputs that are pure plain text
/// (and, inside `{# ... #}` delimiters, comment bodies that cannot close
/// early).
///
/// Rejecting braces in the generated bytes (0.78% of u8 values) keeps
/// shrinking intact, unlike a `prop_filter` over the whole buffer which would
/// reject ~99.97% of 1 KiB samples and exhaust the retry budget.
fn brace_free_bytes() -> BoxedStrategy<Vec<u8>> {
    let byte = any::<u8>().prop_filter("exclude '{' and '}'", |b| *b != b'{' && *b != b'}');
    proptest::collection::vec(byte, 0..MAX_BUFFER_BYTES).boxed()
}

/// A generated nested block tree of `if`/`for` nodes terminated by leaf
/// bodies. One shared shape for both nested-block property tests:
///   - `block_tree_a` forces `cond = true` and `else_body = None`.
///   - `block_tree_b` draws `cond` and an optional `else_body` randomly.
///
/// Each `Leaf` carries a single-byte tag assigned post-generation by
/// `assign_tags`, distinct per leaf position in emission order so the
/// oracle's output fingerprints which path through the tree executed.
#[derive(Clone, Debug)]
enum Block {
    Leaf(u8),
    If {
        cond: bool,
        body: Box<Block>,
        else_body: Option<Box<Block>>,
    },
    For {
        iters: u8,
        body: Box<Block>,
    },
}

/// Number of distinct `Leaf` nodes in the tree. With `MAX_NEST_BLOCK_DEPTH`
/// the true max is `2^MAX_NEST_BLOCK_DEPTH` (Strategy B, every `If` has both
/// arms) or 1 (Strategy A, no `else` branch exists); the `MAX_LEAF_FILTER`
/// guard is purely defensive.
fn leaf_count(b: &Block) -> usize {
    match b {
        Block::Leaf(_) => 1,
        Block::If {
            body, else_body, ..
        } => leaf_count(body) + else_body.as_ref().map_or(0, |eb| leaf_count(eb)),
        Block::For { body, .. } => leaf_count(body),
    }
}

const MAX_LEAF_FILTER: usize = 200;

/// Walks the tree in emission order and assigns each `Leaf` a successive
/// counter byte. Returns `false` (to trigger regeneration) if more than
/// `MAX_LEAF_FILTER` leaves would be tagged — never triggered at
/// `MAX_NEST_BLOCK_DEPTH`, kept as a defensive cap.
fn assign_tags(b: &mut Block, counter: &mut u8) -> bool {
    match b {
        Block::Leaf(slot) => {
            *slot = *counter;
            *counter = counter.wrapping_add(1);
            (*counter as usize) <= MAX_LEAF_FILTER
        }
        Block::If {
            body, else_body, ..
        } => {
            assign_tags(body, counter)
                && else_body.as_mut().is_none_or(|eb| assign_tags(eb, counter))
        }
        Block::For { body, .. } => assign_tags(body, counter),
    }
}

/// Oracle: interprets the tree exactly as the templater should, appending
/// each executed leaf's tag to `out`. Pure, deterministic, no registry or
/// scope. Mirrors `eval.rs:79-130` block-stack semantics: untaken `if`
/// arms and empty iterables skip their body.
fn run(b: &Block, out: &mut Vec<u8>) {
    match b {
        Block::Leaf(tag) => out.push(*tag),
        Block::If {
            cond,
            body,
            else_body,
        } => {
            if *cond {
                run(body, out);
            } else if let Some(eb) = else_body {
                run(eb, out);
            }
        }
        Block::For { iters, body } => {
            for _ in 0..*iters {
                run(body, out);
            }
        }
    }
}

/// Emits valid template source for the tree, left-to-right, matching
/// `run`'s traversal. `for` loops use `[0, 1, ..., iters-1]` as the
/// iterable; the loop variable `x` is never referenced (this test is
/// purely about block structure, not loop-variable scoping).
fn emit(b: &Block, src: &mut Vec<u8>) {
    match b {
        Block::Leaf(tag) => src.push(*tag),
        Block::If {
            cond,
            body,
            else_body,
        } => {
            src.extend_from_slice(b"{% if ");
            src.extend_from_slice(if *cond { b"true" } else { b"false" });
            src.extend_from_slice(b" %}");
            emit(body, src);
            if let Some(eb) = else_body {
                src.extend_from_slice(b"{% else %}");
                emit(eb, src);
            }
            src.extend_from_slice(b"{% end %}");
        }
        Block::For { iters, body } => {
            src.extend_from_slice(b"{% for x in [");
            for i in 0..*iters {
                if i > 0 {
                    src.extend_from_slice(b", ");
                }
                src.extend_from_slice(i.to_string().as_bytes());
            }
            src.extend_from_slice(b"] %}");
            emit(body, src);
            src.extend_from_slice(b"{% end %}");
        }
    }
}

/// randomized `if` conditions with optional `else` arms. Adds
/// branch short-circuit coverage across nesting: a `false` `if` body inside
/// a taken `for` must be skipped (and its `else`, if any, taken) without
/// error, mirroring the tree interpreter. The `else` arm gives the tree up
/// to 2 children per `If`, so leaf tags become a real fingerprint of which
/// path executed.
fn block_tree(max_depth: u32) -> BoxedStrategy<Block> {
    fn node(depth: u32) -> BoxedStrategy<Block> {
        if depth == 0 {
            Just(Block::Leaf(0)).boxed()
        } else {
            let if_branch = (
                any::<bool>(),
                node(depth - 1),
                proptest::option::of(node(depth - 1)),
            )
                .prop_map(|(cond, body, else_body)| Block::If {
                    cond,
                    body: Box::new(body),
                    else_body: else_body.map(Box::new),
                });
            let for_branch =
                (0u8..=MAX_FOR_ITERS, node(depth - 1)).prop_map(|(iters, b)| Block::For {
                    iters,
                    body: Box::new(b),
                });
            prop_oneof![Just(Block::Leaf(0)), if_branch, for_branch].boxed()
        }
    }
    node(max_depth)
        .prop_filter("at most MAX_LEAF_FILTER leaves", |t| {
            leaf_count(t) <= MAX_LEAF_FILTER
        })
        .boxed()
}

proptest! {
    // Generate arbitrary byte buffers up to 1 KiB. Delimiters, invalid UTF-8,
    // embedded NULs, and binary garbage are all fair game: the engine must
    // reject malformed input with an error, never panic.
    #[test]
    fn random_bytes_do_not_panic(src in proptest::collection::vec(any::<u8>(), 0..MAX_BUFFER_BYTES)) {
        let template = Template::from_bytes(src.clone());
        let mut out = Vec::new();
        let _ = template.render(&mut out, &HashMap::new(), &MockRegistry);
    }

    // A byte payload free of `{` and `}` contains no tag delimiter, so the
    // lexer sees a single plain-text token and the engine renders the input
    // verbatim — bytes in, bytes out.
    #[test]
    fn brace_free_bytes_render_unchanged(bytes in brace_free_bytes()) {
        let mut out = Vec::new();
        Template::from_bytes(bytes.clone())
            .render(&mut out, &HashMap::new(), &MockRegistry)
            .unwrap();

        prop_assert_eq!(out, bytes);
    }

    // The same brace-free payload wrapped in `{# ... #}` comment delimiters
    // produces no output at all: a comment is stripped by the lexer, and with
    // braces excluded the inner bytes can never form a stray `#}` that would
    // close the comment early.
    #[test]
    fn brace_free_bytes_in_comment_render_nothing(bytes in brace_free_bytes()) {
        let mut src = Vec::with_capacity(6 + bytes.len());
        src.extend_from_slice(b"{# ");
        src.extend_from_slice(&bytes);
        src.extend_from_slice(b" #}");

        let mut out = Vec::new();
        Template::from_bytes(src)
            .render(&mut out, &HashMap::new(), &MockRegistry)
            .unwrap();

        prop_assert!(out.is_empty());
    }

    // A string literal whose special bytes have been escaped renders back to the
    // original payload: escape processing inverts the escaping applied by
    // `literal_safe_bytes`, and top-level strings are emitted verbatim.
    #[test]
    fn escaped_string_literal_roundtrips_to_original(bytes in proptest::collection::vec(any::<u8>(), 0..MAX_BUFFER_BYTES)) {
        let escaped = string_literal_safe_bytes(&bytes);

        let mut src = Vec::new();
        src.extend_from_slice(br#"{{ ""#);
        src.extend_from_slice(&escaped);
        src.extend_from_slice(br#"" }}"#);

        let mut out = Vec::new();
        Template::from_bytes(src)
            .render(&mut out, &HashMap::new(), &MockRegistry)
            .unwrap();

        prop_assert_eq!(out, bytes);
    }

    // Any i64 integer literal renders as its canonical decimal form.
    #[test]
    fn int_literal_roundtrips_to_canonical_form(n in any::<i64>()) {
        let lit = value_literal_bytes(&Value::Int(n));
        let src = wrap_interp(&lit);

        let mut out = Vec::new();
        Template::from_bytes(src)
            .render(&mut out, &HashMap::new(), &MockRegistry)
            .unwrap();

        prop_assert_eq!(out, lit);
    }

    #[test]
    fn bool_literal_roundtrips_to_canonical_form(b in any::<bool>()) {
        let lit = value_literal_bytes(&Value::Bool(b));
        let src = wrap_interp(&lit);

        let mut out = Vec::new();
        Template::from_bytes(src)
            .render(&mut out, &HashMap::new(), &MockRegistry)
            .unwrap();

        prop_assert_eq!(out, lit);
    }

    // A list literal of strings, integers, bools, and nested lists (up to
    // `MAX_NESTING_DEPTH` levels of nesting) renders back to its own canonical byte form: `[`
    // followed by `, `-joined elements followed by `]`, where strings are
    // quoted, ints are decimal, and bools are `true`/`false`.
    #[test]
    fn list_literal_roundtrips_to_canonical_form(list in list_literal(MAX_NESTING_DEPTH)) {
        let src = wrap_interp(&list);

        let mut out = Vec::new();
        Template::from_bytes(src)
            .render(&mut out, &HashMap::new(), &MockRegistry)
            .unwrap();

        prop_assert_eq!(out, list);
    }

    // A variable-access path navigates a nested Value (up to `MAX_NESTING_DEPTH`
    // levels of List/Map wrapping) down to a primitive leaf and renders the
    // leaf's canonical top-level form: unquoted string, decimal int, or
    // lowercase bool. Exercises variable lookup, dot (Map key) access, and
    // dot-index (List) access in mixed chains such as `foo.bar.0.baz.0.qux`.
    #[test]
    fn variable_access_roundtrips_nested_value(
        (scope, path, expected) in access_path(MAX_NESTING_DEPTH)
    ) {
        let src = wrap_interp(&path);

        let mut out = Vec::new();
        Template::from_bytes(src)
            .render(&mut out, &scope, &MockRegistry)
            .unwrap();

        prop_assert_eq!(out, expected);
    }

    // A function call with 0..=4 primitive arguments (string, int, or bool)
    // renders as the args' canonical forms joined by a single ASCII space.
    // Each arg is parsed, evaluated to a `Value`, passed to the registry as
    // `&[Value]`; the registry joins the args' `write_top` forms into a
    // `Value::Str`, which `write_top` then emits verbatim at top level.
    // Exercises function-name resolution, arg evaluation, variadic arity
    // (including the 0-arg edge case), and the `FnCall → write_top` render
    // path for all three primitive value types.
    #[test]
    fn function_call_renders_space_joined_args(
        (name, args, expected) in join_call(MAX_CALL_ARGS)
    ) {
        let mut body = Vec::new();
        body.extend_from_slice(name.as_bytes());
        body.push(b'(');
        for (i, arg) in args.iter().enumerate() {
            if i > 0 {
                body.extend_from_slice(b", ");
            }
            body.extend_from_slice(&value_literal_bytes(arg));
        }
        body.push(b')');
        let src = wrap_interp(&body);

        let registry = JoinRegistry { name: name.clone() };
        let mut out = Vec::new();
        Template::from_bytes(src)
            .render(&mut out, &HashMap::new(), &registry)
            .unwrap();

        prop_assert_eq!(out, expected);
    }

    // An if/elif/else chain picks exactly one branch to fire. Branches before
    // the firing one have `false` conditions; the firing `if`/`elif` branch is
    // `true`; conditions after the firing one are random (the engine short-
    // circuits at the first `Bool(true)` and never evaluates them, per
    // `templater/spec/syntax.md` §`if` and `eval.rs:79-91`). If no `if`/`elif`
    // fires, the optional `{% else %}` arm (m ∈ {0, 1}) supplies the taken
    // body. Each branch body is a single ASCII digit `b'0' + branch_index`
    // written as plain text, so the rendered output must equal exactly
    // `[b'0' + t]`. Total branches ≤ 9 fits the 0..9 digit range.
    #[test]
    fn if_elif_else_branches_match_taken(
        (n, has_else, t) in if_chain_fixture(),
        trailing in proptest::collection::vec(any::<bool>(), 0..MAX_CHAIN_CONDITIONS)
    ) {
        let mut src = Vec::new();
        let mut trailing_iter = trailing.into_iter();
        for i in 0..n {
            let head: &[u8] = if i == 0 { b"if " } else { b"elif " };
            src.extend_from_slice(b"{% ");
            src.extend_from_slice(head);
            let cond = if i < t {
                // Before the firing branch: must be false.
                false
            } else if i == t {
                // The firing if/elif condition.
                true
            } else {
                // After the firing branch: never evaluated; randomize to
                // stress the short-circuit guarantee.
                trailing_iter.next().unwrap_or(false)
            };
            src.extend_from_slice(cond.to_string().as_bytes());
            src.extend_from_slice(b" %}");
            src.push(b'0' + i as u8);
        }
        if has_else {
            src.extend_from_slice(b"{% else %}");
            src.push(b'0' + n as u8);
        }
        src.extend_from_slice(b"{% end %}");

        let mut out = Vec::new();
        Template::from_bytes(src)
            .render(&mut out, &HashMap::new(), &MockRegistry)
            .unwrap();

        let expected = vec![b'0' + t as u8];
        prop_assert_eq!(out, expected);
    }

    // A for-loop over a List renders each item's top-level form in order: the
    // body `{{ x }}` per iteration emits the item's `write_top` bytes, so the
    // rendered output equals the concatenation of those renders. An empty List
    // short-circuits — the body never runs and no Loop frame is pushed (per
    // `eval.rs:113-119`), so output is empty. Uses `primitive_value()` items
    // whose top-level render (unquoted string / decimal int / lowercase bool)
    // is identical whether standalone or iterated through a `for` body.
    #[test]
    fn for_loop_renders_each_item(
        items in proptest::collection::vec(primitive_value(), 0..MAX_LIST_LEN)
    ) {
        let mut iter_src = Vec::new();
        iter_src.push(b'[');
        for (i, (v, _)) in items.iter().enumerate() {
            if i > 0 {
                iter_src.extend_from_slice(b", ");
            }
            iter_src.extend_from_slice(&value_literal_bytes(v));
        }
        iter_src.push(b']');

        let mut src = Vec::new();
        src.extend_from_slice(b"{% for x in ");
        src.extend_from_slice(&iter_src);
        src.extend_from_slice(b" %}{{ x }}{% end %}");

        let mut expected = Vec::new();
        for (_, render) in &items {
            expected.extend_from_slice(render);
        }

        let mut out = Vec::new();
        Template::from_bytes(src)
            .render(&mut out, &HashMap::new(), &MockRegistry)
            .unwrap();

        prop_assert_eq!(out, expected);
    }

    // randomized `if` conditions with optional `else` arms add
    // branch short-circuit coverage across nesting: a `false` `if` body
    // inside a taken `for` must be skipped (and its `else`, if any, taken)
    // without error, mirroring the tree interpreter. Leaf tags fingerprint
    // which path executed, so the rendered output must equal the oracle's
    // byte-for-byte.
    #[test]
    fn nested_blocks_match_interpreted_output(
        tree in block_tree(MAX_NEST_BLOCK_DEPTH as u32)
    ) {
        let mut tree = tree;
        let mut counter = 0u8;
        prop_assume!(assign_tags(&mut tree, &mut counter));

        let mut src = Vec::new();
        emit(&tree, &mut src);

        let mut expected = Vec::new();
        run(&tree, &mut expected);

        let mut out = Vec::new();
        Template::from_bytes(src)
            .render(&mut out, &HashMap::new(), &MockRegistry)
            .unwrap();

        prop_assert_eq!(out, expected);
    }
}

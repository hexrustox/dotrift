# Dotrift Template Syntax

## Delimiters

```
{{ ... }}     Interpolation
{% ... %}     Statement
{# ... #}     Comment
```

Whitespace (spaces, tabs, and newlines) immediately inside the opening and closing delimiters is optional and trimmed before parsing the inner expression/statement. `{{x}}`, `{{ x }}`, and `{{  x  }}` are equivalent.

A statement or interpolation tag may span multiple physical lines; newlines
inside a tag (outside a string literal) are inner whitespace, trimmed at the
tag's edges and acting as token separators between the keyword and its
operands. Multi-line strings (see Literals) remain a special case — the `\n`
there lives inside the string literal, not the tag surface.

Tags do not nest. The opening `{{` or `{%` starts a tag; the lexer scans to
the first matching closing delimiter, recognizing string literals verbatim —
`}}` inside a string literal does not close the tag. A `{{` encountered inside
a tag body is treated as ordinary text; if the resulting tag body fails to
parse as an expression or statement, it is a parse-time error.

## Whitespace Control

Modifiers (`-`, `=`) attach only to interpolation and statement delimiters (`{{ }}`, `{% %}`); comment delimiters (`{# #}`) cannot carry a modifier. Comment delimiters still act as stops for `=` scanning from other tags (see below).

| Modifier | Position | Effect |
|---|---|---|
| `-` | `{{-` / `{%-` or `-}}` / `-%}` | Trim adjacent spaces/tabs on that side |
| `=` | `{{=` / `{%=` | Scan left from tag's opening `{`, deleting plain text/spaces, until `\n`/SOF or another tag's `}}`/`%}`/`#}` (closing delimiter of any tag, including comments). Stop before `\n`; stop at the closing delimiter |
| `=` | `=}}` / `=%}` | Scan right from tag's closing `}`, deleting plain text/spaces, until `\n`/EOF or another tag's `{{`/`{%`/`{#` (opening delimiter of any tag, including comments). Eat through and include the `\n`; stop before the opening delimiter |

Modifiers are mutually exclusive on the same side — only one may appear at start and one at end. Left and right modifiers are independent.

`=` is directional and never crosses a tag delimiter on the same line. All six tag delimiters (`{{` `}}` `{%` `%}` `{#` `#}`) act as barriers.

Left `=` (`{{=`, `{%=`) scans leftward from the tag's opening `{`, deleting
every character until it reaches a `\n` (or start of file) and stops
**before** that `\n`, or until it reaches the closing delimiter (`}}`, `%}`,
or `#}`) of another tag on the same line (any tag, including comments) and
stops **at** that delimiter, preserving it. Only plain text and whitespace
between the boundary and the tag is deleted.

Right `=` (`=}}`, `=%}`) scans rightward from the tag's closing `}`, deleting
every character until it reaches a `\n` (or end of file) and eats **through and
includes** that `\n`, or until it reaches the opening delimiter (`{{`, `{%`,
or `{#`) of another tag on the same line (any tag, including comments) and
stops **before** that delimiter, preserving it. Only plain text and whitespace
between the tag and the boundary is deleted.

When two `=`-tags share a line, the region between them is eaten — each
scanner stops at the other tag's delimiter, and the plain text between them
is deleted.

`=` scans the byte stream outside of string-literal content; a `\n` inside a
string literal does not act as a stop for the scanner, and is not eaten.

`\r` (carriage return) is treated as ordinary plain text by the `=` scanner,
not as a line terminator; only `\n` terminates lines for the purposes of
whitespace control.

### Examples

```
# this is a comment {%= if true =%}
hello
# this is a comment {%= end =%}
```
Becomes:
```
hello
```

```
prefix text {%= var =%} suffix text
next line
```
If `var` is `"hello"`, becomes:
```
hellonext line
```

```
{{ expr =}} mid {{= expr }}
```
Becomes:
```
{{ expr =}}{{= expr }}
```

## Expressions

No operators. No bracket-index access (`[]`). Dot access only.

```
expr       → list_literal | postfix
postfix    → primary (("." identifier) | ("." integer))*
primary    → literal | identifier | fn_call | "(" expr ")"
fn_call    → identifier "(" (expr ("," expr)*)? ")"
identifier → [A-Za-z_][A-Za-z0-9_]*
integer    → ["-"] digit+
list_literal → "[" (expr ("," expr)*)? "]"
```

### Keywords

`if`, `elif`, `else`, `for`, `in`, and `end` are reserved keywords and
cannot be used as variable names or function names. Using a keyword as an
identifier is a parse-time error.

### Literals

```
"string"              String (double-quoted. Supports `\"` (literal `"`) and `\\` (literal `\`)
                       escape sequences. Any other `\X` renders as both characters verbatim.
                       String literals may span multiple lines (raw newlines are preserved).
                       Inside a string literal, the escape rule (§ Escaping) does not apply —
                       `{`, `}`, `{{`, `}}`, `{%`, `%}`, `{#`, `#}` are all literal text.
                       Only `\"` and `\\` are interpreted. `}` inside a string literal is treated
                       as literal text, not a delimiter close.)
42  -7                Int (backed by i64; literals outside i64 range are
                       a parse-time error. Decimal, optional leading `-`
                       sign, no `+` sign, no underscores. Leading zeros are
                       allowed and parsed as decimal (e.g. `007` → `7`).
                       `+7` is a parse-time error. Must fit signed i64 range
                       `[-9223372036854775808, 9223372036854775807]`; Numbers
                      out of range are parse-time errors.)
true  false           Bool
[ expr, expr, ... ]   List

An empty list literal `[]` is valid and denotes the empty List.
Whitespace inside list and function-call brackets is optional and
trimmed; `["a","b"]`, `[ "a" , "b" ]`, and multi-line forms are
equivalent.
```

No map literal. No trailing commas.

### Variables and Dot Access

```
var
obj.field
list.0         (integer index)
fn().field
```

`.identifier` performs Map key lookup (receiver must be a Map). The
identifier must match `[A-Za-z_][A-Za-z0-9_]*`; an empty identifier
after `.` is a parse-time error.
`.integer` performs List index lookup (receiver must be a List).
`list.field` and `map.0` are render-time type errors.

Map keys may be any String, but dot syntax only reaches keys that
match the identifier grammar. Keys containing digits at the start,
hyphens, spaces, dots, or other non-identifier characters require a
host-provided function to retrieve.

List index must be a non-negative Int; negative indices are a
render-time error.

### Function Calls

Nested calls only — no pipe chaining.

```
eq(a, b)
and(gt(x, 3), lt(y, 10))
join(":", home(), ".bin")
```

`identifier()` is a valid call with zero arguments; the host registry decides whether the function accepts zero args. Trailing commas in function call argument lists are a parse-time error, same as list literals.

Function names follow the same identifier grammar as variables and are
subject to the same keyword reservation (see Keywords). `if()`, `1st()`,
and `kebab-fn()` are parse-time errors.

## Statements

Inside a statement tag, the keyword (`if`, `elif`, `else`, `for`, `in`,
`end`) and its operands must be separated by whitespace. `{%ifx%}` is parsed
as a single identifier `ifx`, not as the keyword `if` followed by `x`.

### `if`

```
{% if expr %} ... {% elif expr %} ... {% else %} ... {% end %}
```

Condition must evaluate to **Bool**. Any other type is a render-time error.

`elif` and `else` are optional. First true branch runs; at most one branch executes.

### `for`

```
{% for var in expr %} ... {% end %}
```

`expr` must evaluate to **List**. Render-time error otherwise.

Expression can be a variable, function call, member access, or list literal:

```
{% for x in items %}
{% for x in fn() %}
{% for x in obj.field %}
{% for x in ["a", "b", fn()] %}
```

`var` shadows any same-named outer variable for the duration of the loop body. The outer binding is restored after `{% end %}`.

If the iterable is empty, the loop body never runs and the loop variable is never bound; the outer binding (if any) is preserved across the entire `{% for %}…{% end %}`.

### Nesting

`{% end %}` closes the innermost unclosed block at the same nesting level. Nesting follows LIFO order — the most recently opened block is closed first.

```dotrift
{% if a %}
  {% for x in items %}
    {% if b %}
    {% end %}
    {# -- closes inner if (b), not the for #}
  {% end %}
  {# -- closes for, not the outer if #}
{% end %}
```

`{% end %}` is the only block-closing form; `{% endif %}` and `{% endfor %}`
are not recognized and are parse-time errors (unrecognized statement).

## Comments

```
{# spans lines #}
```

Stripped by the lexer. No output. No interaction with whitespace modifiers.

The escape rule (§ Escaping) applies uniformly to all six delimiters, including comment delimiters (`{#`, `#}`). Inside a comment, inner escapes are inert — the comment's content is gobbled by the lexer before any escape interpretation runs on it. Inside a comment, an escaped `#}` (odd number of preceding `\` chars) is treated as literal text and stripped along with the comment; the comment continues until the next unescaped `#}`.

## Escaping

A delimiter pair (`` {{ ``, `` {% ``, `` {# ``, `` }} ``, `` %} ``, `` #} ``) can be escaped by preceding it with an odd number of backslashes. Let `n` be the number of consecutive `\` chars immediately before the delimiter:

- **`n` even** (including 0): the delimiter is **not escaped**. The `n` `\` chars render as `n/2` literal `\` chars, and the delimiter is processed normally.
- **`n` odd**: the delimiter **is escaped**. The `n` `\` chars render as `(n-1)/2` literal `\` chars, and the delimiter chars are rendered as literal text.

| Source | n | Literal `\` output | Delimiter behavior |
|---|---|---|---|
| `{{` | 0 (even) | 0 | Tag |
| `\{{` | 1 (odd) | 0 | Literal `{{` |
| `\\{{` | 2 (even) | 1 | Tag |
| `\\\{{` | 3 (odd) | 1 | Literal `{{` |
| `\\\\{{` | 4 (even) | 2 | Tag |

The escape rule fires as part of tokenization, before any `=` whitespace modifier is applied. If `n` is odd, the delimiter (including any `=` sigil) is treated as literal text — no tag is created, so no `=` modifier is in effect. If `n` is even, the delimiter is a real tag, the quantized `n/2` literal `\` chars render adjacent to it, and a `=` modifier on that side may eat those `\` chars as plain text if they fall within its scan.

An escaped opening delimiter means the tag is never created, so its closing delimiter — if unescaped — will be a stray delimiter error:

```
\{{}}        → error: stray closing delimiter
\{{\}}       → outputs literal "{{}}"
```

## Scoping

```
Outer scope
  └─ For scope (loop var shadows outer)
      └─ Nested for scope
```

Variable resolution walks from innermost scope outward. Undefined variable is a render-time error.

`for` introduces a nested scope (the loop variable shadows outer bindings for the body and restores them at `{% end %}`). `if` / `elif` / `else` introduce no new scope; references inside a branch resolve against the enclosing scope (which may be a `for` body).

## Value Types

| Type | Literal |
|---|---|
| String | `"..."` |
| Int | `42` `-7` |
| Bool | `true` `false` |
| List | `[e1, e2]` |
| Map | none |

All types are first-class: holdable by variables, returnable by functions, passable as arguments. Map is the only type without a literal form.

There is no Float type. Function registries returning fractional values must
encode them as String (formatted) or Int (truncated/scaled); the templater
does not perform floating-point arithmetic.

## Functions

### Behavior

- Resolved at render time against a global function registry.
- Undefined function: render-time error.
- Return typed values.
- Context expecting a specific type (Bool in `if`, List in `for`) errors on mismatch.
- No falsy/truthy coercion — `if` takes Bool; `for` takes List.

### Type Rules

All arguments and return values are typed. Type mismatches are render-time errors.

## Error Semantics

All errors are render-time (not parse-time unless syntax is malformed —
including an empty identifier after `.`, an empty interpolation `{{}}` /
`{{ }}` (missing expression), an empty statement `{% %}` / `{%  %}` (missing
statement), a malformed statement head (missing condition in `if`, missing
`in <expr>` in `for`, malformed binding, unrecognized trailing tokens), or
an integer literal overflows i64):

Render-time errors fire only on actually-executed content: expressions
inside a branch not taken, or inside a `for` body that never iterates,
are not evaluated and do not error.

- Undefined variable
- Undefined function
- Wrong number of arguments
- Type mismatch (non-Bool in `if` condition, non-List in `for` iterable, wrong argument types)
- Index access on a String value (dot notation, e.g., `"str".0`)
- List index out of bounds
- Map key not found
- Map key access (`.identifier`) on a non-Map value
- List index access (`.integer`) on a non-List value
- Negative list index
- `{% end %}` without matching `{% if %}` or `{% for %}`
- Unclosed `{% if %}` or `{% for %}` (missing `{% end %}`)

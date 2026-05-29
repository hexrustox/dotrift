# Dotrift Template Syntax

## Delimiters

```
{{ ... }}     Interpolation
{% ... %}     Statement
{# ... #}     Comment
```

## Whitespace Control

Modifiers apply to **all** delimiters (`{{ }}`, `{% %}`). Comments are not affected.

| Modifier | Position | Effect |
|---|---|---|
| `-` | `{{-` / `{%-` or `-}}` / `-%}` | Trim adjacent spaces/tabs on that side |
| `=` | `{{=` / `{%=` | Eat from previous `\n` (or SOF) to tag start |
| `=` | `=}}` / `=%}` | Eat from tag end through next `\n` (or EOF) |

Modifiers are mutually exclusive on the same side — only one may appear at start and one at end. Left and right modifiers are independent.

`=` eats the entire line the tag occupies — the tag produces no output; the expression/body it wraps produces output normally.

`=` stops at the opening (`{{`, `{%`) or closing (`}}`, `%}`) delimiter of any other expression or statement on the same line. Other tags on the line are preserved. Only whitespace and plain text between delimiters is eaten.

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
hello
next line
```

## Expressions

No operators. No bracket-index access (`[]`). Dot access only.

```
expr       → list_literal | postfix
postfix    → primary ("." identifier)*
primary    → literal | identifier | fn_call | "(" expr ")"
fn_call    → identifier "(" (expr ("," expr)*)? ")"
```

### Literals

```
"string"              String (double-quoted. Supports `\"` and `\\` escape sequences.
                       `}` inside a string literal is treated as literal text, not a delimiter close.)
42  -7                Int
true  false           Bool
[ expr, expr, ... ]   List
```

No map literal. No trailing commas.

### Variables and Dot Access

```
var
obj.field
list.0         (integer index)
fn().field
```

### Function Calls

Nested calls only — no pipe chaining.

```
eq(a, b)
and(gt(x, 3), lt(y, 10))
join(":", home(), ".bin")
```

## Statements

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

## Comments

```
{# spans lines #}
```

Stripped by the lexer. No output. No interaction with whitespace modifiers.

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

An escaped opening delimiter means the tag is never created, so its closing delimiter — if unescaped — will be a stray delimiter error:

```
"\{{}}"       → error: stray closing delimiter
"\{{\}}"      → outputs literal "{{}}"
```

## Scoping

```
Outer scope
  └─ For scope (loop var shadows outer)
      └─ Nested for scope
```

Variable resolution walks from innermost scope outward. Undefined variable is a render-time error.

## Value Types

| Type | Literal |
|---|---|
| String | `"..."` |
| Int | `42` `-7` |
| Bool | `true` `false` |
| List | `[e1, e2]` |
| Map | none |

All types are first-class: holdable by variables, returnable by functions, passable as arguments. Map is the only type without a literal form.

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

All errors are render-time (not parse-time unless syntax is malformed):

- Undefined variable
- Undefined function
- Wrong number of arguments
- Type mismatch (non-Bool in `if` condition, non-List in `for` iterable, wrong argument types)
- Index access on a String value (dot notation, e.g., `"str".0`)
- List index out of bounds
- Map key not found
- `{% end %}` without matching `{% if %}` or `{% for %}`
- Unclosed `{% if %}` or `{% for %}` (missing `{% end %}`)

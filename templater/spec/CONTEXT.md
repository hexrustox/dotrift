# Templater

The standalone template engine of dotrift: a text document with embedded tags
that is rendered into output by evaluating expressions, executing statements,
and emitting surviving plain text.

## Language

### Tags & Text

**Tag**:
A region delimited by `{{ }}`, `{% %}`, or `{# #}` recognized by the lexer. Tags do not nest; a string literal inside a tag body shields any inner delimiter.
_Avoid_: directive, block (block is reserved for statement pairs — see Evaluation)

**Delimiter**:
One of the six character sequences `{{`, `}}`, `{%`, `%}`, `{#`, `#}` that open or close a tag. All six act as barriers for the `=` whitespace modifier.
_Avoid_: bracket, marker

**Interpolation**:
A `{{ }}` tag whose evaluated expression's value is inserted into the output.
_Avoid_: substitution, placeholder, variable tag

**Statement**:
A `{% %}` tag performing control flow — `if`, `elif`, `else`, `for`, `in`, or `end`.
_Avoid_: directive, command

**Comment**:
A `{# #}` tag stripped by the lexer; produces no output and carries no modifier.
_Avoid_: note, remark

**Plain text**:
Bytes of the template outside any tag, plus escaped delimiters, emitted verbatim to the output.
_Avoid_: literal text, raw text

**Escape**:
An odd number of consecutive `\` chars immediately before a delimiter, causing the delimiter to render as plain text rather than open or close a tag. An even count (including zero) leaves the delimiter active.
_Avoid_: backslash escape, escaping (overloaded with string-literal escapes)

**Stray delimiter**:
A closing delimiter with no matching unescaped opening tag, or vice versa. A parse-time error.
_Avoid_: orphan delimiter, dangling delimiter

### Whitespace Control

**Modifier**:
A `-` or `=` sigil attached to an interpolation or statement delimiter (never a comment), controlling adjacent plain text. `-` trims adjacent spaces and tabs on its side; `=` scans to a line boundary or another tag's delimiter, eating intervening plain text and (on the right) the terminating newline.
_Avoid_: trimmer, whitespace trim

### Evaluation

**Expression**:
A syntactic unit evaluated to a typed value: a literal, variable, function call, dot access, or list literal. No operators; no bracket indexing.
_Avoid_: term, value

**Block**:
An opening `if` or `for` statement paired with its matching `{% end %}`. Blocks nest in LIFO order; only `{% end %}` closes a block.
_Avoid_: region, section

**Branch**:
An `if`, `elif`, or `else` arm of an `if` block. At most one branch executes; branches introduce no new scope.
_Avoid_: case, arm

**Loop variable**:
The binding introduced by `for … in`, which shadows any outer binding of the same name for the body and is restored at the matching `{% end %}`. If the iterable is empty, the binding is never created.
_Avoid_: iteration variable, item variable

**Iterable**:
The expression value of a `for` statement. Must be a List; any other type is a render-time error.
_Avoid_: collection, sequence

**Scope**:
A binding layer walked by variable resolution from innermost outward. Only `for` introduces a scope; `if`/`elif`/`else` do not. An undefined variable is a render-time error.
_Avoid_: environment, context (overloaded with host context)

**Function registry**:
The host-provided table of named functions resolved at render time. An undefined name in call position is a render-time error; function names share the identifier grammar and keyword reservation with variables.
_Avoid_: function table, host API

**Render-time**:
Of an error: raised only when execution actually reaches the offending content. Expressions in a branch not taken, or a `for` body that never iterates, are not evaluated and do not error.
_Avoid_: runtime

**Parse-time**:
Of an error: raised during tokenization or parsing, before any content executes — malformed syntax, empty identifier after `.`, empty interpolation or statement, malformed statement head, integer literal overflowing i64.
_Avoid_: compile-time

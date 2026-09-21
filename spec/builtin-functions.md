# Builtin functions

The *builtin functions* are the fixed set of named functions dotrift provides
to the templater's host-provided *Function registry* (ADR-0017): the same set
fills the registry for `dotrift.toml` rendering and for deployed templates,
because both consume the same render pipeline. The set lives host-side in
dotrift; the templater spec keeps the registry an unopinionated,
host-provided table.

A name not in this set is a template error when called. The process
environment is reachable only through the environment functions below;
nothing is injected into the *variable context* implicitly (see
`spec/dotrift-toml.md § Rendering before parsing`).

Function names, argument counts, and argument types are checked at render
time as the templater's function-call rules dictate; the per-function rules
below are checked in order at call time: argument count first, then argument
types left to right, then the function's own condition. Errors name the
offending arguments where stated.

## Truthiness

Several functions test a value's truthiness. A value is truthy when it is:

- a non-empty String
- a nonzero Int
- the Bool `true`
- a non-empty List
- a non-empty Map

Empty strings, `0`, `false`, and empty List/Map values are falsy.

## Environment

**`env(name, fallback)` → String.** The value of the process environment
variable `name`, or `fallback` when the variable is unset or does not decode
as valid Unicode. `name` and `fallback` must be Strings; `name` may contain
any characters except `=` and NUL (the OS rejects those).

**`home()` → String.** `$HOME` when set and non-empty; otherwise the user's
home directory as resolved by the platform; otherwise the empty string.

**`os()` → String.** The compiled target operating system (`linux`,
`macos`, `windows`, …).

**`arch()` → String.** The compiled target CPU architecture (`x86_64`,
`aarch64`, …).

## String

All String arguments must be Strings; an Int or other type is an argument
error.

**`upper(s)` → String.** `s` uppercased.

**`lower(s)` → String.** `s` lowercased.

**`trim(s)` → String.** `s` with surrounding whitespace removed.

**`replace(s, from, to)` → String.** Every occurrence of `from` in `s`
replaced with `to`. An empty `from` matches between every character.

**`split(s, sep)` → List of String.** `s` split on every occurrence of
`sep`; empty parts are kept. An empty `sep` splits between every character.

**`join(sep, part...)` → String.** The String parts joined with `sep`
between neighbours. Takes at least two arguments (a separator and one part).

**`starts_with(s, prefix)` → Bool.** Whether `s` begins with `prefix`.

**`ends_with(s, suffix)` → Bool.** Whether `s` ends with `suffix`.

## Comparison

**`eq(a, b)` → Bool** / **`ne(a, b)` → Bool.** Equality across any value
types, but never across types: a String never equals an Int, even when the
text matches.

**`gt(a, b)` / `gte(a, b)` / `lt(a, b)` / `lte(a, b)` → Bool.** Ordering on
two Ints. String arguments are argument errors.

## Math

All operands must be Ints.

**`add(a, b, ...)` → Int.** The sum of two or more Ints.

**`sub(a, b)` → Int.** `a - b`.

**`mul(a, b, ...)` → Int.** The product of two or more Ints.

**`div(a, b)` → Int.** `a / b`, truncating toward zero. Division by zero is
an error, not a crash or a sentinel value.

**`neg(a)` → Int.** `-a`.

## Logic

**`and(a, b, ...)` → Bool.** True when every argument is a truthy Bool;
takes two or more Bools.

**`or(a, b, ...)` → Bool.** True when any argument is a truthy Bool; takes
two or more Bools.

**`not(a)` → Bool.** The negation of a Bool.

**`is_truthy(v)` → Bool.** Truthiness of any value (§ Truthiness).

**`coalesce(v...)` → any.** The first truthy argument; the last argument
when none is truthy. Takes at least one argument.

## Conversion

**`to_str(v)` → String.** The canonical text form of any value: Strings
verbatim, Ints in decimal, Bools as `true`/`false`, Lists as
`[item, item]`, Maps as `{key: value, key: value}` (keys sorted), nested
recursively.

**`to_int(v)` → Int.** An Int unchanged; the Bool `true` as `1` and `false`
as `0`; a String parsed as a decimal integer. A List or Map is an error; an
unparseable String is an error naming the value.

## Collection

**`length(v)` → Int.** The byte length of a String, or the item count of a
List, or the entry count of a Map. An Int or Bool receiver is an error.

**`contains(haystack, needle)` → Bool.** For a String haystack, whether it
contains the String needle; for a List, whether it contains an item equal to
needle; for a Map, whether it contains the String key needle. An Int or Bool
haystack is an error.

**`first(list)` → any.** The first item; an empty List is an error.

**`last(list)` → any.** The last item; an empty List is an error.

**`keys(map)` → List of String.** The keys in sorted order (BTreeMap
order).

**`values(map)` → List.** The values in their keys' sorted order.

**`enumerate(list)` → List of Map.** Each item paired with its zero-based
position as a one-entry-per-item Map `{index: Int, value: item}`.

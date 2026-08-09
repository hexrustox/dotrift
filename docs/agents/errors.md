# Error message conventions

How every dotrift-authored error message is phrased and rendered, across the
whole workspace: the root binary, `templater`, and `tui` errors surfaced to the
user. One voice, no exceptions. Specs stay authoritative for *what* errors
exist and when they fire; this document governs *how* their messages read.

## Rules

1. **Lowercase sentence fragments, no trailing period.**
   Each message is one sentence fragment starting lowercase, ending without a
   period.

   - good: `` undefined variable `{name}` ``
   - bad: `Variable is not defined.`

2. **Backtick-quote syntax elements, paths, and named literals.**
   Quote keywords, delimiters, operators, identifiers, keys, filenames,
   profile names, and deploy kinds — including `{field}` placeholders when
   they denote a token. Do not backtick raw data (numbers, counts).

   - good: `` undefined variable `{name}` ``
   - good: `` `elif` outside of an `if` block `` (tokens being discussed)
   - bad: `` list index `{idx}` out of bounds `` — `{idx}` is data, not a token

3. **One sentence per message.**
   Name the subject the error is about and the facts needed to act on it, in a
   single sentence. When more detail is needed, chain it with `wrap_err()` —
   never a colon-separated clause.

   - good: `` cannot read `dotrift.toml` `` wrapped by a `wrap_err`
     supplying the operation context
   - bad: `cannot resolve source directory: both XDG_DATA_HOME and HOME are
     unset`

4. **Message states what is wrong; advice lives in `label`/`help`.**
   The message describes the error, not the fix. Prescriptive advice goes in
   the miette `label` (span-anchored, "fix this") or `help` (no location, used
   by CLI-level errors), both optional, never duplicated into the message.

   - message: `` undefined variable `{name}` ``
   - label: `define this variable or fix the name`

5. **`label` for spans, `help` for everything else.**
   Use `label` when there is a real source location to underline (templater
   parse/render errors, config positions). Use `help` when there is no span
   (CLI-level errors) or for general guidance.

6. **Rendering.**
   Errors print to stderr. The fancy miette report is rendered via the
   `Result` main return; color and unicode decorations are disabled when
   stderr is not a terminal.

7. **Internal errors.**
   The message still states only what is wrong. If the error may be an
   internal bug rather than a user mistake, say so in `help`.

   - help: `this is likely an internal error`

8. **clap usage errors are exempt.**
   `clap` generates its own messages for parse failures (unknown flags,
   missing required args, bad values). Leave its rendering untouched; this
   document governs dotrift-authored messages only.

## Good vs bad

| good | bad |
| --- | --- |
| `` cannot read `dotrift.toml` `` | `Failed to read the config file.` |
| `` undefined variable `{name}` `` | `Variable is not defined.` |
| `` both XDG_DATA_HOME and HOME are unset `` + `wrap_err("cannot resolve source directory")` | `cannot resolve source directory: both XDG_DATA_HOME and HOME are unset` |
| message: `list index {idx} out of bounds` | `error: you need to use a valid index` (message prescribes, no facts) |

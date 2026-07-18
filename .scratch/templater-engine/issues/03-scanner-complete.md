# 03 — Comments, escape rule, whitespace modifiers (scanner-complete)

**What to build:** The scanner fully enforces `templater/spec/syntax.md` regarding delimiters, escapes, modifiers, and comments — independently of the parser/evaluator, which mostly ignore the new token kinds in this slice. All six delimiters are recognized with the odd-backslash escape rule applied inline during scan (all five table rows: `{{`, `\{{`, `\\{{`, `\\\{{`, `\\\\{{`). `{# #}` comments are stripped at scan time (no `Token::Comment` variant emitted) but act as `=` barriers for whitespace control. `{{-`/`-}}`/`{%-`/`-%}` trim adjacent spaces and tabs on their side (no `\n`, no cross-tag). `{{=`/`=}}`/`{%=`/`=%}` scan over **raw bytes** (not the token stream) to `\n`/SOF/EOF or any of the six delimiters as barriers, with rightward `=` eating through and including the terminating `\n`, leftward `=` stopping before the `\n`, and `\r` treated as plain text. Multi-line tags span physical lines; inside interp/stmt tag bodies the scanner enters string-literal mode at `"` so `}}`/`%}` inside a string literal does not close the tag. Inside comments, no string-literal mode — bytes scan raw until unescaped `#}`. Token body ranges exclude inner-edge whitespace (already trimmed by the scanner).

**Blocked by:** 01

**Status:** ready-for-agent

- [ ] Escape rule (odd-backslash ⇒ literal delimiter; even ⇒ `n/2` backslashes + active delimiter) implemented inline; all five table rows behave per spec.
- [ ] All six delimiters recognized with single-byte lookahead for `{` vs `{{`/`{%`/`{#`.
- [ ] `Token::Stmt(body_range)` emitted for `{% %}` tags (parser may stub these as "unrecognized statement" until ticket 06).
- [ ] Comments stripped at scan time; no `Token::Comment` variant in the enum.
- [ ] `-` modifier trims adjacent spaces and tabs on its side; no `\n`, no cross-tag bleed.
- [ ] `=` modifier scans over raw bytes for `\n`/SOF/EOF/any of six delimiters; rightward eats through and includes `\n`; leftward stops before `\n`; `\r` is plain text.
- [ ] All six delimiters act as `=` barriers (comment, interp, stmt alike).
- [ ] Multi-line tags span physical lines; raw newlines inside a string literal inside a tag body are preserved as part of the literal's value.
- [ ] Inside interp/stmt tag bodies, `"..."` enters string-literal mode so inner `}}`/`%}` don't close the tag; inside comments, raw byte scan until unescaped `#}`.
- [ ] Parse-time errors raised by the scanner: stray closing delimiter, `=` and `-` on the same side of a tag, any modifier on a comment delimiter (`{#=`, `=#}`, etc.), escaped opening delimiter with unescaped closing (e.g. `\{{}}` per spec table).
- [ ] Colocated `#[cfg(test)] mod tests` in `src/scanner.rs` pins the escape rule table, `=`/`-` modifier combinations, comment stripping, string-shielding, multi-line tags, `\r` handling.
- [ ] End-to-end tests in `templater/tests/scanner.rs` exercise escape-rule outputs through the public render API.
- [ ] `cargo test -p templater`, `cargo fmt`, `cargo clippy -p templater` pass.
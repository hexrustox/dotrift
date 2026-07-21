# Templater Agent Guide

The standalone template engine (scanner/parser/eval). Consult
[`spec/syntax.md`](spec/syntax.md) and [`spec/CONTEXT.md`](spec/CONTEXT.md)
before changing CLI, config, DB schema, templater syntax, or pager behavior.

## Testing: unit vs integration

The `templater` crate has both colocated unit tests (`#[cfg(test)]`)
and integration tests (`tests/*.rs`). Use this decision order:

1. **Unit test** — when the behavior is an internal data transformation with a
   clear boundary (scanner tokens, parser AST, string-literal decoding, span
   windows, scope lookup). Assert exact internal shapes that would be awkward
   through the public API.

2. **Integration test** — when the behavior is user-facing per
   [`spec/syntax.md`](spec/syntax.md) or [`spec/CONTEXT.md`](spec/CONTEXT.md).
   Assert rendered output bytes, error variants, or error spans through the
   public `Template::from_bytes` / `Template::render` API. Also use when the
   scenario requires a `FunctionRegistry`, a variable scope, or a custom writer.

3. **Both** — only when each level proves something the other cannot. A unit
   test pinning an exact internal shape plus an integration test confirming
   that shape survives the full pipeline is valid. Duplicating the same
   assertion at both levels is not.

4. **Regression tests** go at the lowest level that reproduces the bug: a
   scanner miscalculation gets a scanner unit test; a bug that only appears in
   the full scan→parse→render pipeline gets an integration test.

5. **Property-based tests** (`proptest`) are for cross-cutting invariants that
   are hard to enumerate but easy to specify — e.g. "any generated template
   renders predictably." Avoid them as a substitute for readable
   unit/integration examples of spec features.

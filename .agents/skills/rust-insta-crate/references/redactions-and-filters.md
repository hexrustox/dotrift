# Redactions and filters

For values whose output churns with volatility — timestamps, uuids, paths, hashes, nondeterministic ordering, floats — neutralize the volatile parts so the snapshot stores only what the test means to pin. Enable the `redactions` and `filters` features to use these.

Two tools, chosen by the value's form:

- **Redactions** — for serde-serialized snapshots (`assert_json_snapshot!`, `assert_yaml_snapshot!`, ...). Replace values at a path with a placeholder before comparing.
- **Filters** — for string snapshots (`assert_snapshot!`). Regex-replace spans of the snapshot text.

## Redactions

Passed as the argument after the value (a map of selector → replacement):

```rust
insta::assert_json_snapshot!(&user, {
    ".id" => "[uuid]",
    ".extra.ssn" => "[ssn]",
});
```

### Selectors

- `.key` — the given key.
- `.$key` — indexes the key of a collection (compound keys).
- `["key"]` — alternative key syntax.
- `[index]` — index into an array; `[]` all array items; `[:end]` / `[start:]` / `[start:end]` slices (negative indexing supported).
- `.*` — all keys at that depth.
- `.**` — deep match (zero or more items); once per selector.

### Replacement kinds

- **Static** — hardcoded placeholder: `".id" => "[uuid]"`.
- **`dynamic_redaction`** — callback over the value that can assert the value's *format* before redacting (e.g. "looks like a uuid"):

  ```rust
  insta::assert_json_snapshot!(&user, {
      ".id" => insta::dynamic_redaction(|value, _path| {
          assert_eq!(value.as_str().unwrap().chars().filter(|&c| c == '-').count(), 4);
          "[uuid]"
      }),
  });
  ```

- **`sorted_redaction`** — sort a map or sequence at a selector; use for `HashSet`/`HashMap` whose serialization order is nondeterministic: `".flags" => insta::sorted_redaction()`.
- **`rounded_redaction`** — round floats at a selector to a precision: `".*" => insta::rounded_redaction(3)`.

## Filters

Regex applied to the snapshot *text* before comparing — for `assert_snapshot!` (and other macros) when the value is inherently a string that redactions cannot address. Set via `with_settings!`:

```rust
insta::with_settings!({filters => vec![
    (r"\b[[:xdigit:]]{32}\b", "[UID]"),
]}, {
    insta::assert_snapshot!(rendered);
});
```

A filter is a `(regex, replacement)` tuple. Use them to normalize paths, absolute line numbers, machine IDs, and the like. For structured data prefer redactions — they are self-documenting at the assertion site; filters hide inside `with_settings!`.

## Settings and scope

`with_settings!` scopes redactions, filters, and other settings to one assertion:

```rust
insta::with_settings!({
    info => &ctx,
    description => source,
    omit_expression => true,
}, {
    insta::assert_snapshot!(template.render(ctx));
});
```

`info` attaches a serializable value to the snapshot's review screen; `description` attaches explanatory text; `omit_expression` drops the default stringified expression. Redactions and filters are the common use; see the settings docs for the rest.

## Reference

- https://insta.rs/docs/redactions/
- https://insta.rs/docs/filters/
- https://insta.rs/docs/settings/

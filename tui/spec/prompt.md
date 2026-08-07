# Prompt Specification

A reusable generic prompt that presents the variants of an enum as a vertical
list of radio buttons. Exactly one option is selected at all times, and the
user confirms a choice with `Enter`. The selected enum variant is returned to
the caller.

The prompt is implemented with **crossterm only** — ratatui is not used for
rendering.

---

## Enum Contract

The prompt operates over an enumerable, unit-like enum:

- Every variant must be enumerable (e.g. `strum::IntoEnumIterator`); variants
  are presented in iterator order.
- Variant names must be recoverable as strings (e.g. via `Debug`) for automatic
  label derivation.
- Payload-bearing enums are not supported: the prompt cannot construct a
  canonical value for such variants, so construction is rejected.
- An enum with zero variants is rejected during construction, before any
  terminal interaction.

## Automatic Option Derivation

Each variant becomes one option. Labels and hotkeys are derived automatically
unless overridden.

- **Label:** derived from the PascalCase variant name by inserting a space
  before each word boundary and lowercasing:
  - `OverwriteIdentical` → `overwrite identical`
  - A run of consecutive capitals is treated as a single word:
    - `HTTPServer` → `http server`
- **Hotkey:** the first ASCII alphabetic character of the derived label,
  matched case-insensitively.
  - `overwrite` → `o`

## Overrides

Callers may override labels and hotkeys per variant through an optional trait
on the enum:

```rust
trait PromptOption {
    fn label(&self) -> Option<&str> { None }
    fn hotkey(&self) -> Option<char> { None }
}
```

- `label` and `hotkey` override independently: overriding a label does not
  require overriding a hotkey, and vice versa.
- Effective hotkeys are validated before interaction:
  - Only ASCII `A-Z` is accepted.
  - Hotkeys are compared case-insensitively.
  - Duplicate effective hotkeys are rejected with an error.

## Defaults

- The prompt accepts an optional `.default(value)` configuration; the value is
  a variant of the enum, so the compiler rejects invalid values.
- When no default is supplied, the **first variant** is selected initially.
- The prompt accepts a `.question("...")` configuration used as the title
  rendered above the option list.
- In **non-TTY** mode (stdin is not a terminal), the prompt returns the
  configured or first default immediately, without rendering or entering raw
  mode.

## Styling

The prompt accepts an optional `.style(PromptStyle)` configuration. `PromptStyle`
is a plain struct with one `Color` per rendered element:

| Element                      | Field                | Default      |
|------------------------------|----------------------|--------------|
| Question title               | `question`           | `Reset`      |
| Selected option row          | `selected`           | `Green`      |
| Selected option's marker     | `marker_selected`    | `Green`      |
| Unselected options' markers  | `marker_unselected`  | `Reset`      |
| Confirmation line (on Enter) | `confirm`            | `Green`      |
| Help line                    | `help`               | `Reset`      |

By default the selected option row and the confirmation line are rendered in
green. Callers override individual elements with struct update syntax:

```rust
SelectPrompt::new()
    .style(PromptStyle { selected: Color::Blue, ..Default::default() })
```

Any future stylable element follows this pattern: a sensible default in
`PromptStyle::default()`, overridable by the caller.

## Rendering

The prompt renders inline in the current terminal — it does not use an
alternate screen.

- The question is rendered as a title line, followed by a vertical list of
  options, one per line, each prefixed with its hotkey:

  ```
  answer to question:
    ○ [s] skip
    ● [o] overwrite
    ○ [d] diff
    ○ [q] quit

    ↑/↓ navigate  Enter select  A-Z jump  Esc cancel
  ```

- **Markers** depend on locale UTF-8 support:
  - UTF-8: `●` selected, `○` unselected.
  - ASCII fallback: `*` selected, a blank space unselected.
- The selected row is additionally highlighted (e.g. bold or reverse video) so
  the marker is not the only selection indicator.
- A concise help line lists the available keys.
- Long labels are truncated to the available width, never wrapped.

### Confirmation

On `Enter`, the interactive list is cleared and a single result line is
printed:

```
answer to question: (skip) ✓
```

- Format: `<question>: (<selected label>) <marker>`
- UTF-8 marker: `✓`.
- ASCII fallback marker: `done` (e.g. `answer to question: (skip) done`).
- On cancellation (`Esc`/`Ctrl+C`) the list is cleared and nothing is printed.

## Keyboard Controls

| Key      | Action                                    |
|----------|-------------------------------------------|
| Up / Left | Move to the previous option              |
| Down / Right | Move to the next option              |
| A-Z      | Jump to the option with that hotkey       |
| Enter    | Confirm the selected option and return it |
| Esc      | Cancel the prompt                         |
| Ctrl+C   | Cancel the prompt                         |

- Navigation wraps at both ends: next from the last option returns to the
  first, and previous from the first option returns to the last.
- Hotkey presses **move the selection only**; they do not confirm. `Enter` is
  the sole confirmation key.
- **Modifier matching is strict.**
  - Movement, hotkey jumps, `Enter`, and `Esc` respond only to unmodified key
    presses (`KeyModifiers::NONE`); any Shift/Alt/Ctrl combination is ignored.
  - `Ctrl+C` cancels only for the exact key `Char('c')` carrying exactly the
    `CONTROL` modifier. No other key or modifier combination cancels the
    prompt.

## Terminal Lifecycle

- UTF-8 support is detected from the `LC_ALL`, `LC_CTYPE`, and `LANG`
  environment variables, as in the pager.
- Raw mode is enabled only after all validation succeeds.
- On `SIGWINCH`/resize, the prompt is cleared and redrawn at the new
  dimensions, preserving the selected option and scroll position.
- On every exit path — confirmation, cancellation, or error — the prompt
  restores raw mode, cursor visibility, and terminal contents.

## Errors

The prompt returns a distinct error for each failure mode:

- `EmptyOptions` — the enum has no variants.
- `InvalidHotkey` — an effective hotkey is not ASCII `A-Z`.
- `DuplicateHotkey` — two options share the same effective hotkey.
- `Cancelled` — the user pressed `Esc` or `Ctrl+C`.
- `Io` — a terminal or I/O failure.

## Overflow and Edge Cases

- When the option list is taller than the terminal, the list scrolls so the
  selected option remains visible; the title and help line stay fixed where
  possible.
- Hotkey input is accepted in either case (`s` or `S`).
- Option ordering and selection are deterministic.
- Exactly one option is selected at every point during interaction.

## Verification Requirements

- Test PascalCase label conversion, including acronym runs.
- Test override behavior and effective-hotkey validation.
- Test default selection and non-TTY behavior.
- Test navigation, wrapping, hotkey jumps, confirmation, and cancellation
  through a pure state-machine boundary with synthetic key events.
- Test rendering markers under Unicode and ASCII locales.

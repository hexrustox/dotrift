# Dotrift Agent Guide

## Project Overview
Dotrift is a Rust-based dotfile manager that maps files from a source directory to a target directory using declarative TOML configuration.

## Key Files
- `spec/command_spec.md`: CLI command behavior (apply, unapply, add, diff, status)
- `spec/config_spec.md`: dotrift.toml format (portal mapping, rules, ignore patterns)
- `Cargo.toml`: Rust package manifest
- `src/main.rs`: Application entrypoint

## Development Workflow
### Standard Rust Commands
- Build: `cargo build`
- Run: `cargo run -- [args]`
- Test: `cargo test`
- Check: `cargo check`
- Format: `cargo fmt`
- Clippy: `cargo clippy`

Here is a proposed section for your `AGENTS.md` file. It covers capitalization, punctuation, and a strict hierarchy for quotation marks to ensure log messages are parseable and easy to read.

## Logging Standards

Consistent logging is critical for debugging and monitoring. All log messages (including `info`, `warn`, `error`, and `debug`) must adhere to the following formatting rules to ensure readability and machine parseability where applicable.

### 1. General Formatting
*   **Capitalization:** Start every log message with an uppercase letter.
*   **Punctuation:** End every log message with a period (`.`).
*   **Tone:** Use plain language. Avoid emotional wording (e.g., do not use "Catastrophic failure," use "Failed to initialize connection").

### 2. Quotation Mark Guidelines
Strict adherence to quotation types prevents ambiguity between code variables, file paths, and user-generated content.

| Quote Type | Symbol | Usage Category | Examples |
| :--- | :--- | :--- | :--- |
| **Backticks** | `` ` `` | **Technical Identifiers** Use for variable names, function names, file paths, URLs, database keys, and system IDs. | File not found: /etc/config.json. Variable \`x\` is undefined. |
| **Double Quotes** | `"` | **String Values & User Input** Use for actual string values, API responses, or user-provided data. | User provided invalid email: "john.doe@". Received payload: "success". |
| **Single Quotes** | `'` | **Reserved / Avoid** Do not use single quotes in log messages unless the data itself contains a single quote. This avoids confusion with JSON string encapsulation. |  |

### 3. What to Quote (and what not to quote)
Not everything needs to be wrapped in quotes. Over-quoting creates visual noise.

*   **DO quote:**
    *   Specific values that help identify the error (e.g., a specific ID, a file path).
    *   Dynamic data injected into the string.
*   **DO NOT quote:**
    *   Generic object names mentioned in the message.
    *   Numbers or booleans (unless they are part of a string payload).

### 4. Examples

**Bad Logging:**
```text
// Missing capitalization, no period, ambiguous quotes
error: failed to load user 12345
// Over-quoting, emotional language
ERROR: Disaster! The config file "settings.yaml" is missing!
// Single quotes used for technical identifiers
Warning: function 'calculateTotal' returned null.
```

**Good Logging:**
```text
// Capitalized, period, ID wrapped in backticks
Error: Failed to load user with ID `12345`.

// Objective tone, file path in backticks
Error: Configuration file `settings.yaml` not found in directory `/etc/config`.

// Function name in backticks, generic object "Result" unquoted
Warning: Function `calculateTotal` returned an unexpected result.
```

### 5. Structured Data (Key-Value Pairs)
When appending structured data to logs (such as `err` or `id`), do not embed the variable in the message string if the logging framework supports key-value pairs. This allows for better indexing in log management systems (e.g., Datadog, Splunk).

**Preferred (Pseudo-code):**
```text
logger.Error("Failed to process request.", "request_id", requestID, "error", err)
```

**Output:**
```text
Error: Failed to process request. request_id=`abc-123` error="connection timeout"
```

# Context Map

## Contexts

- [Dotrift](./spec/CONTEXT.md) — declarative, template-aware dotfile manager; maps files from source to target via `dotrift.toml`
- [Templater](./templater/spec/CONTEXT.md) — standalone template engine: text with embedded tags rendered into output via expressions, statements, and plain text

## Relationships

- **Dotrift → Templater**: Dotrift invokes Templater at apply time for entries whose deploy type is `tmpl`, passing template content and variables from `dotrift_data.toml`

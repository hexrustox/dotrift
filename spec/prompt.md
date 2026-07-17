# Collision Prompt

When dotrift encounters an *obstruction* (term defined in `CONTEXT.md`) —
something already exists on disk at a target path dotrift wishes to write to
(file, directory, dangling symlink, etc.) — the user is prompted with:

```
[s]kip / [o]verwrite / [d]iff / [q]uit
```

Unless otherwise noted:

* **skip:** Skip the operation, continue traversal.
* **overwrite:** Remove the obstruction and proceed.
* **diff:** Open the diff pager (see `spec/pager.md`).
* **quit:** Halt the program.

---

## Non-TTY Behavior

If stdin is not a terminal, the prompt defaults to `skip` — no interactive
choice is offered and the operation is skipped.

---

## Per-Obstruction Actions

The exact filesystem actions attached to each option (e.g. "delete the
directory recursively", "delete DB entries under the directory") depend on
the operation in progress and are specified with that operation. Today the
prompt is used only by `apply` (Phase 3, Directory Nodes and File Nodes);
see `spec/commands/apply.md`.
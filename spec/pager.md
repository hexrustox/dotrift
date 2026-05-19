# Pager TUI Specification

The pager is a terminal UI invoked by dotrift to display file contents and browse directories.

## Layout Modes

### Side-by-Side Mode

Two panes split evenly across the terminal, displaying two files side by side.

- Left pane: content of file A.
- Right pane: content of file B.
- Scroll is locked — both panes always scroll together.

### Explorer Mode

Two panes. Left pane shows a static file; right pane is an interactive file browser.

- Left pane: content of a file (scrollable independently).
- Right pane: file browser rooted at a given directory.
  - Lists directory entries (files, subdirectories, symlinks).
  - Enter on a directory: descend into it.
  - Enter on a file: display its content in-place (fills the right pane).
  - Esc while viewing a file: return to the directory listing.
  - Esc at the root directory: no-op (stays in explorer).
- **Tab**: switch focus between left and right panes. The focused pane responds to scroll/navigation keys.

### Single-Pane Mode

Full-terminal display of a single file.

- A header line is displayed at the top, supplied by the caller.
- The file content fills the remainder of the terminal.

## Header

Each pane has a single header line at the top, visually distinct from content (inverted colors).

- **File pane:** Displays the absolute file path.
- **File explorer pane:** Displays the current working directory (absolute path). Updates as the user navigates subdirectories.
- **Single-pane mode:** The caller-supplied header is displayed. If not supplied, the file path is displayed.
- **Symlink:** The header appends ` → /link/target` to the file path (e.g., `/home/user/cfg → /etc/cfg`). The content area shows the resolved target's content (symlink is followed for display only; the filesystem is not modified).

## Rendering

- Plain text. No syntax highlighting.
- No line numbers.
- Lines longer than the available pane width are truncated (no wrapping).
- Empty files display as an empty line.

## Keybindings

| Key | Action |
|-----|--------|
| Arrow Up / k | Scroll up / move selection up |
| Arrow Down / j | Scroll down / move selection down |
| Page Up / Ctrl+B | Page up |
| Page Down / Ctrl+F | Page down |
| Home / g | Jump to top |
| End / G | Jump to bottom |
| Tab | (Explorer mode) Switch focus between panes |
| Enter | (Explorer mode) Enter directory / open file |
| Esc | (Explorer mode) Go back (file view → listing, subdir → parent) |
| q | Quit pager |
| Ctrl+C | Quit pager |

### Context-Specific Behavior

- In side-by-side and single-pane modes, j/k scroll the content.
- In explorer mode with focus on the right pane, j/k move the selection cursor.
- In explorer mode with focus on the left pane, j/k scroll the source file content.

## Edge Cases

- **Terminal resize:** Re-render layout, preserving scroll positions as closely as possible.
- **Long lines:** Truncated to pane width. No wrapping.
- **File error:** If a file cannot be read, display an error message in the affected pane (e.g. `Error: <message>`).
- **Very large files:** Load and display content; performance is implementation-defined.

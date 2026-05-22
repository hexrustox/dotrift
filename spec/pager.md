# Pager TUI Specification

The pager is a terminal UI invoked by dotrift via the `[d]iff` collision prompt
option. It has three modes selected automatically based on the paths:

- **View**: one path → single file viewer
- **Diff**: two files → side-by-side line diff
- **Explorer**: file + directory → browse directory, preview file

All modes share a 3-row layout: header, content, footer.

---

## View Mode

Single file viewer. Used when a file on disk blocks directory creation.

**Header:** `File <path> blocks directory creation`
**Content:** Full-terminal file display with line-by-line scrolling.
**Footer:** `{pos}/{max}` — 1-indexed scroll position over max reachable position.
Hidden if all lines fit in the viewport.

## Diff Mode

Two panes split evenly by a vertical separator, showing a computed line diff.

**Header:** `Replace <old> with <new>`
**Content:**
- Old file on left, new file on right.
- Lines prefixed with `-` (red, old-only), `+` (green, new-only), or ` ` (unchanged).
- Consecutive delete+insert pairs are collapsed into a single Change row
  (old line on left, new line on right).
- Scroll is locked — both panes always scroll together.
**Footer:** `{pos}/{max} +{added} −{removed}` — scroll position + change counts.
Hidden if all diff pairs fit in the viewport.

## Explorer Mode

Two panes split evenly by a vertical separator. Browser on the left, file preview
on the right. Used when a directory on disk blocks file creation.

**Header:** `Directory <dir> blocks file creation`
**Content:**
- **Left pane (browser):** Directory listing with `..` entry for parent.
  Entries sorted: directories first, files second, alphabetically within groups.
  - Directories shown as `name/`
  - Symlinks shown as `name -> /target`
  - Selected entry marked with `> ` prefix. Cursor only visible when browser
    has focus.
  - `Enter`: descend into directory, or open file for preview.
  - Directory preview opens a file viewer in-place, replacing the listing.
  - `Esc` from file preview: return to directory listing.
  - `Esc` from listing: go to parent directory (no-op at root).
- **Right pane (preview):** Source file content (scrollable independently).
- **Tab**: toggle focus between browser and preview panes.
  The focused pane responds to scroll keys.
**Footer:** `{Focus}  {pos}/{max}` — focus label + position:
- Browser (Dir state): cursor index / entry count
- Browser (File state): scroll position / max of opened file
- Preview: scroll position / max of source file

## Rendering

- Plain text. No syntax highlighting. No line numbers.
- Lines longer than the available column width are truncated (no wrapping).
- Empty files display as an empty line.
- Files are read via line-offset index — only visible lines are read per frame.
  Full file content is not held in memory.

## Headers and Footers

- One full-width header line per mode.
- One full-width footer line per mode.
- Footer row collapses to zero height when content fits entirely in the viewport
  and no status information would be displayed.

## Keybindings

| Key | Action |
|-----|--------|
| Arrow Up / k | Scroll up / move cursor up |
| Arrow Down / j | Scroll down / move cursor down |
| Page Up / Ctrl+B | Page up |
| Page Down / Ctrl+F | Page down |
| Home / g | Jump to top |
| End / G | Jump to bottom |
| q | Quit pager |
| Ctrl+C | Quit pager |
| Tab | (Explorer) Switch focus between panes |
| Enter | (Explorer) Enter directory / open file |
| Esc | (Explorer) Go back (file view → listing, subdir → parent) |

All modes respond to scroll keys (j/k, arrows, PgUp/PgDn, Home/End).
Tab, Enter, Esc dispatch to the mode via trait methods and are no-ops in View
and Diff modes. 

## Edge Cases

- **Terminal resize:** Re-render layout, preserving scroll positions as closely
  as possible.
- **Long lines:** Truncated to column width. No wrapping.
- **File error:** `FileViewer` propagates I/O error up to `run()`, which returns
  `Err`.
- **Large files:** Line-offset index built once (~8 bytes per line). Only visible
  lines read per frame via seek + read. No whole-file memory.
- **Non-TTY:** `run()` returns `Ok(())` immediately if stdin is not a terminal.
  The `diff` option is not offered in the collision prompt when no TTY.

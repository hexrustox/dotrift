# Pager TUI Specification

The pager is a terminal UI invoked by dotrift via the `[d]iff` collision prompt
option. It has three modes selected automatically based on the paths:

- **View**: one path → single file viewer
- **Diff**: two files → side-by-side line diff
- **Explorer**: file + directory → browse directory, preview file

All modes share a 2-row layout: content area and a footer bar.

---

## View Mode

Single file viewer. Used when a file on disk blocks directory creation.

**Content:** Full-terminal file display with line-by-line scrolling.
**Footer:** A reversed-style bar. When the file has more lines
than the viewport, a scroll indicator `(pos/max)` is shown left-aligned
(1-indexed position over max reachable position). Otherwise only the path is shown.

## Diff Mode

Two panes split evenly by a vertical separator, showing a computed line diff.

**Content:**
- Old file on left, new file on right.
- Lines prefixed with `-` (red, old-only), `+` (green, new-only), or ` ` (unchanged).
- Replace regions are decomposed into per-line pairs: lines present on both sides
  are marked as Change (`-`/`+`), and extra lines on either side are marked as
  standalone Delete or Insert.
- Scroll is locked — both panes always scroll together.
**Footer:** A reversed-style bar with `(pos/max) (+N −M)` left-aligned as the
status, showing scroll position and total add/remove counts across the entire
diff. The footer bar is always visible.

## Explorer Mode

Two panes split evenly by a vertical separator. Browser on the left, file preview
on the right. Used when a directory on disk blocks file creation.

**Content:**
- **CWD line:** A single line above the listing showing the current directory path.
- **Left pane (browser):** Directory listing.
  Entries sorted: directories first, files second, alphabetically within groups.
  - Directories shown as `name/`.
  - Symlinks shown as `name → /target`. Broken symlinks receive a distinct color.
  - Entry colors follow `LS_COLORS` conventions using mode bits and file extensions.
  - Selected entry marked with a cursor prefix (e.g. `> `). Cursor only visible
    when browser has focus.
  - `Enter`: descend into directory (including symlinks to directories), or open
    file for in-place preview.
  - `Esc` from file preview: return to directory listing.
  - `Esc` from listing: go to parent directory (no-op at root).
- **Error state:** When entering a directory or opening a file fails (e.g.
  permission denied), the browser pane displays the error message and the footer
  shows `Error`. Press `Esc` to return to the previous directory.
- **Right pane (preview):** Source file content (scrollable independently).
- **Tab**: toggle focus between browser and preview panes. The focused pane
  responds to scroll keys.
**Footer:** A reversed-style bar (see Footer Bar section). Status appears left-aligned:
- Browser (directory listing): `Browser (N/M)` — 1-indexed cursor over entry count.
- Browser (file preview): `Browser (pos/max)` — scroll position.
- Browser (error): `Browser` — error message displayed in pane.
- Preview: `Preview (pos/max)` — scroll position.

## Rendering

- Plain text. No syntax highlighting. No line numbers.
- Lines longer than the available column width are truncated (no wrapping).
- Empty files display as an empty line.
- Files are read via line-offset index — only visible lines are read per frame.
  Full file content is not held in memory.
- The vertical separator is `│` under UTF-8 locales and `|` otherwise. The
  symlink arrow is `→` under UTF-8 locales and `->` otherwise. The cursor
  prefix is `▶ ` under UTF-8 locales and `> ` otherwise. Locale is detected
  from the `LC_ALL`, `LC_CTYPE`, and `LANG` environment variables.

## Footer Bar

- One full-width reversed-style row at the bottom of the screen.
- Three-section layout: status (left), path (center), help hint (right).
- **Left:** Mode-specific status. In View mode, when content fits entirely in the
  viewport, the scroll status portion is omitted.
- **Center:** Truly centered path (file path in View/Diff modes, current directory
  path in Explorer mode).
- **Right:** `h help` hint.
- When the bar is too narrow for all three sections, center is dropped and the
  layout falls back to left-aligned status + right-aligned help.

## Keybindings

| Key | Action |
|-----|--------|
| Arrow Up / k | Scroll up / move cursor up |
| Arrow Down / j | Scroll down / move cursor down |
| Page Up / Ctrl+B | Page up |
| Page Down / Ctrl+F | Page down |
| Ctrl+D | Scroll half page down |
| Ctrl+U | Scroll half page up |
| Home / g | Jump to top |
| End / G | Jump to bottom |
| q | Quit pager |
| Ctrl+C | Quit pager |
| h | Toggle help popup |
| Tab | (Explorer) Switch focus between panes |
| Enter | (Explorer) Enter directory / open file |
| Esc | (Explorer) Go back (file view → listing, subdir → parent) |

All modes respond to scroll keys (j/k, arrows, PgUp/PgDn, Home/End).
Tab, Enter, Esc are no-ops in View and Diff modes.

## Edge Cases

- **Terminal resize:** Re-render layout, preserving scroll positions as closely
  as possible.
- **Long lines:** Truncated to column width. No wrapping.
- **File error:** If a file cannot be read, the pager exits with an error.
- **Large files:** Line-offset index built once (~8 bytes per line). Only visible
  lines read per frame via seek + read. No whole-file memory.
- **Non-TTY:** The pager returns immediately if stdin is not a terminal.
  The `[d]iff` option is not offered in the collision prompt when no TTY.
- **Unicode:** If the locale does not declare UTF-8, ASCII fallback characters are
  used for separators, arrows, and cursor indicators.

use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use insta::assert_snapshot;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use test_case::test_case;

const READY_TIMEOUT: Duration = Duration::from_secs(10);
const RENDER_SETTLE: Duration = Duration::from_millis(100);
const UTF8_ENV: &[(&str, &str)] = &[("LC_ALL", "en_US.UTF-8")];

/// crossterm renders `Clear(FromCursorDown)` as CSI `J`; the prompt emits it
/// exactly once per render (in `clear_prompt`), so counting it tracks
/// re-renders without coupling to any user-facing text.
const RENDER_CLEAR: &str = "\x1b[J";

#[derive(Clone, Copy)]
enum PromptFixture {
    Basic,
    Many,
    Default,
    Custom,
    Multiline,
    Wide,
}

impl PromptFixture {
    const fn bin(self) -> &'static str {
        env!("CARGO_BIN_EXE_prompt_fixture")
    }

    const fn arg(self) -> &'static str {
        match self {
            PromptFixture::Basic => "basic",
            PromptFixture::Many => "many",
            PromptFixture::Default => "default",
            PromptFixture::Custom => "custom",
            PromptFixture::Multiline => "multiline",
            PromptFixture::Wide => "wide",
        }
    }
}

fn pty_size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

struct PromptSession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    rx: Receiver<Vec<u8>>,
    output: Vec<u8>,
    screen: vt100::Parser,
    cursor_query_answered: bool,
}

impl PromptSession {
    fn spawn(fixture: PromptFixture, rows: u16, cols: u16, env: &[(&str, &str)]) -> Self {
        let pair = native_pty_system()
            .openpty(pty_size(rows, cols))
            .expect("openpty");

        let mut command = CommandBuilder::new(fixture.bin());
        command.arg(fixture.arg());
        for (key, value) in env {
            command.env(*key, *value);
        }
        let child = pair.slave.spawn_command(command).expect("spawn child");
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().expect("clone reader");
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) if tx.send(buf[..n].to_vec()).is_err() => break,
                    Ok(_) => {}
                }
            }
        });

        let writer = pair.master.take_writer().expect("take writer");
        Self {
            master: pair.master,
            writer,
            child,
            rx,
            output: Vec::new(),
            screen: vt100::Parser::new(rows, cols, 0),
            cursor_query_answered: false,
        }
    }

    /// Spawns the fixture in a default 24×120 UTF-8 terminal.
    fn spawn_standard(fixture: PromptFixture) -> Self {
        Self::spawn(fixture, 24, 120, UTF8_ENV)
    }

    fn absorb(&mut self, chunk: Vec<u8>) {
        self.screen.process(&chunk);
        self.output.extend_from_slice(&chunk);
        // The prompt queries the cursor position once, before rendering
        // anything, so the cursor is always at home (row 1, column 1) at that
        // point. Answer the query so `crossterm::cursor::position` can return.
        if !self.cursor_query_answered && self.output.windows(4).any(|window| window == b"\x1b[6n")
        {
            self.cursor_query_answered = true;
            self.writer
                .write_all(b"\x1b[1;1R")
                .expect("answer cursor position query");
            self.writer.flush().expect("flush cursor position answer");
        }
    }

    /// Blocks until the prompt's cursor-position query has been answered. The
    /// prompt emits that query (and the hide-cursor sequence) only after
    /// entering raw mode, so this signals that sending input is now safe.
    fn wait_for_first_chunk(&mut self) {
        while !self.cursor_query_answered {
            let chunk = self
                .rx
                .recv_timeout(READY_TIMEOUT)
                .expect("prompt never rendered anything (timeout)");
            self.absorb(chunk);
        }
    }

    fn send(&mut self, keys: &[u8]) {
        self.writer.write_all(keys).expect("write keys");
        self.writer.flush().expect("flush keys");
    }

    fn resize(&mut self, rows: u16, cols: u16) {
        self.master.resize(pty_size(rows, cols)).expect("resize");
    }

    fn drain_and_wait(&mut self) {
        while let Ok(chunk) = self.rx.recv() {
            self.absorb(chunk);
        }
        self.child.wait().expect("child status");
    }

    fn finish(mut self) -> Vec<u8> {
        self.drain_and_wait();
        self.output
    }

    /// Drains output until the prompt has rendered `renders` times, so a
    /// mid-interaction screen snapshot reflects a fully flushed re-render
    /// rather than a wall-clock quiet period. Returns only once any tail bytes
    /// of the final render have settled.
    fn drain_until(&mut self, renders: usize) {
        while self.occurrences(RENDER_CLEAR) < renders {
            let chunk = self
                .rx
                .recv_timeout(READY_TIMEOUT)
                .expect("prompt did not render expected output (timeout)");
            self.absorb(chunk);
        }
        // The clear bytes mark the start of a render, so absorb any tail of
        // the final flush; the fixture blocks in event::read right after
        // flushing, so the stream going quiet means the render fully arrived.
        while let Ok(chunk) = self.rx.recv_timeout(RENDER_SETTLE) {
            self.absorb(chunk);
        }
    }

    fn occurrences(&self, marker: &str) -> usize {
        self.output
            .windows(marker.len())
            .filter(|window| *window == marker.as_bytes())
            .count()
    }

    fn screen(&mut self) -> &vt100::Screen {
        self.screen.screen()
    }

    /// Drains the remaining output and snapshots the full render.
    fn finish_and_snapshot(self) {
        let bytes = self.finish();
        let name = std::thread::current().name().unwrap().replace(":", "_");
        let mut settings = insta::Settings::new();
        settings.set_strip_ansi_escape_codes(true);
        settings.bind(|| assert_snapshot!(name, String::from_utf8_lossy(&bytes)));
    }
}

#[test_case(PromptFixture::Basic, b"\x1b" ; "esc_cancels_without_selection")]
#[test_case(PromptFixture::Basic, b"\x1b[B\r" ; "down_then_enter_confirms_second_option")]
#[test_case(PromptFixture::Basic, b"\t\r" ; "tab_then_enter_confirms_next_option")]
#[test_case(PromptFixture::Basic, b"\x1b[Z\r" ; "shift_tab_wraps_to_last_option")]
#[test_case(PromptFixture::Basic, b"m\r" ; "hotkey_m_selects_metro")]
#[test_case(PromptFixture::Basic, b"\x03" ; "ctrl_c_cancels")]
#[test_case(PromptFixture::Basic, b"!\r" ; "non_hotkey_ignored_then_enter_confirms")]
#[test_case(PromptFixture::Default, b"\x1b" ; "esc_accepts_default_selection")]
#[test_case(PromptFixture::Custom, b"z\r" ; "custom_hotkey_selects_carpool")]
fn keyboard_input_selects_or_cancels(fixture: PromptFixture, keys: &[u8]) {
    let mut session = PromptSession::spawn_standard(fixture);
    session.wait_for_first_chunk();
    session.send(keys);
    session.finish_and_snapshot();
}

#[test]
fn multiline_question_redisplays_from_column_zero_on_navigation() {
    let mut session = PromptSession::spawn_standard(PromptFixture::Multiline);
    session.wait_for_first_chunk();
    session.send(b"\x1b[B");
    session.send(b"\x1b[B");
    session.drain_until(3);
    assert_snapshot!(session.screen().contents());
    session.send(b"\r");
    session.finish();
}

#[test]
fn long_question_and_option_labels_wrap_and_rerender_cleanly() {
    let mut session = PromptSession::spawn(PromptFixture::Wide, 24, 40, UTF8_ENV);
    session.wait_for_first_chunk();
    session.send(b"\x1b[B");
    session.drain_until(2);
    let screen = session.screen();
    assert_eq!(
        screen.contents().matches("Select your preferred").count(),
        1
    );
    let (_, cols) = screen.size();
    let mut rows: Vec<String> = screen.rows(0, cols).collect();
    while rows.last().is_some_and(String::is_empty) {
        rows.pop();
    }
    let snapshot = rows.join("\n");
    assert_snapshot!(snapshot);
    session.send(b"\r");
    session.finish();
}

#[test]
fn unicode_disabled_renders_ascii_markers() {
    let mut session = PromptSession::spawn(
        PromptFixture::Basic,
        24,
        120,
        &[("LANG", "C"), ("LC_CTYPE", "C"), ("LC_ALL", "C")],
    );
    session.wait_for_first_chunk();
    session.send(b"\r");
    session.finish_and_snapshot();
}

#[test]
fn small_terminal_scrolls_options_to_stay_reachable() {
    let mut session = PromptSession::spawn(PromptFixture::Many, 4, 120, UTF8_ENV);
    session.wait_for_first_chunk();
    session.send(b"\x1b[B");
    session.send(b"\x1b[B");
    session.send(b"\r");
    session.finish_and_snapshot();
}

#[test]
fn resize_during_interaction_keeps_prompt_usable() {
    let mut session = PromptSession::spawn_standard(PromptFixture::Basic);
    session.wait_for_first_chunk();
    session.resize(6, 80);
    session.send(b"\r");
    session.finish_and_snapshot();
}

fn assert_color_free(output: &[u8]) {
    assert!(
        !output.windows(7).any(|window| window == b"\x1b[38;5;"),
        "output contains foreground color escape: {output:?}"
    );
    assert!(
        !output.windows(5).any(|window| window == b"\x1b[39m"),
        "output contains color reset escape: {output:?}"
    );
}

#[test]
fn no_color_env_disables_color_escapes() {
    let mut session = PromptSession::spawn(PromptFixture::Basic, 24, 120, &[("NO_COLOR", "1")]);
    session.wait_for_first_chunk();
    session.send(b"\r");
    assert_color_free(&session.finish());
}

#[test]
fn dumb_terminal_disables_color_escapes() {
    let mut session = PromptSession::spawn(PromptFixture::Basic, 24, 120, &[("TERM", "dumb")]);
    session.wait_for_first_chunk();
    session.send(b"\r");
    assert_color_free(&session.finish());
}

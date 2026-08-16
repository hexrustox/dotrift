use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use insta::assert_snapshot;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use test_case::test_case;

const READY_TIMEOUT: Duration = Duration::from_secs(10);
const UTF8_ENV: &[(&str, &str)] = &[("LC_ALL", "en_US.UTF-8")];

#[derive(Clone, Copy)]
enum PromptFixture {
    Basic,
    Many,
    Default,
    Custom,
}

impl PromptFixture {
    const fn bin(self) -> &'static str {
        match self {
            PromptFixture::Basic => env!("CARGO_BIN_EXE_prompt_basic"),
            PromptFixture::Many => env!("CARGO_BIN_EXE_prompt_many_options"),
            PromptFixture::Default => env!("CARGO_BIN_EXE_prompt_default"),
            PromptFixture::Custom => env!("CARGO_BIN_EXE_prompt_custom"),
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
}

impl PromptSession {
    fn spawn(fixture: PromptFixture, rows: u16, cols: u16, env: &[(&str, &str)]) -> Self {
        let pair = native_pty_system()
            .openpty(pty_size(rows, cols))
            .expect("openpty");

        let mut command = CommandBuilder::new(fixture.bin());
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
        }
    }

    /// Spawns the fixture in a default 24×120 UTF-8 terminal.
    fn spawn_standard(fixture: PromptFixture) -> Self {
        Self::spawn(fixture, 24, 120, UTF8_ENV)
    }

    /// Blocks until the prompt's first output chunk arrives. The prompt only
    /// emits output after entering raw mode, so this signals that sending
    /// input is now safe.
    fn wait_for_first_chunk(&mut self) {
        let chunk = self
            .rx
            .recv_timeout(READY_TIMEOUT)
            .expect("prompt never rendered anything (timeout)");
        self.output.extend_from_slice(&chunk);
    }

    fn send(&mut self, keys: &[u8]) {
        self.writer.write_all(keys).expect("write keys");
        self.writer.flush().expect("flush keys");
    }

    fn resize(&mut self, rows: u16, cols: u16) {
        self.master.resize(pty_size(rows, cols)).expect("resize");
    }

    fn finish(mut self) -> Vec<u8> {
        while let Ok(chunk) = self.rx.recv() {
            self.output.extend_from_slice(&chunk);
        }
        self.child.wait().expect("child status");
        self.output
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

#[test_case(PromptFixture::Basic, b"\x1b" ; "baseline_esc_cancels")]
#[test_case(PromptFixture::Basic, b"\x1b[B\r" ; "down_confirms_second")]
#[test_case(PromptFixture::Basic, b"\t\r" ; "tab_confirms_next")]
#[test_case(PromptFixture::Basic, b"\x1b[Z\r" ; "shift_tab_wraps_to_last")]
#[test_case(PromptFixture::Basic, b"m\r" ; "hotkey_m_selects_metro")]
#[test_case(PromptFixture::Basic, b"\x03" ; "ctrl_c_cancels")]
#[test_case(PromptFixture::Basic, b"!\r" ; "ignores_non_hotkey_then_confirms")]
#[test_case(PromptFixture::Default, b"\x1b" ; "default_selects_metro")]
#[test_case(PromptFixture::Custom, b"z\r" ; "custom_hotkey_selects_carpool")]
fn interact_under_pty(fixture: PromptFixture, keys: &[u8]) {
    let mut session = PromptSession::spawn_standard(fixture);
    session.wait_for_first_chunk();
    session.send(keys);
    session.finish_and_snapshot();
}

#[test]
fn unicode_off_ascii_markers() {
    let mut session = PromptSession::spawn(
        PromptFixture::Basic,
        24,
        120,
        &[("LANG", "C"), ("LC_CTYPE", "C"), ("LC_ALL", "C")],
    );
    session.wait_for_first_chunk();
    session.send(b"\x1b");
    session.finish_and_snapshot();
}

#[test]
fn small_terminal_windows_options() {
    let mut session = PromptSession::spawn(PromptFixture::Many, 4, 120, UTF8_ENV);
    session.wait_for_first_chunk();
    session.send(b"\x1b[B");
    session.send(b"\x1b[B");
    session.send(b"\x1b");
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

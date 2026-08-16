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
enum Probe {
    Basic,
    Many,
    Default,
    Custom,
}

const fn probe_bin(probe: Probe) -> &'static str {
    match probe {
        Probe::Basic => env!("CARGO_BIN_EXE_prompt_basic"),
        Probe::Many => env!("CARGO_BIN_EXE_prompt_many_options"),
        Probe::Default => env!("CARGO_BIN_EXE_prompt_default"),
        Probe::Custom => env!("CARGO_BIN_EXE_prompt_custom"),
    }
}

struct SpawnedProbe {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    rx: Receiver<Vec<u8>>,
    output: Vec<u8>,
}

impl SpawnedProbe {
    fn spawn(probe: Probe, rows: u16, cols: u16, env: &[(&str, &str)]) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty");

        let mut command = CommandBuilder::new(probe_bin(probe));
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

    /// Blocks until the probe's first output chunk arrives. The prompt only
    /// emits output after entering raw mode, so this signals that sending
    /// input is now safe.
    fn wait_for_first_chunk(&mut self) {
        let chunk = self
            .rx
            .recv_timeout(READY_TIMEOUT)
            .expect("probe never rendered anything (timeout)");
        self.output.extend_from_slice(&chunk);
    }

    fn send(&mut self, keys: &[u8]) {
        self.writer.write_all(keys).expect("write keys");
        self.writer.flush().expect("flush keys");
    }

    /// Returns the output received so far without ending the probe,
    /// waiting briefly for the stream to settle.
    fn drain_available(&mut self) -> Vec<u8> {
        let mut idle = 0;
        loop {
            match self.rx.try_recv() {
                Ok(chunk) => {
                    self.output.extend_from_slice(&chunk);
                    idle = 0;
                }
                Err(_) => {
                    idle += 1;
                    if idle >= 4 {
                        return self.output.clone();
                    }
                    thread::sleep(Duration::from_millis(25));
                }
            }
        }
    }

    fn resize(&mut self, rows: u16, cols: u16) {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("resize");
    }

    fn finish(mut self) -> Vec<u8> {
        while let Ok(chunk) = self.rx.recv() {
            self.output.extend_from_slice(&chunk);
        }
        self.child.wait().expect("child status");
        self.output
    }
}

fn assert_rendered(bytes: Vec<u8>) {
    let name = std::thread::current().name().unwrap().replace(":", "_");
    let mut settings = insta::Settings::new();
    settings.set_strip_ansi_escape_codes(true);
    settings.bind(|| assert_snapshot!(name, String::from_utf8_lossy(&bytes)));
}

#[test_case(Probe::Basic, b"\x1b" ; "baseline_esc_cancels")]
#[test_case(Probe::Basic, b"\x1b[B\r" ; "down_confirms_second")]
#[test_case(Probe::Basic, b"\t\r" ; "tab_confirms_next")]
#[test_case(Probe::Basic, b"\x1b[Z\r" ; "shift_tab_wraps_to_last")]
#[test_case(Probe::Basic, b"m\r" ; "hotkey_m_selects_metro")]
#[test_case(Probe::Basic, b"\x03" ; "ctrl_c_cancels")]
#[test_case(Probe::Basic, b"!\r" ; "ignores_non_hotkey_then_confirms")]
#[test_case(Probe::Default, b"\x1b" ; "default_selects_metro")]
#[test_case(Probe::Custom, b"z\r" ; "custom_hotkey_selects_carpool")]
fn interact_under_pty(probe: Probe, keys: &[u8]) {
    let mut spawned = SpawnedProbe::spawn(probe, 24, 120, UTF8_ENV);
    spawned.wait_for_first_chunk();
    spawned.send(keys);
    let bytes = spawned.finish();
    assert_rendered(bytes);
}

#[test]
fn unicode_off_ascii_markers() {
    let mut spawned = SpawnedProbe::spawn(
        Probe::Basic,
        24,
        120,
        &[("LANG", "C"), ("LC_CTYPE", "C"), ("LC_ALL", "C")],
    );
    spawned.wait_for_first_chunk();
    spawned.send(b"\x1b");
    assert_rendered(spawned.finish());
}

#[test]
fn small_terminal_windows_options() {
    let mut spawned = SpawnedProbe::spawn(Probe::Many, 4, 120, UTF8_ENV);
    spawned.wait_for_first_chunk();
    spawned.send(b"\x1b[B");
    spawned.send(b"\x1b[B");
    spawned.send(b"\x1b");
    assert_rendered(spawned.finish());
}

#[test]
fn resize_during_interaction_keeps_prompt_usable() {
    let mut spawned = SpawnedProbe::spawn(Probe::Basic, 24, 120, UTF8_ENV);
    spawned.wait_for_first_chunk();
    spawned.resize(6, 80);
    spawned.send(b"\r");
    assert_rendered(spawned.finish());
}

#[test]
fn arrows_reposition_before_confirmation() {
    let mut spawned = SpawnedProbe::spawn(Probe::Basic, 24, 120, UTF8_ENV);
    spawned.wait_for_first_chunk();
    spawned.send(b"\x1b[B\x1b[B");
    let before = spawned.drain_available();
    spawned.send(b"\r");
    let after = spawned.finish();

    assert_rendered(before);
    assert_rendered(after);
}

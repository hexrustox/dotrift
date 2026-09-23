use std::fmt;

use crossterm::style::Color;
use tui::apply_color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    Deployed,
    Replaced,
    Skipped,
    Removed,
    Pruned,
    Obstruction,
    Managed,
    Unmanaged,
    Active,
}

impl Outcome {
    fn color(self) -> Color {
        match self {
            Outcome::Deployed | Outcome::Managed | Outcome::Active => Color::Green,
            Outcome::Replaced => Color::Cyan,
            Outcome::Skipped => Color::DarkGrey,
            Outcome::Removed | Outcome::Unmanaged => Color::Red,
            Outcome::Pruned => Color::Magenta,
            Outcome::Obstruction => Color::Yellow,
        }
    }
}

/// Owns the run's output policy — color support, outcome gating, and quiet —
/// so call sites print without deciding any of it themselves.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Reporter {
    color: bool,
    outcome_enabled: bool,
    quiet: bool,
}

impl Reporter {
    pub(crate) fn always(color: bool) -> Self {
        Self::new(color, true, false)
    }

    pub(crate) fn new(color: bool, outcome_enabled: bool, quiet: bool) -> Self {
        Self {
            color,
            outcome_enabled,
            quiet,
        }
    }

    pub(crate) fn paint<F>(&self, outcome: Outcome, content: F) -> String
    where
        F: fmt::Display + crossterm::style::Stylize,
        F::Styled: fmt::Display,
    {
        apply_color(content, outcome.color(), self.color)
    }

    pub(crate) fn line(&self, args: fmt::Arguments) {
        stdout_imp::emit_stdout(&format!("{args}\n"));
    }

    /// Prints the caller-formatted line when the run's outcome policy
    /// (verbose or dry-run) allows it; nothing is emitted when suppressed.
    pub(crate) fn outcome_line(&self, args: fmt::Arguments) {
        if !self.outcome_enabled {
            return;
        }
        self.line(args);
    }

    /// Prints the run summary unless the run is quiet.
    pub(crate) fn summary(&self, args: fmt::Arguments) {
        if self.quiet {
            return;
        }
        stdout_imp::emit_stdout(&format!("{args}\n"));
    }

    /// Prints to standard error, never suppressed.
    pub(crate) fn warning(&self, args: fmt::Arguments) {
        stdout_imp::emit_stderr(&format!(
            "{} {args}\n",
            apply_color("WARNING", Color::Yellow, self.color)
        ));
    }
}

/// Where the content diff is written when no pager ran.
pub(crate) fn diff_sink() -> impl std::io::Write {
    stdout_imp::diff_sink()
}

#[cfg(feature = "testing")]
pub use stdout_imp::{clear, take_errors, take_output};

#[cfg(not(feature = "testing"))]
mod stdout_imp {
    pub(super) fn emit_stdout(line: &str) {
        print!("{line}");
    }

    pub(super) fn emit_stderr(line: &str) {
        eprint!("{line}");
    }

    pub(super) fn diff_sink() -> std::io::Stdout {
        std::io::stdout()
    }
}

#[cfg(feature = "testing")]
mod stdout_imp {
    use std::cell::RefCell;

    thread_local! {
        static OUTPUT: RefCell<String> = const { RefCell::new(String::new()) };
        static ERRORS: RefCell<String> = const { RefCell::new(String::new()) };
    }

    pub(super) fn emit_stdout(line: &str) {
        OUTPUT.with(|output| output.borrow_mut().push_str(line));
    }

    pub(super) fn emit_stderr(line: &str) {
        ERRORS.with(|errors| errors.borrow_mut().push_str(line));
    }

    pub fn take_output() -> String {
        OUTPUT.with(|output| std::mem::take(&mut *output.borrow_mut()))
    }

    pub fn take_errors() -> String {
        ERRORS.with(|errors| std::mem::take(&mut *errors.borrow_mut()))
    }

    pub fn clear() {
        OUTPUT.with(|output| output.borrow_mut().clear());
        ERRORS.with(|errors| errors.borrow_mut().clear());
    }

    pub(super) struct OutputWriter;

    impl std::io::Write for OutputWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            OUTPUT.with(|output| {
                output
                    .borrow_mut()
                    .push_str(String::from_utf8_lossy(buf).as_ref())
            });
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    pub(super) fn diff_sink() -> OutputWriter {
        OutputWriter
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use test_case::test_case;

    use super::*;

    #[test_case(false, "" ; "suppressed_without_outcome_enabled")]
    #[test_case(true, "hello\n" ; "prints_caller_formatted_args")]
    fn outcome_line_respects_outcome_enabled(outcome_enabled: bool, expected: &str) {
        clear();
        Reporter::new(false, outcome_enabled, false).outcome_line(format_args!("hello"));
        assert_eq!(take_output(), expected);
    }

    #[test]
    fn summary_is_suppressed_when_quiet() {
        clear();
        Reporter::new(false, true, true).summary(format_args!("hello"));
        assert_eq!(take_output(), "");
        Reporter::new(false, true, false).summary(format_args!("hello"));
        assert_eq!(take_output(), "hello\n");
    }

    #[test]
    fn line_prints_formatted_args() {
        clear();
        Reporter::always(false).line(format_args!("{} {:<8}", "a", 3));
        assert_eq!(take_output(), "a 3       \n");
    }

    #[test]
    fn output_and_error_buffers_are_independent() {
        clear();
        Reporter::always(false).line(format_args!("out"));
        Reporter::always(false).warning(format_args!("err"));
        assert_eq!(take_output(), "out\n");
        assert_eq!(take_errors(), "WARNING err\n");
        assert_eq!(take_output(), "");
    }

    #[test]
    fn diff_sink_writes_into_the_output_buffer() {
        clear();
        diff_sink().write_all(b"diff bytes").unwrap();
        assert_eq!(take_output(), "diff bytes");
    }

    #[test]
    fn paint_without_color_support_keeps_plain_text() {
        let report = Reporter::always(false);
        assert_eq!(report.paint(Outcome::Managed, "managed"), "managed");
    }
}

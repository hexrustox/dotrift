// TODO refactor
use std::cell::RefCell;

#[cfg(feature = "testing")]
#[macro_export]
macro_rules! println_capture {
    () => {
        $crate::capture::push("\n")
    };
    ($($arg:tt)*) => {{
        let mut line = format!($($arg)*);
        line.push('\n');
        $crate::capture::push(&line);
    }};
}

#[cfg(not(feature = "testing"))]
#[macro_export]
macro_rules! println_capture {
    ($($arg:tt)*) => {
        println!($($arg)*)
    };
}

thread_local! {
    static BUFFER: RefCell<String> = const { RefCell::new(String::new()) };
}

pub fn push(line: &str) {
    BUFFER.with(|buffer| buffer.borrow_mut().push_str(line));
}

#[cfg(feature = "testing")]
pub struct CaptureWriter;

#[cfg(feature = "testing")]
impl std::io::Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        push(String::from_utf8_lossy(buf).as_ref());
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub fn take() -> String {
    BUFFER.with(|buffer| std::mem::take(&mut *buffer.borrow_mut()))
}

pub fn clear() {
    BUFFER.with(|buffer| buffer.borrow_mut().clear());
}

#[cfg(test)]
mod tests {
    #[test]
    fn captures_formatted_args_with_trailing_newline() {
        crate::capture::clear();
        crate::println_capture!("hello {}", "world");
        crate::println_capture!();
        assert_eq!(crate::capture::take(), "hello world\n\n");
    }
}

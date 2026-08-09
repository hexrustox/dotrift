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

pub fn take() -> String {
    BUFFER.with(|buffer| std::mem::take(&mut *buffer.borrow_mut()))
}

pub fn clear() {
    BUFFER.with(|buffer| buffer.borrow_mut().clear());
}

#[cfg(test)]
mod tests {
    #[test]
    fn macro_captures_formatted_lines() {
        crate::capture::clear();
        crate::println_capture!("hello {}", "world");
        crate::println_capture!();
        assert_eq!(crate::capture::take(), "hello world\n\n");
    }
}

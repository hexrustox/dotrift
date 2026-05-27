use std::fmt;

#[derive(Debug)]
pub struct Error {
    pub msg: String,
    pub at: usize,
    pub len: usize,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.msg)
    }
}

impl std::error::Error for Error {}

impl Error {
    pub fn new(msg: impl Into<String>, at: usize, len: usize) -> Self {
        Self {
            msg: msg.into(),
            at,
            len,
        }
    }

    pub fn span(&self) -> (usize, usize) {
        (self.at, self.len)
    }

    pub fn line_col(&self, source: &[u8]) -> (usize, usize) {
        let mut line = 1;
        let mut col = 1;
        for (i, &b) in source.iter().enumerate() {
            if i >= self.at {
                break;
            }
            if b == b'\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
        }
        (line, col)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_line_col_first_line() {
        let source = b"hello";
        let err = Error::new("test", 0, 1);
        assert_eq!(err.line_col(source), (1, 1));
    }

    #[test]
    fn test_line_col_second_line() {
        let source = b"hello\nworld";
        let err = Error::new("test", 7, 1);
        assert_eq!(err.line_col(source), (2, 2));
    }

    #[test]
    fn test_line_col_empty() {
        let source = b"";
        let err = Error::new("test", 0, 1);
        assert_eq!(err.line_col(source), (1, 1));
    }
}

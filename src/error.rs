//! Errors carry a sentence for a person, not a code: nothing above `main`
//! branches on a variant.

use std::fmt;

/// A refusal, with the sentence the user reads.
#[derive(Debug)]
pub struct Error {
    message: String,
}

impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

/// The result of anything that can refuse.
pub type Result<T> = std::result::Result<T, Error>;

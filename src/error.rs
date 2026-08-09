//! What a refusal is.
//!
//! The app's rule is *refuse rather than guess*, and every refusal ends up in
//! front of a person: either on their shell before tmux exists, or on the pane
//! they are typing in. So an error carries a sentence, not a code — there is
//! nothing above `main` that would branch on a variant.

use std::fmt;

/// A refusal, with the sentence the user reads.
#[derive(Debug)]
pub struct Error {
    message: String,
}

impl Error {
    /// Refuse, saying why.
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

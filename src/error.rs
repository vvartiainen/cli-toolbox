use std::io;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
    #[error("{0}")]
    Displayed(String),
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl Error {
    pub fn msg(message: impl std::fmt::Display) -> Self {
        Self::Message(message.to_string())
    }

    pub fn displayed(message: impl std::fmt::Display) -> Self {
        Self::Displayed(message.to_string())
    }

    pub fn was_displayed(&self) -> bool {
        matches!(self, Self::Displayed(_))
    }
}

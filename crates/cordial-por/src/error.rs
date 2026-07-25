use std::fmt;

/// Error placeholder for future Proof-of-Reputation validation and transitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PorError {
    InvalidConfiguration(String),
}

impl fmt::Display for PorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration(message) => {
                write!(f, "invalid Proof-of-Reputation configuration: {message}")
            }
        }
    }
}

impl std::error::Error for PorError {}

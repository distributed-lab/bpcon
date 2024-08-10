//! Definition of the BPCon errors.

use std::fmt;

#[derive(Debug)]
pub enum BallotError {
    MessageParsing(String),
    InvalidState(String),
    Communication(String),
}

impl fmt::Display for BallotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            BallotError::MessageParsing(ref err) => write!(f, "Message parsing error: {}", err),
            BallotError::InvalidState(ref err) => write!(f, "Invalid state error: {}", err),
            BallotError::Communication(ref err) => write!(f, "Communication error: {}", err),
        }
    }
}

impl std::error::Error for BallotError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        // Since these are all simple String errors, there is no underlying source error.
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ballot_error_message_parsing() {
        let error = BallotError::MessageParsing("Parsing failed".into());
        if let BallotError::MessageParsing(msg) = error {
            assert_eq!(msg, "Parsing failed");
        } else {
            panic!("Expected MessageParsing error");
        }
    }

    #[test]
    fn test_ballot_error_invalid_state() {
        let error = BallotError::InvalidState("Invalid state transition".into());
        if let BallotError::InvalidState(msg) = error {
            assert_eq!(msg, "Invalid state transition");
        } else {
            panic!("Expected InvalidState error");
        }
    }
}

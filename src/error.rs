//! Definition of the BPCon errors.
#[derive(Debug)]
pub enum BallotError {
    MessageParsing(String),
    InvalidState(String),
    Communication(String),
    LeaderElection,
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

//! Definition of the BPCon errors.
#[derive(Debug)]
pub enum BallotError {
    MessageParsing(String),
    InvalidState(String),
    Communication(String),
    LeaderElection,
}

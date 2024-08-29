use crate::party::{PartyEvent, PartyStatus};
use crate::value::Value;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LaunchBallotError {
    #[error("failed to send event {0}: {1}")]
    FailedToSendEvent(PartyEvent, String),
    #[error("event channel closed")]
    EventChannelClosed,
    #[error("message channel closed")]
    MessageChannelClosed,
    #[error("failed to follow event {0}: {1}")]
    FollowEventError(PartyEvent, FollowEventError),
    #[error("leader election error: {0}")]
    LeaderElectionError(String),
}

#[derive(Error, Debug)]
pub enum FollowEventError {
    #[error("{0}")]
    PartyStatusMismatch(PartyStatusMismatch),
    #[error("{0}")]
    SerializationError(SerializationError),
    #[error("{0}")]
    FailedToSendMessage(String),
}

#[derive(Error, Debug)]
pub enum UpdateStateError<V: Value> {
    #[error("{0}")]
    PartyStatusMismatch(PartyStatusMismatch),
    #[error("{0}")]
    BallotNumberMismatch(BallotNumberMismatch),
    #[error("{0}")]
    LeaderMismatch(LeaderMismatch),
    #[error("{0}")]
    ValueMismatch(ValueMismatch<V>),
    #[error("value verification failed")]
    ValueVerificationFailed,
    #[error("{0}")]
    DeserializationError(DeserializationError),
}

#[derive(Error, Debug)]
#[error(
    "party status mismatch: party status is {party_status} whilst needed status is {needed_status}"
)]
pub struct PartyStatusMismatch {
    pub party_status: PartyStatus,
    pub needed_status: PartyStatus,
}

#[derive(Error, Debug)]
#[error("ballot number mismatch: party's ballot number is {party_ballot_number} whilst received {message_ballot_number} in the message")]
pub struct BallotNumberMismatch {
    pub party_ballot_number: u64,
    pub message_ballot_number: u64,
}

#[derive(Error, Debug)]
#[error("leader mismatch: party's leader is {party_leader} whilst the message was sent by {message_sender}")]
pub struct LeaderMismatch {
    pub party_leader: u64,
    pub message_sender: u64,
}

#[derive(Error, Debug)]
#[error(
    "value mismatch: party's value is {party_value} whilst received {message_value} in the message"
)]
pub struct ValueMismatch<V: Value> {
    pub party_value: V,
    pub message_value: V,
}

#[derive(Error, Debug)]
pub enum DeserializationError {
    #[error("message deserialization error: {0}")]
    Message(String),
    #[error("value deserialization error: {0}")]
    Value(String),
}

#[derive(Error, Debug)]
pub enum SerializationError {
    #[error("message serialization error: {0}")]
    Message(String),
    #[error("value serialization error: {0}")]
    Value(String),
}

impl From<PartyStatusMismatch> for FollowEventError {
    fn from(error: PartyStatusMismatch) -> Self {
        FollowEventError::PartyStatusMismatch(error)
    }
}

impl From<SerializationError> for FollowEventError {
    fn from(error: SerializationError) -> Self {
        FollowEventError::SerializationError(error)
    }
}

impl<V: Value> From<PartyStatusMismatch> for UpdateStateError<V> {
    fn from(error: PartyStatusMismatch) -> Self {
        UpdateStateError::PartyStatusMismatch(error)
    }
}

impl<V: Value> From<LeaderMismatch> for UpdateStateError<V> {
    fn from(error: LeaderMismatch) -> Self {
        UpdateStateError::LeaderMismatch(error)
    }
}

impl<V: Value> From<BallotNumberMismatch> for UpdateStateError<V> {
    fn from(error: BallotNumberMismatch) -> Self {
        UpdateStateError::BallotNumberMismatch(error)
    }
}

impl<V: Value> From<ValueMismatch<V>> for UpdateStateError<V> {
    fn from(error: ValueMismatch<V>) -> Self {
        UpdateStateError::ValueMismatch(error)
    }
}

impl<V: Value> From<DeserializationError> for UpdateStateError<V> {
    fn from(error: DeserializationError) -> Self {
        UpdateStateError::DeserializationError(error)
    }
}

//! Definitions central to BPCon errors.

use crate::party::{PartyEvent, PartyStatus};
use crate::value::Value;
use thiserror::Error;

/// Represents the various errors that can occur during the ballot launch process.
#[derive(Error, Debug)]
pub enum LaunchBallotError {
    /// Occurs when an attempt to send a `PartyEvent` fails.
    ///
    /// - `0`: The `PartyEvent` that failed to send.
    /// - `1`: A string detailing the specific failure reason.
    #[error("failed to send event {0}: {1}")]
    FailedToSendEvent(PartyEvent, String),

    /// Occurs when the event channel is unexpectedly closed.
    #[error("event channel closed")]
    EventChannelClosed,

    /// Occurs when the message channel is unexpectedly closed.
    #[error("message channel closed")]
    MessageChannelClosed,

    /// Occurs when following a `PartyEvent` fails.
    ///
    /// - `0`: The `PartyEvent` that caused the error.
    /// - `1`: The specific `FollowEventError` detailing why the follow operation failed.
    #[error("failed to follow event {0}: {1}")]
    FollowEventError(PartyEvent, FollowEventError),

    /// Occurs during leader election if an error is encountered.
    ///
    /// - `0`: A string providing details about the leader election failure.
    #[error("leader election error: {0}")]
    LeaderElectionError(String),
}

/// Represents the various errors that can occur while following a party event.
#[derive(Error, Debug)]
pub enum FollowEventError {
    /// Occurs when there is a mismatch in the expected party status.
    ///
    /// - `0`: The specific `PartyStatusMismatch` that caused the error.
    #[error("{0}")]
    PartyStatusMismatch(PartyStatusMismatch),

    /// Occurs when there is an error during serialization.
    ///
    /// - `0`: The specific `SerializationError` encountered.
    #[error("{0}")]
    SerializationError(SerializationError),

    /// Occurs when sending a message related to the event fails.
    ///
    /// - `0`: A string describing the reason for the failure.
    #[error("{0}")]
    FailedToSendMessage(String),
}

/// Represents the various errors that can occur when updating the state in a consensus protocol.
///
/// This enum is generic over `V`, which must implement the `Value` trait.
#[derive(Error, Debug)]
pub enum UpdateStateError<V: Value> {
    /// Occurs when there is a mismatch in the expected party status.
    ///
    /// - `0`: The specific `PartyStatusMismatch` that caused the error.
    #[error("{0}")]
    PartyStatusMismatch(PartyStatusMismatch),

    /// Occurs when there is a mismatch in the ballot number.
    ///
    /// - `0`: The specific `BallotNumberMismatch` that caused the error.
    #[error("{0}")]
    BallotNumberMismatch(BallotNumberMismatch),

    /// Occurs when there is a mismatch in the expected leader.
    ///
    /// - `0`: The specific `LeaderMismatch` that caused the error.
    #[error("{0}")]
    LeaderMismatch(LeaderMismatch),

    /// Occurs when there is a mismatch in the expected value.
    ///
    /// - `0`: The specific `ValueMismatch<V>` that caused the error, where `V` is the type of the value.
    #[error("{0}")]
    ValueMismatch(ValueMismatch<V>),

    /// Occurs when the value verification fails during the state update process.
    #[error("value verification failed")]
    ValueVerificationFailed,

    /// Occurs when there is an error during deserialization.
    ///
    /// - `0`: The specific `DeserializationError` encountered.
    #[error("{0}")]
    DeserializationError(DeserializationError),
}

/// Represents an error that occurs when there is a mismatch between the current status of the party
/// and the status required for an operation.
///
/// This error is typically encountered when an operation requires the party to be in a specific
/// state, but the party is in a different state.
#[derive(Error, Debug)]
#[error(
    "party status mismatch: party status is {party_status} whilst needed status is {needed_status}"
)]
pub struct PartyStatusMismatch {
    /// The current status of the party.
    pub party_status: PartyStatus,
    /// The status required for the operation to proceed.
    pub needed_status: PartyStatus,
}

/// Represents an error that occurs when there is a mismatch between the ballot number of the party
/// and the ballot number received in a message.
#[derive(Error, Debug)]
#[error("ballot number mismatch: party's ballot number is {party_ballot_number} whilst received {message_ballot_number} in the message")]
pub struct BallotNumberMismatch {
    /// The ballot number held by the party.
    pub party_ballot_number: u64,
    /// The ballot number received in the message.
    pub message_ballot_number: u64,
}

/// Represents an error that occurs when there is a mismatch between the expected leader
/// of the party and the sender of the message.
#[derive(Error, Debug)]
#[error("leader mismatch: party's leader is {party_leader} whilst the message was sent by {message_sender}")]
pub struct LeaderMismatch {
    /// The leader identifier of the party.
    pub party_leader: u64,
    /// The identifier of the message sender.
    pub message_sender: u64,
}

/// Represents an error that occurs when there is a mismatch between the value held by the party
/// and the value received in a message.
///
/// This struct is generic over `V`, which must implement the `Value` trait.
#[derive(Error, Debug)]
#[error(
    "value mismatch: party's value is {party_value} whilst received {message_value} in the message"
)]
pub struct ValueMismatch<V: Value> {
    /// The value held by the party.
    pub party_value: V,
    /// The value received in the message.
    pub message_value: V,
}

/// Represents the various errors that can occur during the deserialization process.
#[derive(Error, Debug)]
pub enum DeserializationError {
    /// Occurs when there is an error deserializing a message.
    ///
    /// - `0`: A string describing the specific deserialization error for the message.
    #[error("message deserialization error: {0}")]
    Message(String),

    /// Occurs when there is an error deserializing a value.
    ///
    /// - `0`: A string describing the specific deserialization error for the value.
    #[error("value deserialization error: {0}")]
    Value(String),
}

/// Represents the various errors that can occur during the serialization process.
#[derive(Error, Debug)]
pub enum SerializationError {
    /// Occurs when there is an error serializing a message.
    ///
    /// - `0`: A string describing the specific serialization error for the message.
    #[error("message serialization error: {0}")]
    Message(String),

    /// Occurs when there is an error serializing a value.
    ///
    /// - `0`: A string describing the specific serialization error for the value.
    #[error("value serialization error: {0}")]
    Value(String),
}

/// Converts a `PartyStatusMismatch` into a `FollowEventError`.
///
/// This conversion allows `PartyStatusMismatch` errors to be treated as `FollowEventError`,
/// which may occur when there is a status mismatch while following an event.
impl From<PartyStatusMismatch> for FollowEventError {
    fn from(error: PartyStatusMismatch) -> Self {
        FollowEventError::PartyStatusMismatch(error)
    }
}

/// Converts a `SerializationError` into a `FollowEventError`.
///
/// This conversion is used when a serialization error occurs during the process
/// of following an event, allowing it to be handled as a `FollowEventError`.
impl From<SerializationError> for FollowEventError {
    fn from(error: SerializationError) -> Self {
        FollowEventError::SerializationError(error)
    }
}

/// Converts a `PartyStatusMismatch` into an `UpdateStateError<V>`.
///
/// This conversion is useful when a status mismatch occurs while updating the state,
/// allowing the error to be handled within the context of state updates.
impl<V: Value> From<PartyStatusMismatch> for UpdateStateError<V> {
    fn from(error: PartyStatusMismatch) -> Self {
        UpdateStateError::PartyStatusMismatch(error)
    }
}

/// Converts a `LeaderMismatch` into an `UpdateStateError<V>`.
///
/// This conversion is used when there is a leader mismatch during state updates,
/// allowing the error to be propagated and handled as an `UpdateStateError`.
impl<V: Value> From<LeaderMismatch> for UpdateStateError<V> {
    fn from(error: LeaderMismatch) -> Self {
        UpdateStateError::LeaderMismatch(error)
    }
}

/// Converts a `BallotNumberMismatch` into an `UpdateStateError<V>`.
///
/// This conversion is used when there is a ballot number mismatch during state updates,
/// allowing the error to be propagated and handled as an `UpdateStateError`.
impl<V: Value> From<BallotNumberMismatch> for UpdateStateError<V> {
    fn from(error: BallotNumberMismatch) -> Self {
        UpdateStateError::BallotNumberMismatch(error)
    }
}

/// Converts a `ValueMismatch<V>` into an `UpdateStateError<V>`.
///
/// This conversion is used when there is a value mismatch during state updates,
/// allowing the error to be propagated and handled as an `UpdateStateError`.
impl<V: Value> From<ValueMismatch<V>> for UpdateStateError<V> {
    fn from(error: ValueMismatch<V>) -> Self {
        UpdateStateError::ValueMismatch(error)
    }
}

/// Converts a `DeserializationError` into an `UpdateStateError<V>`.
///
/// This conversion is used when a deserialization error occurs during state updates,
/// allowing the error to be handled within the context of state updates.
impl<V: Value> From<DeserializationError> for UpdateStateError<V> {
    fn from(error: DeserializationError) -> Self {
        UpdateStateError::DeserializationError(error)
    }
}

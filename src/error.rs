//! Definition of the BPCon errors.

use crate::party::PartyStatus;
use crate::Value;
use std::fmt::{Display, Formatter, Result};

pub enum LaunchBallotError {
    FailedToSendEvent(String),
    EventChannelClosed,
    MessageChannelClosed,
    FollowEventError(FollowEventError),
    LeaderElectionError(String),
}

pub enum FollowEventError {
    PartyStatusMismatch(PartyStatusMismatch),
    SerializationError(SerializationError),
    FailedToSendMessage(String),
}

pub enum UpdateStateError<V: Value> {
    PartyStatusMismatch(PartyStatusMismatch),
    BallotNumberMismatch(BallotNumberMismatch),
    LeaderMismatch(LeaderMismatch),
    ValueMismatch(ValueMismatch<V>),
    ValueVerificationFailed,
    DeserializationError(DeserializationError),
}

pub struct PartyStatusMismatch {
    pub party_status: PartyStatus,
    pub needed_status: PartyStatus,
}

pub struct BallotNumberMismatch {
    pub party_ballot_number: u64,
    pub message_ballot_number: u64,
}

pub struct LeaderMismatch {
    pub party_leader: u64,
    pub message_sender: u64,
}

pub struct ValueMismatch<V: Value> {
    pub party_value: V,
    pub message_value: V,
}

pub enum DeserializationError {
    Message(String),
    Value(String),
}

pub enum SerializationError {
    Message(String),
    Value(String),
}

impl From<FollowEventError> for LaunchBallotError {
    fn from(error: FollowEventError) -> Self {
        LaunchBallotError::FollowEventError(error)
    }
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

impl Display for PartyStatusMismatch {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(
            f,
            "Party status mismatch: party status is {:?} whilst needed status is {:?}.",
            self.party_status, self.needed_status
        )
    }
}

impl Display for BallotNumberMismatch {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(
            f,
            "Ballot number mismatch: party's ballot number is {} whilst received {} in the message.",
            self.party_ballot_number, self.message_ballot_number
        )
    }
}

impl Display for LeaderMismatch {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(
            f,
            "Leader mismatch: party's leader is {} whilst the message was sent by {}.",
            self.party_leader, self.message_sender
        )
    }
}

impl<V: Value> Display for ValueMismatch<V> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(
            f,
            "Value mismatch: party's value is {} whilst received {} in the message.",
            self.party_value, self.message_value
        )
    }
}

impl Display for DeserializationError {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            DeserializationError::Message(err) => {
                write!(f, "Message deserialization error: {}", err)
            }
            DeserializationError::Value(err) => {
                write!(f, "Value deserialization error: {}", err)
            }
        }
    }
}

impl Display for SerializationError {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            SerializationError::Message(err) => {
                write!(f, "Message serialization error: {}", err)
            }
            SerializationError::Value(err) => {
                write!(f, "Value serialization error: {}", err)
            }
        }
    }
}

impl Display for LaunchBallotError {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match *self {
            LaunchBallotError::FailedToSendEvent(ref err) => write!(
                f,
                "Got error while running ballot: failed to send event: {}",
                err
            ),
            LaunchBallotError::EventChannelClosed => {
                write!(f, "Got error while running ballot: event channel closed")
            }
            LaunchBallotError::MessageChannelClosed => {
                write!(f, "Got error while running ballot: message channel closed")
            }
            LaunchBallotError::FollowEventError(ref err) => {
                write!(f, "Got error while running ballot: {}", err)
            }
            LaunchBallotError::LeaderElectionError(ref err) => write!(
                f,
                "Got error while running ballot: leader election error{}",
                err
            ),
        }
    }
}

impl Display for FollowEventError {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match *self {
            FollowEventError::PartyStatusMismatch(ref err) => {
                write!(f, "Unable to follow event: {}", err)
            }
            FollowEventError::SerializationError(ref err) => {
                write!(f, "Unable to follow event: {}", err)
            }
            FollowEventError::FailedToSendMessage(ref err) => {
                write!(f, "Unable to follow event: {}", err)
            }
        }
    }
}

impl<V: Value> Display for UpdateStateError<V> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match *self {
            UpdateStateError::PartyStatusMismatch(ref err) => {
                write!(f, "Unable to update state: {}", err)
            }
            UpdateStateError::BallotNumberMismatch(ref err) => {
                write!(f, "Unable to update state: {}", err)
            }
            UpdateStateError::LeaderMismatch(ref err) => {
                write!(f, "Unable to update state: {}", err)
            }
            UpdateStateError::ValueMismatch(ref err) => {
                write!(f, "Unable to update state: {}", err)
            }
            UpdateStateError::ValueVerificationFailed => {
                write!(f, "Unable to update state: value verification failed")
            }
            UpdateStateError::DeserializationError(ref err) => {
                write!(f, "Unable to update state: {}", err)
            }
        }
    }
}

//! Definition of the BPCon errors.

use crate::party::{PartyEvent, PartyStatus};
use crate::Value;
use std::fmt::{Display, Formatter, Result};

pub enum LaunchBallotError {
    FailedToSendEvent((PartyEvent, String)),
    EventChannelClosed,
    MessageChannelClosed,
    FollowEventError((PartyEvent, FollowEventError)),
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
            "party status mismatch: party status is {} whilst needed status is {}",
            self.party_status, self.needed_status
        )
    }
}

impl Display for BallotNumberMismatch {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(
            f,
            "ballot number mismatch: party's ballot number is {} whilst received {} in the message",
            self.party_ballot_number, self.message_ballot_number
        )
    }
}

impl Display for LeaderMismatch {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(
            f,
            "leader mismatch: party's leader is {} whilst the message was sent by {}",
            self.party_leader, self.message_sender
        )
    }
}

impl<V: Value> Display for ValueMismatch<V> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(
            f,
            "value mismatch: party's value is {} whilst received {} in the message",
            self.party_value, self.message_value
        )
    }
}

impl Display for DeserializationError {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            DeserializationError::Message(err) => {
                write!(f, "message deserialization error: {err}")
            }
            DeserializationError::Value(err) => {
                write!(f, "value deserialization error: {err}")
            }
        }
    }
}

impl Display for SerializationError {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            SerializationError::Message(err) => {
                write!(f, "message serialization error: {err}")
            }
            SerializationError::Value(err) => {
                write!(f, "value serialization error: {err}")
            }
        }
    }
}

impl Display for LaunchBallotError {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "launch ballot error: ")?;
        match *self {
            LaunchBallotError::FailedToSendEvent((ref event, ref err)) => {
                write!(f, "failed to send event {event}: {err}",)
            }
            LaunchBallotError::EventChannelClosed => {
                write!(f, "event channel closed")
            }
            LaunchBallotError::MessageChannelClosed => {
                write!(f, "message channel closed")
            }
            LaunchBallotError::FollowEventError((ref event, ref err)) => {
                write!(f, "failed to follow event {event}: {err}",)
            }
            LaunchBallotError::LeaderElectionError(ref err) => {
                write!(f, "leader election error: {err}",)
            }
        }
    }
}

impl Display for FollowEventError {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "follow event error: ")?;
        match *self {
            FollowEventError::PartyStatusMismatch(ref err) => {
                write!(f, "{err}")
            }
            FollowEventError::SerializationError(ref err) => {
                write!(f, "{err}")
            }
            FollowEventError::FailedToSendMessage(ref err) => {
                write!(f, "{err}")
            }
        }
    }
}

impl<V: Value> Display for UpdateStateError<V> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "update state error: ")?;
        match *self {
            UpdateStateError::PartyStatusMismatch(ref err) => {
                write!(f, "{err}")
            }
            UpdateStateError::BallotNumberMismatch(ref err) => {
                write!(f, "{err}")
            }
            UpdateStateError::LeaderMismatch(ref err) => {
                write!(f, "{err}")
            }
            UpdateStateError::ValueMismatch(ref err) => {
                write!(f, "{err}")
            }
            UpdateStateError::ValueVerificationFailed => {
                write!(f, "value verification failed")
            }
            UpdateStateError::DeserializationError(ref err) => {
                write!(f, "{err}")
            }
        }
    }
}

//! Definition of the BPCon messages.

use crate::error::{DeserializationError, SerializationError};
use rkyv::{AlignedVec, Archive, Deserialize, Infallible, Serialize};
use std::collections::HashSet;

/// Message ready for transfer.
#[derive(Debug, Clone)]
pub struct MessagePacket {
    /// Serialized message contents.
    pub content_bytes: AlignedVec,
    /// Routing information.
    pub routing: MessageRouting,
}

/// Full routing information for the message.
#[derive(PartialEq, Eq, Debug, Copy, Clone)]
pub struct MessageRouting {
    /// Which participant this message came from.
    pub sender: u64,
    /// Stores the BPCon message type.
    pub msg_type: ProtocolMessage,
}

/// Representation of message types of the consensus.
#[derive(PartialEq, Eq, Debug, Copy, Clone)]
pub enum ProtocolMessage {
    Msg1a,
    Msg1b,
    Msg2a,
    Msg2av,
    Msg2b,
}

impl std::fmt::Display for ProtocolMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ProtocolMessage: {:?}", self)
    }
}

// Value in messages is stored in serialized format, i.e bytes in order to omit
// strict restriction for `Value` trait to be [de]serializable only with `rkyv`.

#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[archive(compare(PartialEq), check_bytes)]
#[archive_attr(derive(Debug))]
pub struct Message1aContent {
    pub ballot: u64,
}

#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[archive(compare(PartialEq), check_bytes)]
#[archive_attr(derive(Debug))]
pub struct Message1bContent {
    pub ballot: u64,
    pub last_ballot_voted: Option<u64>,
    pub last_value_voted: Option<Vec<u8>>,
}

#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[archive(compare(PartialEq), check_bytes)]
#[archive_attr(derive(Debug))]
pub struct Message2aContent {
    pub ballot: u64,
    pub value: Vec<u8>,
}

#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[archive(compare(PartialEq), check_bytes)]
#[archive_attr(derive(Debug))]
pub struct Message2avContent {
    pub ballot: u64,
    pub received_value: Vec<u8>,
}

#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[archive(compare(PartialEq), check_bytes)]
#[archive_attr(derive(Debug))]
pub struct Message2bContent {
    pub ballot: u64,
}

macro_rules! impl_packable {
    ($type:ty, $msg_type:expr) => {
        impl $type {
            pub fn pack(&self, sender: u64) -> Result<MessagePacket, SerializationError> {
                let content_bytes = rkyv::to_bytes::<_, 256>(self)
                    .map_err(|err| SerializationError::Message(err.to_string()))?;
                Ok(MessagePacket {
                    content_bytes,
                    routing: Self::route(sender),
                })
            }

            pub fn unpack(msg: &MessagePacket) -> Result<Self, DeserializationError> {
                let archived = rkyv::check_archived_root::<Self>(msg.content_bytes.as_slice())
                    .map_err(|err| DeserializationError::Message(err.to_string()))?;
                archived
                    .deserialize(&mut Infallible)
                    .map_err(|err| DeserializationError::Message(err.to_string()))
            }

            fn route(sender: u64) -> MessageRouting {
                MessageRouting {
                    sender,
                    msg_type: $msg_type,
                }
            }
        }
    };
}

impl_packable!(Message1aContent, ProtocolMessage::Msg1a);
impl_packable!(Message1bContent, ProtocolMessage::Msg1b);
impl_packable!(Message2aContent, ProtocolMessage::Msg2a);
impl_packable!(Message2avContent, ProtocolMessage::Msg2av);
impl_packable!(Message2bContent, ProtocolMessage::Msg2b);

/// A struct to keep track of senders and the cumulative weight of their messages.
#[derive(PartialEq, Eq, Clone, Debug, Default)]
pub struct MessageRoundState {
    senders: HashSet<u64>,
    weight: u128,
}

impl MessageRoundState {
    /// Creates a new instance of `MessageRoundState`.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_weight(&self) -> u128 {
        self.weight
    }

    /// Adds a sender and their corresponding weight.
    pub fn add_sender(&mut self, sender: u64, weight: u128) {
        self.senders.insert(sender);
        self.weight += weight;
    }

    /// Checks if the sender has already sent a message.
    pub fn contains_sender(&self, sender: &u64) -> bool {
        self.senders.contains(sender)
    }

    /// Resets the state.
    pub fn reset(&mut self) {
        self.senders.clear();
        self.weight = 0;
    }
}

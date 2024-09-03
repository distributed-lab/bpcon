//! Definitions central to BPCon messages.
//!
//! This module defines the structures and functionality for messages used in the BPCon consensus protocol.
//! It includes message contents, routing information, and utilities for packing and unpacking messages.

use crate::error::{DeserializationError, SerializationError};
use rkyv::{AlignedVec, Archive, Deserialize, Infallible, Serialize};
use std::collections::HashSet;

/// A message ready for transfer within the BPCon protocol.
#[derive(Debug, Clone)]
pub struct MessagePacket {
    /// Serialized message contents.
    pub content_bytes: AlignedVec,
    /// Routing information for the message.
    pub routing: MessageRouting,
}

/// Full routing information for a BPCon message.
/// This struct is used to manage and route messages through the BPCon protocol.
#[derive(PartialEq, Eq, Debug, Copy, Clone)]
pub struct MessageRouting {
    /// The ID of the participant who sent the message.
    pub sender: u64,
    /// The type of BPCon protocol message.
    pub msg_type: ProtocolMessage,
}

/// Enumeration of the different types of protocol messages used in BPCon.
///
/// These message types represent the various stages of the BPCon consensus protocol,
/// each corresponding to a specific phase in the process.
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

// The following structures represent the contents of different BPCon protocol messages.
// Each message type is serialized before being sent, and deserialized upon receipt.

// Value in messages is stored in serialized format, i.e., bytes, to avoid strict restrictions
// on the `Value` trait being [de]serializable only with `rkyv`.

/// Contents of a BPCon message of type 1a.
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[archive(compare(PartialEq), check_bytes)]
#[archive_attr(derive(Debug))]
pub struct Message1aContent {
    /// The ballot number associated with this message.
    pub ballot: u64,
}

/// Contents of a BPCon message of type 1b.
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[archive(compare(PartialEq), check_bytes)]
#[archive_attr(derive(Debug))]
pub struct Message1bContent {
    /// The ballot number associated with this message.
    pub ballot: u64,
    /// The last ballot number that was voted on, if any.
    pub last_ballot_voted: Option<u64>,
    /// The last value that was voted on, serialized as a vector of bytes, if any.
    pub last_value_voted: Option<Vec<u8>>,
}

/// Contents of a BPCon message of type 2a.
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[archive(compare(PartialEq), check_bytes)]
#[archive_attr(derive(Debug))]
pub struct Message2aContent {
    /// The ballot number associated with this message.
    pub ballot: u64,
    /// The proposed value, serialized as a vector of bytes.
    pub value: Vec<u8>,
}

/// Contents of a BPCon message of type 2av.
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[archive(compare(PartialEq), check_bytes)]
#[archive_attr(derive(Debug))]
pub struct Message2avContent {
    /// The ballot number associated with this message.
    pub ballot: u64,
    /// The received value, serialized as a vector of bytes.
    pub received_value: Vec<u8>,
}

/// Contents of a BPCon message of type 2b.
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[archive(compare(PartialEq), check_bytes)]
#[archive_attr(derive(Debug))]
pub struct Message2bContent {
    /// The ballot number associated with this message.
    pub ballot: u64,
}

// Macro to implement packing and unpacking logic for each message content type.
// This logic handles serialization to bytes and deserialization from bytes, as well as setting up routing information.

macro_rules! impl_packable {
    ($type:ty, $msg_type:expr) => {
        impl $type {
            /// Packs the message content into a `MessagePacket` for transfer.
            ///
            /// This method serializes the message content and combines it with routing information.
            ///
            /// # Parameters
            ///
            /// - `sender`: The ID of the participant sending the message.
            ///
            /// # Returns
            ///
            /// A `MessagePacket` containing the serialized message and routing information, or a `SerializationError` if packing fails.
            pub fn pack(&self, sender: u64) -> Result<MessagePacket, SerializationError> {
                let content_bytes = rkyv::to_bytes::<_, 256>(self)
                    .map_err(|err| SerializationError::Message(err.to_string()))?;
                Ok(MessagePacket {
                    content_bytes,
                    routing: Self::route(sender),
                })
            }

            /// Unpacks a `MessagePacket` to extract the original message content.
            ///
            /// This method deserializes the message content from the byte representation stored in the packet.
            ///
            /// # Parameters
            ///
            /// - `msg`: A reference to the `MessagePacket` to be unpacked.
            ///
            /// # Returns
            ///
            /// The deserialized message content, or a `DeserializationError` if unpacking fails.
            pub fn unpack(msg: &MessagePacket) -> Result<Self, DeserializationError> {
                let archived = rkyv::check_archived_root::<Self>(msg.content_bytes.as_slice())
                    .map_err(|err| DeserializationError::Message(err.to_string()))?;
                archived
                    .deserialize(&mut Infallible)
                    .map_err(|err| DeserializationError::Message(err.to_string()))
            }

            /// Creates routing information for the message.
            ///
            /// This method sets up the routing details, including the sender and the message type.
            ///
            /// # Parameters
            ///
            /// - `sender`: The ID of the participant sending the message.
            ///
            /// # Returns
            ///
            /// A `MessageRouting` instance containing the routing information.
            fn route(sender: u64) -> MessageRouting {
                MessageRouting {
                    sender,
                    msg_type: $msg_type,
                }
            }
        }
    };
}

// Implement the packing and unpacking functionality for each message content type.
impl_packable!(Message1aContent, ProtocolMessage::Msg1a);
impl_packable!(Message1bContent, ProtocolMessage::Msg1b);
impl_packable!(Message2aContent, ProtocolMessage::Msg2a);
impl_packable!(Message2avContent, ProtocolMessage::Msg2av);
impl_packable!(Message2bContent, ProtocolMessage::Msg2b);

/// A struct to keep track of senders and the cumulative weight of their messages.
///
/// `MessageRoundState` is used to monitor the participants who have sent messages and to accumulate
/// the total weight of these messages. This helps in determining when a quorum has been reached.
#[derive(PartialEq, Eq, Clone, Debug, Default)]
pub struct MessageRoundState {
    senders: HashSet<u64>,
    weight: u128,
}

impl MessageRoundState {
    /// Creates a new, empty `MessageRoundState`.
    ///
    /// This method initializes an empty state with no senders and zero weight.
    ///
    /// # Returns
    ///
    /// A new `MessageRoundState` instance.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the cumulative weight of all messages received in this round.
    ///
    /// # Returns
    ///
    /// The total weight of all messages that have been added to this state.
    pub fn get_weight(&self) -> u128 {
        self.weight
    }

    /// Adds a sender and their corresponding weight to the state.
    ///
    /// This method updates the state to include the specified sender and increments the cumulative
    /// weight by the specified amount.
    ///
    /// # Parameters
    ///
    /// - `sender`: The ID of the participant sending the message.
    /// - `weight`: The weight associated with this sender's message.
    pub fn add_sender(&mut self, sender: u64, weight: u128) {
        self.senders.insert(sender);
        self.weight += weight;
    }

    /// Checks if a sender has already sent a message in this round.
    ///
    /// # Parameters
    ///
    /// - `sender`: A reference to the ID of the participant.
    ///
    /// # Returns
    ///
    /// `true` if the sender has already sent a message, `false` otherwise.
    pub fn contains_sender(&self, sender: &u64) -> bool {
        self.senders.contains(sender)
    }

    /// Resets the state for a new round.
    ///
    /// This method clears all recorded senders and resets the cumulative weight to zero,
    /// preparing the state for a new round of message collection.
    pub fn reset(&mut self) {
        self.senders.clear();
        self.weight = 0;
    }
}

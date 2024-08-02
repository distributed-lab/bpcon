//! Definition of the BPCon messages.

use serde::{Deserialize, Serialize};

/// Message ready for transfer.
pub struct MessageWire {
    /// Serialized message contents.
    pub content_bytes: Vec<u8>,
    /// Routing information.
    pub routing: MessageRouting,
}

/// Full routing information for the message.
pub struct MessageRouting {
    /// Which participant this message came from.
    pub sender: u64,
    /// Where this message should be delivered. Can be empty if `is_broadcast` is `true`
    pub receivers: Vec<u64>,
    /// Indicates whether this message shall be broadcast to other participants.
    pub is_broadcast: bool,
    /// Stores the BPCon message type.
    pub msg_type: ProtocolMessage,
}

/// Representation of message types of the consensus.
pub enum ProtocolMessage {
    Msg1a,
    Msg1b,
    Msg1c,
    Msg2a,
    Msg2av,
    Msg2b,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Message1aContent {}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Message1bContent {}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Message1cContent {}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Message2aContent {}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Message2avContent {}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Message2bContent {}
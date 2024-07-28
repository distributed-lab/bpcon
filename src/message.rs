//! Definition of the BPCon message trait and enum

pub mod msg1a;
pub mod msg1b;
pub mod msg1c;
pub mod msg2a;
pub mod msg2av;
pub mod msg2b;

use serde::{Deserialize, Serialize};

/// Generic communicative unit in ballot.
pub trait Message: Serialize + for<'a> Deserialize<'a> {
    /// Which participant this message came from.
    fn get_sender_id(&self) -> u64;
    /// Where this message should be delivered.
    fn get_receivers_id(&self) -> Vec<u64>;
    /// Indicates whether this message shall be broadcast to other participants. Can be empty of `is_broadcast` is `true`
    fn is_broadcast(&self) -> bool;
    /// Encode inner message to bytes and receive routing information.
    fn msg_routing(&self) -> MessageRouting;
    /// Returns the BPCon message type
    fn msg_type(&self) -> ProtocolMessage;
}

/// Full routing information for the message.
pub struct MessageRouting {
    /// Which participant this message came from.
    pub sender: u64,
    /// Where this message should be delivered. Can be empty of `is_broadcast` is `true`
    pub receivers: Vec<u64>,
    /// Indicates whether this message shall be broadcast to other participants.
    pub is_broadcast: bool,
    /// Stores the BPCon message type
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
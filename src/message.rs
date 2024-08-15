//! Definition of the BPCon messages.

use rkyv::{AlignedVec, Archive, Deserialize, Serialize};

/// Message ready for transfer.
pub struct MessagePacket {
    /// Serialized message contents.
    pub content_bytes: AlignedVec,
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
    Msg2a,
    Msg2av,
    Msg2b,
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

impl Message1aContent {
    pub fn get_routing(id: u64) -> MessageRouting {
        MessageRouting {
            sender: id,
            receivers: vec![],
            is_broadcast: true,
            msg_type: ProtocolMessage::Msg1a,
        }
    }
}

impl Message1bContent {
    pub fn get_routing(id: u64) -> MessageRouting {
        MessageRouting {
            sender: id,
            receivers: vec![],
            is_broadcast: true,
            msg_type: ProtocolMessage::Msg1b,
        }
    }
}

impl Message2aContent {
    pub fn get_routing(id: u64) -> MessageRouting {
        MessageRouting {
            sender: id,
            receivers: vec![],
            is_broadcast: true,
            msg_type: ProtocolMessage::Msg2a,
        }
    }
}

impl Message2avContent {
    pub fn get_routing(id: u64) -> MessageRouting {
        MessageRouting {
            sender: id,
            receivers: vec![],
            is_broadcast: true,
            msg_type: ProtocolMessage::Msg2av,
        }
    }
}

impl Message2bContent {
    pub fn get_routing(id: u64) -> MessageRouting {
        MessageRouting {
            sender: id,
            receivers: vec![],
            is_broadcast: true,
            msg_type: ProtocolMessage::Msg2b,
        }
    }
}

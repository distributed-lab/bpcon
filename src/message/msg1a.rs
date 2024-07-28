//! Definition of the BPCon messages implementation

use serde::{Deserialize, Serialize};
use crate::message::{Message, MessageRouting, ProtocolMessage};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Message1a {}

impl Message for Message1a {
    fn get_sender_id(&self) -> u64 {
        todo!()
    }

    fn get_receivers_id(&self) -> Vec<u64> {
        todo!()
    }

    fn is_broadcast(&self) -> bool {
        todo!()
    }

    fn msg_routing(&self) -> MessageRouting {
        todo!()
    }

    fn msg_type(&self) -> ProtocolMessage {
        todo!()
    }
}

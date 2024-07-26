use crate::Identifier;

/// Generic communicative unit in ballot.
pub trait Message<I: Identifier> {
    /// Which participant this message came from.
    fn get_sender_id(&self) -> I;
    /// Where this message should be delivered.
    fn get_receivers_id(&self) -> Vec<I>;
    /// Indicates whether this message shall be broadcast to other participants.
    fn is_broadcast(&self) -> bool;
    /// Encode inner message to bytes and receive routing information.
    fn wire(&self) -> (Vec<u8>, MessageRouting<I>);
}

/// Full routing information for the message.
pub struct MessageRouting<I: Identifier>{
    /// Which participant this message came from.
    pub sender: I,
    /// Where this message should be delivered.
    pub receivers: Vec<I>,
    /// Indicates whether this message shall be broadcast to other participants.
    pub is_broadcast: bool,
}

/// Representation of message types of the consensus.
pub enum ProtocolMessage{
    Msg1a,
    Msg1b,
    Msg1c,
    Msg2a,
    Msg2av,
    Msg2b,
}
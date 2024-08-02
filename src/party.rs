//! Definition of the BPCon participant structure.

use std::cmp::PartialEq;
use std::sync::mpsc::{channel, Receiver, Sender};
use crate::{Value, ValueSelector};
use crate::error::BallotError;
use crate::message::{Message1aContent, Message1bContent, Message1cContent, Message2aContent, Message2avContent, Message2bContent, MessageRouting, MessageWire, ProtocolMessage};

/// BPCon configuration. Includes ballot time bounds, and other stuff.
pub struct BallotConfig {
    // TODO: define config fields.
}

/// Party status defines the statuses of the ballot for the particular participant
/// depending on local calculations.
#[derive(PartialEq)]
pub(crate) enum PartyStatus {
    None,
    Launched,
    Passed1a,
    Passed1b,
    Passed1c,
    Passed2a,
    Finished,
    Failed,
    Stopped,
}

/// Party events is used for the ballot flow control.
#[derive(PartialEq)]
pub(crate) enum PartyEvent {
    Launch1a,
    Launch1b,
    Launch1c,
    Launch2a,
    Launch2av,
    Launch2b,
    Finalize,
}

/// Party of the BPCon protocol that executes ballot.
///
/// The communication between party and external
/// system is done via `in_receiver` and `out_sender` channels. External system should take
/// care about authentication while receiving incoming messages and then push them to the
/// corresponding `Sender` to the `in_receiver`. Same, it should tke care about listening of new
/// messages in the corresponding `Receiver` to the `out_sender` and submitting them to the
/// corresponding party based on information in `MessageRouting`.
///
/// After finishing of the ballot protocol, party will place the selected value to the
/// `value_sender` or `BallotError` if ballot failed.
pub struct Party<V: Value, VS: ValueSelector<V>> {
    /// This party's identifier.
    pub id: u64,
    /// Other ballot parties' ids.
    pub party_ids: Vec<u64>,

    /// Communication queues.
    msg_in_receiver: Receiver<MessageWire>,
    msg_out_sender: Sender<MessageWire>,

    /// Query to submit result.
    value_sender: Sender<Result<V, BallotError>>,

    /// Query to receive and send events that run ballot protocol
    event_receiver: Receiver<PartyEvent>,
    event_sender: Sender<PartyEvent>,

    /// Ballot config (e.g. ballot time bounds).
    cfg: BallotConfig,

    /// Main functional for value selection.
    value_selector: VS,

    /// Status of the ballot execution
    status: PartyStatus,

    // TODO: define other state fields if needed.
}


impl<V: Value, VS: ValueSelector<V>> Party<V, VS> {
    pub fn new(
        id: u64,
        party_ids: Vec<u64>,
        value_sender: Sender<Result<V, BallotError>>,
        cfg: BallotConfig,
        value_selector: VS,
    ) -> (
        Self,
        Receiver<MessageWire>,
        Sender<MessageWire>
    ) {
        let (event_sender, event_receiver) = channel();
        let (msg_in_sender, msg_in_receiver) = channel();
        let (msg_out_sender, msg_out_receiver) = channel();

        (
            Self {
                id,
                party_ids,
                msg_in_receiver,
                msg_out_sender,
                value_sender,
                event_receiver,
                event_sender,
                cfg,
                value_selector,
                status: PartyStatus::None,
            },
            msg_out_receiver,
            msg_in_sender
        )
    }

    pub fn is_launched(&self) -> bool {
        return !self.is_stopped();
    }

    pub fn is_stopped(&self) -> bool {
        return self.status == PartyStatus::Finished || self.status == PartyStatus::Failed || self.status == PartyStatus::Stopped;
    }

    /// Start the party.
    pub async fn launch(&mut self) {
        while self.is_launched() {
            /// Check for new messages
            if let Ok(msg_wire) = self.msg_in_receiver.try_recv() {
                self.update_state(msg_wire.content_bytes, msg_wire.routing);
            }

            /// Check for new events
            if let Ok(event) = self.event_receiver.try_recv() {
                self.follow_event(event);
            }

            // TODO: emit events to run ballot protocol according to the ballot configuration `BallotConfig`
        }
    }

    /// Update party's state based on message type.
    fn update_state(&mut self, msg: Vec<u8>, routing: MessageRouting) {
        // TODO: Implement according to protocol rules.
        match routing.msg_type {
            ProtocolMessage::Msg1a => {
                if let Ok(msg) = serde_json::from_slice::<Message1aContent>(msg.as_slice()) {}
            }
            ProtocolMessage::Msg1b => {
                if let Ok(msg) = serde_json::from_slice::<Message1bContent>(msg.as_slice()) {}
            }
            ProtocolMessage::Msg1c => {
                if let Ok(msg) = serde_json::from_slice::<Message1cContent>(msg.as_slice()) {}
            }
            ProtocolMessage::Msg2a => {
                if let Ok(msg) = serde_json::from_slice::<Message2aContent>(msg.as_slice()) {}
            }
            ProtocolMessage::Msg2av => {
                if let Ok(msg) = serde_json::from_slice::<Message2avContent>(msg.as_slice()) {}
            }
            ProtocolMessage::Msg2b => {
                if let Ok(msg) = serde_json::from_slice::<Message2bContent>(msg.as_slice()) {}
            }
        }
    }

    /// Executes ballot actions according to the received event.
    fn follow_event(&mut self, event: PartyEvent) {
        // TODO: Implement according to protocol rules.
        match event {
            PartyEvent::Launch1a => {}
            PartyEvent::Launch1b => {}
            PartyEvent::Launch1c => {}
            PartyEvent::Launch2a => {}
            PartyEvent::Launch2av => {}
            PartyEvent::Launch2b => {}
            PartyEvent::Finalize => {}
        }
    }
}

impl<V: Value, VS: ValueSelector<V>> Drop for Party<V, VS> {
    fn drop(&mut self) {
        self.status = PartyStatus::Stopped
    }
}
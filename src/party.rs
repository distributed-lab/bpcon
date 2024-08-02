//! Definition of the BPCon participant structure.

use std::sync::mpsc::{Receiver, Sender};
use crate::{Value, ValueSelector};
use crate::error::BallotError;
use crate::message::{MessageRouting, MessageWire, ProtocolMessage};

/// BPCon configuration. Includes ballot time bounds, and other stuff.
pub struct BallotConfig {
    // TODO: define config fields.
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
    in_receiver: Receiver<MessageWire>,
    out_sender: Sender<MessageWire>,

    /// Query to submit result.
    value_sender: Sender<Result<V, BallotError>>,

    /// Ballot config (e.g. ballot time bounds).
    cfg: BallotConfig,

    /// Main functional for value selection.
    value_selector: VS,

    // TODO: define other state fields if needed.
}

impl<V: Value, VS: ValueSelector<V>> Party<V, VS> {
    pub fn new(
        id: u64,
        party_ids: Vec<u64>,
        in_receiver: Receiver<MessageWire>,
        out_sender: Sender<MessageWire>,
        value_sender: Sender<Result<V, BallotError>>,
        cfg: BallotConfig,
        value_selector: VS,
    ) -> Self {
        Self {
            id,
            party_ids,
            in_receiver,
            out_sender,
            value_sender,
            cfg,
            value_selector,
        }
    }

    /// Start the party.
    pub fn start(&mut self) {
        // TODO: launch party
    }

    /// Update party's state based on message type.
    fn update_state(&mut self, msg: Vec<u8>, routing: MessageRouting) {
        // TODO: Implement according to protocol rules.
        match routing.msg_type {
            ProtocolMessage::Msg1a => {}
            ProtocolMessage::Msg1b => {}
            ProtocolMessage::Msg1c => {}
            ProtocolMessage::Msg2a => {}
            ProtocolMessage::Msg2av => {}
            ProtocolMessage::Msg2b => {}
        }
    }
}

impl<V: Value, VS: ValueSelector<V>> Drop for Party<V, VS> {
    fn drop(&mut self) {
        // TODO: stop party.
    }
}
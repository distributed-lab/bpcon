//! Definition of the BPCon participant structure.

use crate::message::{
    Message1aContent, Message1bContent, Message2aContent, Message2avContent, Message2bContent,
    MessageRouting, MessageWire, ProtocolMessage,
};
use crate::{Value, ValueSelector};
use crate::error::BallotError;
use std::cmp::PartialEq;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use rand::Rng;

/// BPCon configuration. Includes ballot time bounds and other stuff.
pub struct BPConConfig {
    /// Parties weights: `party_weights[i]` corresponds to the i-th party weight
    pub party_weights: Vec<u64>,

    /// Threshold weight to define BFT quorum: should be > 2/3 of total weight
    pub threshold: u128,
    // TODO: define other config fields.
}

/// Party status defines the statuses of the ballot for the particular participant
/// depending on local calculations.
#[derive(PartialEq)]
pub(crate) enum PartyStatus {
    None,
    Launched,
    Passed1a,
    Passed1b,
    Passed2a,
    Passed2av,
    Passed2b,
    Finished,
    Failed,
}

/// Party events is used for the ballot flow control.
#[derive(PartialEq)]
pub(crate) enum PartyEvent {
    Launch1a,
    Launch1b,
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

    /// Communication queues.
    msg_in_receiver: Receiver<MessageWire>,
    msg_out_sender: Sender<MessageWire>,

    /// Query to receive and send events that run ballot protocol
    event_receiver: Receiver<PartyEvent>,
    event_sender: Sender<PartyEvent>,

    /// BPCon config (e.g. ballot time bounds, parties weights, etc.).
    cfg: BPConConfig,

    /// Main functional for value selection.
    value_selector: VS,

    /// Status of the ballot execution
    status: PartyStatus,

    /// Current ballot number
    ballot: u64,

    /// Last ballot where party submitted 2b message
    last_ballot_voted: Option<u64>,

    /// Last value for which party submitted 2b message
    last_value_voted: Option<Vec<u8>>,

    /// Local round fields

    /// 1b round state
    ///
    parties_voted_before: HashMap<u64, Option<Vec<u8>>>, // id <-> value
    messages_1b_weight: u128,

    /// 2a round state
    ///
    value_2a: Option<V>,

    /// 2av round state
    ///
    messages_2av_senders: HashSet<u64>,
    messages_2av_weight: u128,

    /// 2b round state
    ///
    messages_2b_senders: HashSet<u64>,
    messages_2b_weight: u128,
}

impl<V: Value, VS: ValueSelector<V>> Party<V, VS> {
    pub fn new(
        id: u64,
        cfg: BPConConfig,
        value_selector: VS,
    ) -> (Self, Receiver<MessageWire>, Sender<MessageWire>) {
        let (event_sender, event_receiver) = channel();
        let (msg_in_sender, msg_in_receiver) = channel();
        let (msg_out_sender, msg_out_receiver) = channel();

        (
            Self {
                id,
                msg_in_receiver,
                msg_out_sender,
                event_receiver,
                event_sender,
                cfg,
                value_selector,
                status: PartyStatus::None,
                ballot: 0,
                last_ballot_voted: None,
                last_value_voted: None,
                parties_voted_before: HashMap::new(),
                messages_1b_weight: 0,
                value_2a: None,
                messages_2av_senders: HashSet::new(),
                messages_2av_weight: 0,
                messages_2b_senders: HashSet::new(),
                messages_2b_weight: 0,
            },
            msg_out_receiver,
            msg_in_sender,
        )
    }

    pub fn ballot(&self) -> u64 {
        self.ballot
    }

    pub fn is_launched(&self) -> bool {
        !self.is_stopped()
    }

    pub fn is_stopped(&self) -> bool {
        self.status == PartyStatus::Finished || self.status == PartyStatus::Failed
    }

    pub fn get_value_selected(&self) -> Option<V> {
        // Only `Finished` status means reached BFT agreement
        if self.status == PartyStatus::Finished {
            return self.value_2a.clone();
        }

        None
    }

    fn get_leader(&self) -> Result<u64, BallotError> {
        let total_weight: u64 = self.cfg.party_weights.iter().sum();
        if total_weight == 0 {
            return Err(BallotError::LeaderElection);
        }

        let mut rng = rand::thread_rng();
        let random_value: u64 = rng.gen_range(0..total_weight);

        let mut cumulative_weight = 0;
        for (i, &weight) in self.cfg.party_weights.iter().enumerate() {
            cumulative_weight += weight;
            if random_value < cumulative_weight {
                return Ok(i as u64);
            }
        }
        Err(BallotError::LeaderElection)
    }

    fn get_value(&self) -> V {
        self.value_selector.select(&self.parties_voted_before)
    }

    /// Start the next ballot. It's expected from the external system to re-run ballot protocol in
    /// case of failed ballot.
    pub async fn launch_ballot(&mut self) -> Result<Option<V>, BallotError> {
        self.prepare_next_ballot();

        while self.is_launched() {
            if let Ok(msg_wire) = self.msg_in_receiver.try_recv() {
                if let Err(err) = self.update_state(msg_wire.content_bytes, msg_wire.routing) {
                    self.status = PartyStatus::Failed;
                    return Err(err);
                }
            }

            if let Ok(event) = self.event_receiver.try_recv() {
                if let Err(err) = self.follow_event(event) {
                    self.status = PartyStatus::Failed;
                    return Err(err);
                }
            }

            // TODO: Emit events to run ballot protocol according to the ballot configuration
            if self.event_sender.send(PartyEvent::Launch1a).is_err() {
                self.status = PartyStatus::Failed;
                return Err(BallotError::Communication("Failed to send Launch1a event".into()));
            }
            if self.event_sender.send(PartyEvent::Launch1b).is_err() {
                self.status = PartyStatus::Failed;
                return Err(BallotError::Communication("Failed to send Launch1b event".into()));
            }
            if self.event_sender.send(PartyEvent::Launch2a).is_err() {
                self.status = PartyStatus::Failed;
                return Err(BallotError::Communication("Failed to send Launch2a event".into()));
            }
            if self.event_sender.send(PartyEvent::Launch2av).is_err() {
                self.status = PartyStatus::Failed;
                return Err(BallotError::Communication("Failed to send Launch2av event".into()));
            }
            if self.event_sender.send(PartyEvent::Launch2b).is_err() {
                self.status = PartyStatus::Failed;
                return Err(BallotError::Communication("Failed to send Launch2b event".into()));
            }
            if self.event_sender.send(PartyEvent::Finalize).is_err() {
                self.status = PartyStatus::Failed;
                return Err(BallotError::Communication("Failed to send Finalize event".into()));
            }
        }

        Ok(self.get_value_selected())
    }

    /// Prepare state before running a ballot
    fn prepare_next_ballot(&mut self) {
        self.status = PartyStatus::None;
        self.ballot += 1;

        // Clean state
        self.parties_voted_before = HashMap::new();
        self.messages_1b_weight = 0;
        self.value_2a = None;
        self.messages_2av_senders = HashSet::new();
        self.messages_2av_weight = 0;
        self.messages_2b_senders = HashSet::new();
        self.messages_2b_weight = 0;

        // Cleaning channels
        while self.event_receiver.try_recv().is_ok() {}
        while self.msg_in_receiver.try_recv().is_ok() {}

        self.status = PartyStatus::Launched;
    }

    /// Update party's state based on message type.
    fn update_state(&mut self, m: Vec<u8>, routing: MessageRouting) -> Result<(), BallotError> {
        match routing.msg_type {
            ProtocolMessage::Msg1a => {
                let msg: Message1aContent = serde_json::from_slice(m.as_slice())
                    .map_err(|_| BallotError::MessageParsing("Failed to parse Msg1a".into()))?;

                if msg.ballot != self.ballot {
                    return Err(BallotError::InvalidState("Ballot number mismatch in Msg1a".into()));
                }

                if routing.sender != self.get_leader()? {
                    return Err(BallotError::InvalidState("Invalid leader in Msg1a".into()));
                }

                self.status = PartyStatus::Passed1a;
            }
            ProtocolMessage::Msg1b => {
                let msg: Message1bContent = serde_json::from_slice(m.as_slice())
                    .map_err(|_| BallotError::MessageParsing("Failed to parse Msg1b".into()))?;

                if msg.ballot != self.ballot {
                    return Err(BallotError::InvalidState("Ballot number mismatch in Msg1b".into()));
                }

                if let Some(last_ballot_voted) = msg.last_ballot_voted {
                    if last_ballot_voted >= self.ballot {
                        return Err(BallotError::InvalidState("Received outdated 1b message".into()));
                    }
                }

                if let std::collections::hash_map::Entry::Vacant(e) =
                    self.parties_voted_before.entry(routing.sender)
                {
                    e.insert(msg.last_value_voted);
                    self.messages_1b_weight +=
                        self.cfg.party_weights[routing.sender as usize] as u128;

                    if self.messages_1b_weight > self.cfg.threshold {
                        self.status = PartyStatus::Passed1b;
                    }
                }
            }
            ProtocolMessage::Msg2a => {
                let msg: Message2aContent = serde_json::from_slice(m.as_slice())
                    .map_err(|_| BallotError::MessageParsing("Failed to parse Msg2a".into()))?;

                if msg.ballot != self.ballot {
                    return Err(BallotError::InvalidState("Ballot number mismatch in Msg2a".into()));
                }

                if routing.sender != self.get_leader()? {
                    return Err(BallotError::InvalidState("Invalid leader in Msg2a".into()));
                }

                let value_received: V = serde_json::from_slice(msg.value.as_slice())
                    .map_err(|_| BallotError::MessageParsing("Failed to parse value in Msg2a".into()))?;

                if self.value_selector.verify(&value_received, &self.parties_voted_before) {
                    self.status = PartyStatus::Passed2a;
                    self.value_2a = Some(value_received);
                } else {
                    return Err(BallotError::InvalidState("Failed to verify value in Msg2a".into()));
                }
            }
            ProtocolMessage::Msg2av => {
                let msg: Message2avContent = serde_json::from_slice(m.as_slice())
                    .map_err(|_| BallotError::MessageParsing("Failed to parse Msg2av".into()))?;

                if msg.ballot != self.ballot {
                    return Err(BallotError::InvalidState("Ballot number mismatch in Msg2av".into()));
                }

                let value_received: V = serde_json::from_slice(msg.received_value.as_slice())
                    .map_err(|_| BallotError::MessageParsing("Failed to parse value in Msg2av".into()))?;

                if value_received != self.value_2a.clone().unwrap() {
                    return Err(BallotError::InvalidState("Received different value in Msg2av".into()));
                }

                if !self.messages_2av_senders.contains(&routing.sender) {
                    self.messages_2av_senders.insert(routing.sender);
                    self.messages_2av_weight +=
                        self.cfg.party_weights[routing.sender as usize] as u128;

                    if self.messages_2av_weight > self.cfg.threshold {
                        self.status = PartyStatus::Passed2av;
                    }
                }
            }
            ProtocolMessage::Msg2b => {
                let msg: Message2bContent = serde_json::from_slice(m.as_slice())
                    .map_err(|_| BallotError::MessageParsing("Failed to parse Msg2b".into()))?;

                if msg.ballot != self.ballot {
                    return Err(BallotError::InvalidState("Ballot number mismatch in Msg2b".into()));
                }

                if self.messages_2av_senders.contains(&routing.sender)
                    && !self.messages_2b_senders.contains(&routing.sender)
                {
                    self.messages_2b_senders.insert(routing.sender);
                    self.messages_2b_weight +=
                        self.cfg.party_weights[routing.sender as usize] as u128;

                    if self.messages_2b_weight > self.cfg.threshold {
                        self.status = PartyStatus::Passed2b;
                    }
                }
            }
        }
        Ok(())
    }

    /// Executes ballot actions according to the received event.
    fn follow_event(&mut self, event: PartyEvent) -> Result<(), BallotError> {
        match event {
            PartyEvent::Launch1a => {
                if self.status != PartyStatus::Launched {
                    return Err(BallotError::InvalidState("Cannot launch 1a, incorrect state".into()));
                }
                if self.get_leader()? == self.id {
                    self.msg_out_sender.send(MessageWire {
                        content_bytes: serde_json::to_vec(&Message1aContent { ballot: self.ballot })
                            .map_err(|_| BallotError::MessageParsing("Failed to serialize Msg1a".into()))?,
                        routing: Message1aContent::get_routing(self.id),
                    }).map_err(|_| BallotError::Communication("Failed to send Msg1a".into()))?;
                }
            }
            PartyEvent::Launch1b => {
                if self.status != PartyStatus::Passed1a {
                    return Err(BallotError::InvalidState("Cannot launch 1b, incorrect state".into()));
                }
                self.msg_out_sender.send(MessageWire {
                    content_bytes: serde_json::to_vec(&Message1bContent {
                        ballot: self.ballot,
                        last_ballot_voted: self.last_ballot_voted,
                        last_value_voted: self.last_value_voted.clone(),
                    })
                        .map_err(|_| BallotError::MessageParsing("Failed to serialize Msg1b".into()))?,
                    routing: Message1bContent::get_routing(self.id),
                }).map_err(|_| BallotError::Communication("Failed to send Msg1b".into()))?;
            }
            PartyEvent::Launch2a => {
                if self.status != PartyStatus::Passed1b {
                    return Err(BallotError::InvalidState("Cannot launch 2a, incorrect state".into()));
                }
                if self.get_leader()? == self.id {
                    self.msg_out_sender.send(MessageWire {
                        content_bytes: serde_json::to_vec(&Message2aContent {
                            ballot: self.ballot,
                            value: serde_json::to_vec(&self.get_value())
                                .map_err(|_| BallotError::MessageParsing("Failed to serialize value for Msg2a".into()))?,
                        })
                            .map_err(|_| BallotError::MessageParsing("Failed to serialize Msg2a".into()))?,
                        routing: Message2aContent::get_routing(self.id),
                    }).map_err(|_| BallotError::Communication("Failed to send Msg2a".into()))?;
                }
            }
            PartyEvent::Launch2av => {
                if self.status != PartyStatus::Passed2a {
                    return Err(BallotError::InvalidState("Cannot launch 2av, incorrect state".into()));
                }
                self.msg_out_sender.send(MessageWire {
                    content_bytes: serde_json::to_vec(&Message2avContent {
                        ballot: self.ballot,
                        received_value: serde_json::to_vec(&self.value_2a.clone().unwrap())
                            .map_err(|_| BallotError::MessageParsing("Failed to serialize value for Msg2av".into()))?,
                    })
                        .map_err(|_| BallotError::MessageParsing("Failed to serialize Msg2av".into()))?,
                    routing: Message2avContent::get_routing(self.id),
                }).map_err(|_| BallotError::Communication("Failed to send Msg2av".into()))?;
            }
            PartyEvent::Launch2b => {
                if self.status != PartyStatus::Passed2av {
                    return Err(BallotError::InvalidState("Cannot launch 2b, incorrect state".into()));
                }
                self.msg_out_sender.send(MessageWire {
                    content_bytes: serde_json::to_vec(&Message2bContent { ballot: self.ballot })
                        .map_err(|_| BallotError::MessageParsing("Failed to serialize Msg2b".into()))?,
                    routing: Message2bContent::get_routing(self.id),
                }).map_err(|_| BallotError::Communication("Failed to send Msg2b".into()))?;
            }
            PartyEvent::Finalize => {
                if self.status != PartyStatus::Passed2b {
                    return Err(BallotError::InvalidState("Cannot finalize, incorrect state".into()));
                }
                self.status = PartyStatus::Finished;
            }
        }
        Ok(())
    }
}

//! Definition of the BPCon participant structure.

use std::cmp::PartialEq;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use crate::{Value, ValueSelector};
use crate::message::{Message1aContent, Message1bContent, Message2aContent, Message2avContent, Message2bContent, MessageRouting, MessageWire, ProtocolMessage};

/// BPCon configuration. Includes ballot time bounds, and other stuff.
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
    ) -> (
        Self,
        Receiver<MessageWire>,
        Sender<MessageWire>,
    ) {
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
        return self.ballot;
    }

    pub fn is_launched(&self) -> bool {
        return !self.is_stopped();
    }

    pub fn is_stopped(&self) -> bool {
        return self.status == PartyStatus::Finished || self.status == PartyStatus::Failed;
    }

    pub fn get_value_selected(&self) -> Option<V> {
        // Only `Finished` status means reached BFT agreement
        if self.status == PartyStatus::Finished {
            return self.value_2a.clone();
        }

        return None;
    }

    fn get_leader(&self) -> u64 {
        // TODO: implement weight random based on some conditions
        todo!()
    }

    fn get_value(&self) -> V {
        return self.value_selector.select(&self.parties_voted_before);
    }

    /// Start the next ballot. It's expected from the external system to re-run ballot protocol in
    /// case of failed ballot.
    pub async fn launch_ballot(&mut self) -> Option<V> {
        self.prepare_next_ballot();

        while self.is_launched() {
            // Check for new messages
            if let Ok(msg_wire) = self.msg_in_receiver.try_recv() {
                self.update_state(msg_wire.content_bytes, msg_wire.routing);
            }

            // Check for new events
            if let Ok(event) = self.event_receiver.try_recv() {
                self.follow_event(event);
            }

            // TODO: emit events to run ballot protocol according to the ballot configuration `BallotConfig`
            self.event_sender.send(PartyEvent::Launch1a).unwrap();
            self.event_sender.send(PartyEvent::Launch1b).unwrap();
            self.event_sender.send(PartyEvent::Launch2a).unwrap();
            self.event_sender.send(PartyEvent::Launch2av).unwrap();
            self.event_sender.send(PartyEvent::Launch2b).unwrap();
            self.event_sender.send(PartyEvent::Finalize).unwrap();
        }

        return self.get_value_selected();
    }

    /// Prepare state before running a ballot
    fn prepare_next_ballot(&mut self) {
        self.status = PartyStatus::None;
        self.ballot = self.ballot + 1;

        // Clean state
        self.parties_voted_before = HashMap::new();
        self.messages_1b_weight = 0;
        self.value_2a = None;
        self.messages_2av_senders = HashSet::new();
        self.messages_2av_weight = 0;
        self.messages_2b_senders = HashSet::new();
        self.messages_2b_weight = 0;

        // Cleaning channels
        while let Ok(_) = self.event_receiver.try_recv() {}
        while let Ok(_) = self.msg_in_receiver.try_recv() {}

        self.status = PartyStatus::Launched;
    }

    /// Update party's state based on message type.
    fn update_state(&mut self, m: Vec<u8>, routing: MessageRouting) {
        // TODO: Implement according to protocol rules.
        match routing.msg_type {
            ProtocolMessage::Msg1a => {
                if let Ok(msg) = serde_json::from_slice::<Message1aContent>(m.as_slice()) {
                    if msg.ballot != self.ballot {
                        return;
                    }

                    if routing.sender != self.get_leader() {
                        return;
                    }

                    self.status = PartyStatus::Passed1a;
                }
            }
            ProtocolMessage::Msg1b => {
                if let Ok(msg) = serde_json::from_slice::<Message1bContent>(m.as_slice()) {
                    if msg.ballot != self.ballot {
                        return;
                    }

                    if let Some(last_ballot_voted) = msg.last_ballot_voted {
                        if last_ballot_voted >= self.ballot {
                            return;
                        }
                    }

                    if !self.parties_voted_before.contains_key(&routing.sender) {
                        self.parties_voted_before.insert(routing.sender, msg.last_value_voted);
                        self.messages_1b_weight += self.cfg.party_weights[routing.sender as usize] as u128;

                        if self.messages_1b_weight > self.cfg.threshold {
                            self.status = PartyStatus::Passed1b
                        }
                    }
                }
            }
            ProtocolMessage::Msg2a => {
                if let Ok(msg) = serde_json::from_slice::<Message2aContent>(m.as_slice()) {
                    if msg.ballot != self.ballot {
                        return;
                    }

                    if routing.sender != self.get_leader() {
                        return;
                    }

                    if let Ok(value_received) = serde_json::from_slice::<V>(msg.value.as_slice()) {
                        if self.value_selector.verify(&value_received, &self.parties_voted_before) {
                            self.status = PartyStatus::Passed2a;
                            self.value_2a = Some(value_received);
                        }
                    }
                }
            }
            ProtocolMessage::Msg2av => {
                if let Ok(msg) = serde_json::from_slice::<Message2avContent>(m.as_slice()) {
                    if msg.ballot != self.ballot {
                        return;
                    }

                    if let Ok(value_received) = serde_json::from_slice::<V>(msg.received_value.as_slice()) {
                        if value_received != self.value_2a.clone().unwrap() {
                            return;
                        }
                    }

                    if !self.messages_2av_senders.contains(&routing.sender) {
                        self.messages_2av_senders.insert(routing.sender);
                        self.messages_2av_weight += self.cfg.party_weights[routing.sender as usize] as u128;

                        if self.messages_2av_weight > self.cfg.threshold {
                            self.status = PartyStatus::Passed2av
                        }
                    }
                }
            }
            ProtocolMessage::Msg2b => {
                if let Ok(msg) = serde_json::from_slice::<Message2bContent>(m.as_slice()) {
                    if msg.ballot != self.ballot {
                        return;
                    }

                    // Only those who submitted 2av
                    if self.messages_2av_senders.contains(&routing.sender) && !self.messages_2b_senders.contains(&routing.sender) {
                        self.messages_2b_senders.insert(routing.sender);
                        self.messages_2b_weight += self.cfg.party_weights[routing.sender as usize] as u128;

                        if self.messages_2b_weight > self.cfg.threshold {
                            self.status = PartyStatus::Passed2b
                        }
                    }
                }
            }
        }
    }

    /// Executes ballot actions according to the received event.
    fn follow_event(&mut self, event: PartyEvent) {
        // TODO: Implement according to protocol rules.
        match event {
            PartyEvent::Launch1a => {
                if self.status != PartyStatus::Launched {
                    self.status = PartyStatus::Failed;
                    return;
                }

                if self.get_leader() == self.id {
                    self.msg_out_sender.send(MessageWire {
                        content_bytes: serde_json::to_vec(&Message1aContent { ballot: self.ballot }).unwrap(),
                        routing: Message1aContent::get_routing(self.id),
                    }).unwrap();
                }
            }
            PartyEvent::Launch1b => {
                if self.status != PartyStatus::Passed1a {
                    self.status = PartyStatus::Failed;
                    return;
                }

                self.msg_out_sender.send(MessageWire {
                    content_bytes: serde_json::to_vec(&Message1bContent {
                        ballot: self.ballot,
                        last_ballot_voted: self.last_ballot_voted.clone(),
                        last_value_voted: self.last_value_voted.clone(),
                    }).unwrap(),
                    routing: Message1bContent::get_routing(self.id),
                }).unwrap();
            }
            PartyEvent::Launch2a => {
                if self.status != PartyStatus::Passed1b {
                    self.status = PartyStatus::Failed;
                    return;
                }

                if self.get_leader() == self.id {
                    self.msg_out_sender.send(MessageWire {
                        content_bytes: serde_json::to_vec(&Message2aContent {
                            ballot: self.ballot,
                            value: serde_json::to_vec(&self.get_value()).unwrap(),
                        }).unwrap(),
                        routing: Message2aContent::get_routing(self.id),
                    }).unwrap();
                }
            }
            PartyEvent::Launch2av => {
                if self.status != PartyStatus::Passed2a {
                    self.status = PartyStatus::Failed;
                    return;
                }

                self.msg_out_sender.send(MessageWire {
                    content_bytes: serde_json::to_vec(&Message2avContent {
                        ballot: self.ballot,
                        received_value: serde_json::to_vec(&self.value_2a.clone().unwrap()).unwrap(),
                    }).unwrap(),
                    routing: Message2avContent::get_routing(self.id),
                }).unwrap();
            }
            PartyEvent::Launch2b => {
                if self.status != PartyStatus::Passed2av {
                    self.status = PartyStatus::Failed;
                    return;
                }

                self.msg_out_sender.send(MessageWire {
                    content_bytes: serde_json::to_vec(&Message2bContent {
                        ballot: self.ballot,
                    }).unwrap(),
                    routing: Message2bContent::get_routing(self.id),
                }).unwrap();
            }
            PartyEvent::Finalize => {
                if self.status != PartyStatus::Passed2av {
                    self.status = PartyStatus::Failed;
                    return;
                }

                self.status = PartyStatus::Finished;
            }
        }
    }
}

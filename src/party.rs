//! Definition of the BPCon participant structure.

use crate::error::BallotError;
use crate::message::{
    Message1aContent, Message1bContent, Message2aContent, Message2avContent, Message2bContent,
    MessageRouting, MessageWire, ProtocolMessage,
};
use crate::{Value, ValueSelector};
use rand::prelude::StdRng;
use rand::prelude::*;
use rand::Rng;
use std::cmp::PartialEq;
use std::collections::hash_map::DefaultHasher;
use std::collections::hash_map::Entry::Vacant;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::mpsc::{channel, Receiver, Sender};

/// BPCon configuration. Includes ballot time bounds and other stuff.
pub struct BPConConfig {
    /// Parties weights: `party_weights[i]` corresponds to the i-th party weight
    pub party_weights: Vec<u64>,

    /// Threshold weight to define BFT quorum: should be > 2/3 of total weight
    pub threshold: u128,

    /// Leader of the ballot, computed using seed obtained from config.
    leader: u64,
    // TODO: define other config fields.
}

impl BPConConfig {
    /// Create new config instance.
    pub fn new(party_weights: Vec<u64>, threshold: u128) -> Self {
        let mut cfg = Self {
            party_weights,
            threshold,
            leader: 0,
        };
        cfg.leader = cfg.compute_leader().unwrap();

        cfg
    }

    /// Compute leader in a weighed randomized manner.
    /// Uses seed from the config, making it deterministic.
    fn compute_leader(&self) -> Result<u64, BallotError> {
        let seed = self.compute_seed();

        let total_weight: u64 = self.party_weights.iter().sum();
        if total_weight == 0 {
            return Err(BallotError::LeaderElection("Zero weight sum".into()));
        }

        // Use the seed from the config to create a deterministic random number generator.
        let mut rng = StdRng::seed_from_u64(seed);

        let random_value: u64 = rng.gen_range(0..total_weight);

        let mut cumulative_weight = 0;
        for (i, &weight) in self.party_weights.iter().enumerate() {
            cumulative_weight += weight;
            if random_value < cumulative_weight {
                return Ok(i as u64);
            }
        }
        Err(BallotError::LeaderElection("Election failed".into()))
    }

    /// Compute seed for randomized leader election.
    fn compute_seed(&self) -> u64 {
        let mut hasher = DefaultHasher::new();

        // Hash each field that should contribute to the seed
        self.party_weights.hash(&mut hasher);
        self.threshold.hash(&mut hasher);

        // You can add more fields as needed

        // Generate the seed from the hash
        hasher.finish()
    }
}

/// Party status defines the statuses of the ballot for the particular participant
/// depending on local calculations.
#[derive(PartialEq, Debug)]
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

/// A struct to keep track of senders and the cumulative weight of their messages.
struct MessageRoundState {
    senders: HashSet<u64>,
    weight: u128,
}

impl MessageRoundState {
    /// Creates a new instance of `MessageRoundState`.
    fn new() -> Self {
        Self {
            senders: HashSet::new(),
            weight: 0,
        }
    }

    /// Adds a sender and their corresponding weight.
    fn add_sender(&mut self, sender: u64, weight: u128) {
        self.senders.insert(sender);
        self.weight += weight;
    }

    /// Checks if the sender has already sent a message.
    fn contains_sender(&self, sender: &u64) -> bool {
        self.senders.contains(sender)
    }

    /// Resets the state.
    fn reset(&mut self) {
        self.senders.clear();
        self.weight = 0;
    }
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
    messages_2av_state: MessageRoundState,

    /// 2b round state
    ///
    messages_2b_state: MessageRoundState,
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
                messages_2av_state: MessageRoundState::new(),
                messages_2b_state: MessageRoundState::new(),
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
            self.event_sender.send(PartyEvent::Launch1a).map_err(|_| {
                self.status = PartyStatus::Failed;
                BallotError::Communication("Failed to send Launch1a event".into())
            })?;

            self.event_sender.send(PartyEvent::Launch1b).map_err(|_| {
                self.status = PartyStatus::Failed;
                BallotError::Communication("Failed to send Launch1b event".into())
            })?;

            self.event_sender.send(PartyEvent::Launch2a).map_err(|_| {
                self.status = PartyStatus::Failed;
                BallotError::Communication("Failed to send Launch2a event".into())
            })?;

            self.event_sender.send(PartyEvent::Launch2av).map_err(|_| {
                self.status = PartyStatus::Failed;
                BallotError::Communication("Failed to send Launch2av event".into())
            })?;

            self.event_sender.send(PartyEvent::Launch2b).map_err(|_| {
                self.status = PartyStatus::Failed;
                BallotError::Communication("Failed to send Launch2b event".into())
            })?;

            self.event_sender.send(PartyEvent::Finalize).map_err(|_| {
                self.status = PartyStatus::Failed;
                BallotError::Communication("Failed to send Finalize event".into())
            })?;
        }

        Ok(self.get_value_selected())
    }

    /// Prepare state before running a ballot.
    fn prepare_next_ballot(&mut self) {
        self.status = PartyStatus::None;
        self.ballot += 1;

        // Clean state
        self.parties_voted_before = HashMap::new();
        self.messages_1b_weight = 0;
        self.value_2a = None;
        self.messages_2av_state.reset();
        self.messages_2b_state.reset();

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
                    return Err(BallotError::InvalidState(
                        "Ballot number mismatch in Msg1a".into(),
                    ));
                }

                if routing.sender != self.cfg.leader {
                    return Err(BallotError::InvalidState("Invalid leader in Msg1a".into()));
                }

                self.status = PartyStatus::Passed1a;
            }
            ProtocolMessage::Msg1b => {
                let msg: Message1bContent = serde_json::from_slice(m.as_slice())
                    .map_err(|_| BallotError::MessageParsing("Failed to parse Msg1b".into()))?;

                if msg.ballot != self.ballot {
                    return Err(BallotError::InvalidState(
                        "Ballot number mismatch in Msg1b".into(),
                    ));
                }

                if let Some(last_ballot_voted) = msg.last_ballot_voted {
                    if last_ballot_voted >= self.ballot {
                        return Err(BallotError::InvalidState(
                            "Received outdated 1b message".into(),
                        ));
                    }
                }

                if let Vacant(e) = self.parties_voted_before.entry(routing.sender) {
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
                    return Err(BallotError::InvalidState(
                        "Ballot number mismatch in Msg2a".into(),
                    ));
                }

                if routing.sender != self.cfg.leader {
                    return Err(BallotError::InvalidState("Invalid leader in Msg2a".into()));
                }

                let value_received: V =
                    serde_json::from_slice(msg.value.as_slice()).map_err(|_| {
                        BallotError::MessageParsing("Failed to parse value in Msg2a".into())
                    })?;

                if self
                    .value_selector
                    .verify(&value_received, &self.parties_voted_before)
                {
                    self.status = PartyStatus::Passed2a;
                    self.value_2a = Some(value_received);
                } else {
                    return Err(BallotError::InvalidState(
                        "Failed to verify value in Msg2a".into(),
                    ));
                }
            }
            ProtocolMessage::Msg2av => {
                let msg: Message2avContent = serde_json::from_slice(m.as_slice())
                    .map_err(|_| BallotError::MessageParsing("Failed to parse Msg2av".into()))?;

                if msg.ballot != self.ballot {
                    return Err(BallotError::InvalidState(
                        "Ballot number mismatch in Msg2av".into(),
                    ));
                }

                let value_received: V = serde_json::from_slice(msg.received_value.as_slice())
                    .map_err(|_| {
                        BallotError::MessageParsing("Failed to parse value in Msg2av".into())
                    })?;

                if value_received != self.value_2a.clone().unwrap() {
                    return Err(BallotError::InvalidState(
                        "Received different value in Msg2av".into(),
                    ));
                }

                if !self.messages_2av_state.contains_sender(&routing.sender) {
                    self.messages_2av_state.add_sender(
                        routing.sender,
                        self.cfg.party_weights[routing.sender as usize] as u128,
                    );

                    if self.messages_2av_state.weight > self.cfg.threshold {
                        self.status = PartyStatus::Passed2av;
                    }
                }
            }
            ProtocolMessage::Msg2b => {
                let msg: Message2bContent = serde_json::from_slice(m.as_slice())
                    .map_err(|_| BallotError::MessageParsing("Failed to parse Msg2b".into()))?;

                if msg.ballot != self.ballot {
                    return Err(BallotError::InvalidState(
                        "Ballot number mismatch in Msg2b".into(),
                    ));
                }

                if self.messages_2av_state.contains_sender(&routing.sender)
                    && !self.messages_2b_state.contains_sender(&routing.sender)
                {
                    self.messages_2b_state.add_sender(
                        routing.sender,
                        self.cfg.party_weights[routing.sender as usize] as u128,
                    );

                    if self.messages_2b_state.weight > self.cfg.threshold {
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
                    return Err(BallotError::InvalidState(
                        "Cannot launch 1a, incorrect state".into(),
                    ));
                }
                if self.cfg.leader == self.id {
                    self.msg_out_sender
                        .send(MessageWire {
                            content_bytes: serde_json::to_vec(&Message1aContent {
                                ballot: self.ballot,
                            })
                            .map_err(|_| {
                                BallotError::MessageParsing("Failed to serialize Msg1a".into())
                            })?,
                            routing: Message1aContent::get_routing(self.id),
                        })
                        .map_err(|_| BallotError::Communication("Failed to send Msg1a".into()))?;
                }
            }
            PartyEvent::Launch1b => {
                if self.status != PartyStatus::Passed1a {
                    return Err(BallotError::InvalidState(
                        "Cannot launch 1b, incorrect state".into(),
                    ));
                }
                self.msg_out_sender
                    .send(MessageWire {
                        content_bytes: serde_json::to_vec(&Message1bContent {
                            ballot: self.ballot,
                            last_ballot_voted: self.last_ballot_voted,
                            last_value_voted: self.last_value_voted.clone(),
                        })
                        .map_err(|_| {
                            BallotError::MessageParsing("Failed to serialize Msg1b".into())
                        })?,
                        routing: Message1bContent::get_routing(self.id),
                    })
                    .map_err(|_| BallotError::Communication("Failed to send Msg1b".into()))?;
            }
            PartyEvent::Launch2a => {
                if self.status != PartyStatus::Passed1b {
                    return Err(BallotError::InvalidState(
                        "Cannot launch 2a, incorrect state".into(),
                    ));
                }
                if self.cfg.leader == self.id {
                    self.msg_out_sender
                        .send(MessageWire {
                            content_bytes: serde_json::to_vec(&Message2aContent {
                                ballot: self.ballot,
                                value: serde_json::to_vec(&self.get_value()).map_err(|_| {
                                    BallotError::MessageParsing(
                                        "Failed to serialize value for Msg2a".into(),
                                    )
                                })?,
                            })
                            .map_err(|_| {
                                BallotError::MessageParsing("Failed to serialize Msg2a".into())
                            })?,
                            routing: Message2aContent::get_routing(self.id),
                        })
                        .map_err(|_| BallotError::Communication("Failed to send Msg2a".into()))?;
                }
            }
            PartyEvent::Launch2av => {
                if self.status != PartyStatus::Passed2a {
                    return Err(BallotError::InvalidState(
                        "Cannot launch 2av, incorrect state".into(),
                    ));
                }
                self.msg_out_sender
                    .send(MessageWire {
                        content_bytes: serde_json::to_vec(&Message2avContent {
                            ballot: self.ballot,
                            received_value: serde_json::to_vec(&self.value_2a.clone().unwrap())
                                .map_err(|_| {
                                    BallotError::MessageParsing(
                                        "Failed to serialize value for Msg2av".into(),
                                    )
                                })?,
                        })
                        .map_err(|_| {
                            BallotError::MessageParsing("Failed to serialize Msg2av".into())
                        })?,
                        routing: Message2avContent::get_routing(self.id),
                    })
                    .map_err(|_| BallotError::Communication("Failed to send Msg2av".into()))?;
            }
            PartyEvent::Launch2b => {
                if self.status != PartyStatus::Passed2av {
                    return Err(BallotError::InvalidState(
                        "Cannot launch 2b, incorrect state".into(),
                    ));
                }
                self.msg_out_sender
                    .send(MessageWire {
                        content_bytes: serde_json::to_vec(&Message2bContent {
                            ballot: self.ballot,
                        })
                        .map_err(|_| {
                            BallotError::MessageParsing("Failed to serialize Msg2b".into())
                        })?,
                        routing: Message2bContent::get_routing(self.id),
                    })
                    .map_err(|_| BallotError::Communication("Failed to send Msg2b".into()))?;
            }
            PartyEvent::Finalize => {
                if self.status != PartyStatus::Passed2b {
                    return Err(BallotError::InvalidState(
                        "Cannot finalize, incorrect state".into(),
                    ));
                }
                self.status = PartyStatus::Finished;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;

    // Mock implementation of Value
    #[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct MockValue(u64); // Simple mock type wrapping an integer

    impl Value for MockValue {}

    // Mock implementation of ValueSelector
    struct MockValueSelector;

    impl ValueSelector<MockValue> for MockValueSelector {
        fn verify(&self, _v: &MockValue, _m: &HashMap<u64, Option<Vec<u8>>>) -> bool {
            true // For testing, always return true
        }

        fn select(&self, _m: &HashMap<u64, Option<Vec<u8>>>) -> MockValue {
            MockValue(42) // For testing, always return the same value
        }
    }

    #[test]
    fn test_compute_leader_determinism() {
        let party_weights = vec![1, 2, 7]; // Weighted case
        let threshold = 7; // example threshold

        // Initialize the configuration once
        let config = BPConConfig::new(party_weights.clone(), threshold);

        // Compute the leader multiple times
        let leader1 = config.compute_leader().unwrap();
        let leader2 = config.compute_leader().unwrap();
        let leader3 = config.compute_leader().unwrap();

        // All leaders should be the same due to deterministic seed
        assert_eq!(
            leader1, leader2,
            "Leaders should be consistent on repeated calls"
        );
        assert_eq!(
            leader2, leader3,
            "Leaders should be consistent on repeated calls"
        );
    }

    #[test]
    #[should_panic]
    fn test_compute_leader_zero_weights() {
        let party_weights = vec![0, 0, 0];
        let threshold = 1; // example threshold

        // Create the config, which will attempt to compute the leader
        BPConConfig::new(party_weights, threshold);
    }

    #[test]
    fn test_update_state_msg1a() {
        let cfg = BPConConfig {
            party_weights: vec![1, 2, 3],
            threshold: 4,
            leader: 1,
        };
        let mut party = Party::<MockValue, MockValueSelector>::new(0, cfg, MockValueSelector).0;
        party.status = PartyStatus::Launched;
        party.ballot = 1;

        let msg = Message1aContent { ballot: 1 };
        let routing = MessageRouting {
            sender: 1,
            receivers: vec![2, 3],
            is_broadcast: false,
            msg_type: ProtocolMessage::Msg1a,
        };

        let msg_wire = MessageWire {
            content_bytes: serde_json::to_vec(&msg).unwrap(),
            routing,
        };

        party
            .update_state(msg_wire.content_bytes, msg_wire.routing)
            .unwrap();
        assert_eq!(party.status, PartyStatus::Passed1a);
    }

    #[test]
    fn test_update_state_msg1b() {
        let cfg = BPConConfig {
            party_weights: vec![1, 2, 3], // Total weight is 6
            threshold: 4,                 // Threshold is 4
            leader: 1,
        };
        let mut party = Party::<MockValue, MockValueSelector>::new(0, cfg, MockValueSelector).0;
        party.status = PartyStatus::Passed1a;
        party.ballot = 1;

        // First, send a 1b message from party 1 (weight 2)
        let msg1 = Message1bContent {
            ballot: 1,
            last_ballot_voted: Some(0),
            last_value_voted: Some(vec![1, 2, 3]),
        };
        let routing1 = MessageRouting {
            sender: 1, // Party 1 sends the message
            receivers: vec![0],
            is_broadcast: false,
            msg_type: ProtocolMessage::Msg1b,
        };

        let msg_wire1 = MessageWire {
            content_bytes: serde_json::to_vec(&msg1).unwrap(),
            routing: routing1,
        };

        party
            .update_state(msg_wire1.content_bytes, msg_wire1.routing)
            .unwrap();

        // Now, send a 1b message from party 2 (weight 3)
        let msg2 = Message1bContent {
            ballot: 1,
            last_ballot_voted: Some(0),
            last_value_voted: Some(vec![1, 2, 3]),
        };
        let routing2 = MessageRouting {
            sender: 2, // Party 2 sends the message
            receivers: vec![0],
            is_broadcast: false,
            msg_type: ProtocolMessage::Msg1b,
        };

        let msg_wire2 = MessageWire {
            content_bytes: serde_json::to_vec(&msg2).unwrap(),
            routing: routing2,
        };

        party
            .update_state(msg_wire2.content_bytes, msg_wire2.routing)
            .unwrap();

        // After both messages, the cumulative weight is 2 + 3 = 5, which exceeds the threshold
        assert_eq!(party.status, PartyStatus::Passed1b);
    }

    #[test]
    fn test_update_state_msg2a() {
        let cfg = BPConConfig {
            party_weights: vec![1, 2, 3],
            threshold: 4,
            leader: 1,
        };
        let mut party = Party::<MockValue, MockValueSelector>::new(0, cfg, MockValueSelector).0;
        party.status = PartyStatus::Passed1b;
        party.ballot = 1;

        let msg = Message2aContent {
            ballot: 1,
            value: serde_json::to_vec(&MockValue(42)).unwrap(),
        };
        let routing = MessageRouting {
            sender: 1,
            receivers: vec![0],
            is_broadcast: false,
            msg_type: ProtocolMessage::Msg2a,
        };

        let msg_wire = MessageWire {
            content_bytes: serde_json::to_vec(&msg).unwrap(),
            routing,
        };

        party
            .update_state(msg_wire.content_bytes, msg_wire.routing)
            .unwrap();

        assert_eq!(party.status, PartyStatus::Passed2a);
    }

    #[test]
    fn test_update_state_msg2av() {
        let cfg = BPConConfig {
            party_weights: vec![1, 2, 3],
            threshold: 4,
            leader: 1,
        };
        let mut party = Party::<MockValue, MockValueSelector>::new(0, cfg, MockValueSelector).0;
        party.status = PartyStatus::Passed2a;
        party.ballot = 1;
        party.value_2a = Some(MockValue(42));

        // Send first 2av message from party 2 (weight 3)
        let msg1 = Message2avContent {
            ballot: 1,
            received_value: serde_json::to_vec(&MockValue(42)).unwrap(),
        };
        let routing1 = MessageRouting {
            sender: 2,
            receivers: vec![0],
            is_broadcast: false,
            msg_type: ProtocolMessage::Msg2av,
        };

        let msg_wire1 = MessageWire {
            content_bytes: serde_json::to_vec(&msg1).unwrap(),
            routing: routing1,
        };

        party
            .update_state(msg_wire1.content_bytes, msg_wire1.routing)
            .unwrap();

        // Now send a second 2av message from party 1 (weight 2)
        let msg2 = Message2avContent {
            ballot: 1,
            received_value: serde_json::to_vec(&MockValue(42)).unwrap(),
        };
        let routing2 = MessageRouting {
            sender: 1,
            receivers: vec![0],
            is_broadcast: false,
            msg_type: ProtocolMessage::Msg2av,
        };

        let msg_wire2 = MessageWire {
            content_bytes: serde_json::to_vec(&msg2).unwrap(),
            routing: routing2,
        };

        party
            .update_state(msg_wire2.content_bytes, msg_wire2.routing)
            .unwrap();

        // The cumulative weight (3 + 2) should exceed the threshold of 4
        assert_eq!(party.status, PartyStatus::Passed2av);
    }

    #[test]
    fn test_update_state_msg2b() {
        let cfg = BPConConfig {
            party_weights: vec![1, 2, 3],
            threshold: 4,
            leader: 1,
        };
        let mut party = Party::<MockValue, MockValueSelector>::new(0, cfg, MockValueSelector).0;
        party.status = PartyStatus::Passed2av;
        party.ballot = 1;

        // Simulate that both party 2 and party 1 already sent 2av messages
        party.messages_2av_state.add_sender(1, 1);
        party.messages_2av_state.add_sender(2, 2);

        // Send first 2b message from party 2 (weight 3)
        let msg1 = Message2bContent { ballot: 1 };
        let routing1 = MessageRouting {
            sender: 2,
            receivers: vec![0],
            is_broadcast: false,
            msg_type: ProtocolMessage::Msg2b,
        };

        let msg_wire1 = MessageWire {
            content_bytes: serde_json::to_vec(&msg1).unwrap(),
            routing: routing1,
        };

        party
            .update_state(msg_wire1.content_bytes, msg_wire1.routing)
            .unwrap();

        // Print the current state and weight
        println!(
            "After first Msg2b: Status = {:?}, 2b Weight = {}",
            party.status, party.messages_2b_state.weight
        );

        // Now send a second 2b message from party 1 (weight 2)
        let msg2 = Message2bContent { ballot: 1 };
        let routing2 = MessageRouting {
            sender: 1,
            receivers: vec![0],
            is_broadcast: false,
            msg_type: ProtocolMessage::Msg2b,
        };

        let msg_wire2 = MessageWire {
            content_bytes: serde_json::to_vec(&msg2).unwrap(),
            routing: routing2,
        };

        party
            .update_state(msg_wire2.content_bytes, msg_wire2.routing)
            .unwrap();

        // Print the current state and weight
        println!(
            "After second Msg2b: Status = {:?}, 2b Weight = {}",
            party.status, party.messages_2b_state.weight
        );

        // The cumulative weight (3 + 2) should exceed the threshold of 4
        assert_eq!(party.status, PartyStatus::Passed2b);
    }

    #[test]
    fn test_follow_event_launch1a() {
        let cfg = BPConConfig {
            party_weights: vec![1, 2, 3],
            threshold: 4,
            leader: 0,
        };
        let (mut party, _msg_out_receiver, _msg_in_sender) =
            Party::<MockValue, MockValueSelector>::new(0, cfg, MockValueSelector);

        party.status = PartyStatus::Launched;
        party.ballot = 1;

        party
            .follow_event(PartyEvent::Launch1a)
            .expect("Failed to follow Launch1a event");

        // If the party is the leader and in the Launched state, the event should trigger a message.
        assert_eq!(party.status, PartyStatus::Launched); // Status remains Launched, as no state change expected here
    }

    #[test]
    fn test_ballot_reset_after_failure() {
        let cfg = BPConConfig {
            party_weights: vec![1, 2, 3],
            threshold: 4,
            leader: 0,
        };
        let (mut party, _, _) =
            Party::<MockValue, MockValueSelector>::new(0, cfg, MockValueSelector);

        party.status = PartyStatus::Failed;
        party.ballot = 1;

        party.prepare_next_ballot();

        // Check that state has been reset
        assert_eq!(party.status, PartyStatus::Launched);
        assert_eq!(party.ballot, 2); // Ballot number should have incremented
        assert!(party.parties_voted_before.is_empty());
        assert_eq!(party.messages_1b_weight, 0);
        assert!(party.messages_2av_state.senders.is_empty());
        assert_eq!(party.messages_2av_state.weight, 0);
        assert!(party.messages_2b_state.senders.is_empty());
        assert_eq!(party.messages_2b_state.weight, 0);
    }

    #[test]
    fn test_follow_event_communication_failure() {
        let cfg = BPConConfig {
            party_weights: vec![1, 2, 3],
            threshold: 4,
            leader: 0,
        };
        let (mut party, msg_out_receiver, _) =
            Party::<MockValue, MockValueSelector>::new(0, cfg, MockValueSelector);

        party.status = PartyStatus::Launched;
        party.ballot = 1;

        drop(msg_out_receiver); // Drop the receiver to simulate a communication failure

        let result = party.follow_event(PartyEvent::Launch1a);

        match result {
            Err(BallotError::Communication(err_msg)) => {
                assert_eq!(
                    err_msg, "Failed to send Msg1a",
                    "Expected specific communication error message"
                );
            }
            _ => panic!("Expected BallotError::Communication, got {:?}", result),
        }
    }
}

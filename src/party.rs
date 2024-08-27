//! Definition of the BPCon participant structure.

use crate::error::BallotError;
use crate::message::{
    Message1aContent, Message1bContent, Message2aContent, Message2avContent, Message2bContent,
    MessagePacket, MessageRouting, ProtocolMessage,
};
use crate::{Value, ValueSelector};
use rkyv::{AlignedVec, Deserialize, Infallible};
use std::cmp::{Ordering, PartialEq};
use std::collections::hash_map::DefaultHasher;
use std::collections::hash_map::Entry::Vacant;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use seeded_random::{Random, Seed};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::time::{self, Duration};

/// BPCon configuration. Includes ballot time bounds and other stuff.
#[derive(PartialEq, Eq, Debug, Clone)]
pub struct BPConConfig {
    /// Parties weights: `party_weights[i]` corresponds to the i-th party weight
    pub party_weights: Vec<u64>,

    /// Threshold weight to define BFT quorum: should be > 2/3 of total weight
    pub threshold: u128,

    /// Timeout before ballot is launched.
    /// Differs from `launch1a_timeout` having another status and not listening
    /// to external events and messages.
    pub launch_timeout: Duration,

    /// Timeout before 1a stage is launched.
    pub launch1a_timeout: Duration,

    /// Timeout before 1b stage is launched.
    pub launch1b_timeout: Duration,

    /// Timeout before 2a stage is launched.
    pub launch2a_timeout: Duration,

    /// Timeout before 2av stage is launched.
    pub launch2av_timeout: Duration,

    /// Timeout before 2b stage is launched.
    pub launch2b_timeout: Duration,

    /// Timeout before finalization stage is launched.
    pub finalize_timeout: Duration,

    /// Timeout for a graceful period to help parties with latency.
    pub grace_period: Duration,
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
#[derive(PartialEq, Debug)]
pub(crate) enum PartyEvent {
    Launch1a,
    Launch1b,
    Launch2a,
    Launch2av,
    Launch2b,
    Finalize,
}

/// A struct to keep track of senders and the cumulative weight of their messages.
#[derive(PartialEq, Debug)]
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

/// Trait incorporating logic for leader election.
pub trait LeaderElector<V: Value, VS: ValueSelector<V>>: Send {
    /// Get leader for current ballot.
    /// Returns id of the elected party or error.
    fn get_leader(&self, party: &Party<V, VS>) -> Result<u64, BallotError>;
}

pub struct DefaultLeaderElector {}

impl DefaultLeaderElector {
    /// Compute seed for randomized leader election.
    fn compute_seed<V: Value, VS: ValueSelector<V>>(party: &Party<V, VS>) -> u64 {
        let mut hasher = DefaultHasher::new();

        // Hash each field that should contribute to the seed
        party.cfg.party_weights.hash(&mut hasher);
        party.cfg.threshold.hash(&mut hasher);
        party.ballot.hash(&mut hasher);

        // You can add more fields as needed

        // Generate the seed from the hash
        hasher.finish()
    }

    /// Hash the seed to a value within a given range.
    fn hash_to_range(seed: u64, range: u64) -> u64 {
        // Select the `k` suck that value 2^k >= `range` and 2^k is the smallest.
        let mut k = 64;
        while 1u64 << (k - 1) >= range {
            k = k - 1;
        }

        // The following algorithm selects a random u64 value using `ChaCha12Rng`
        // and reduces the result to the k-bits such that 2^k >= `range` the closes power of to the `range`.
        // After we check if the result lies in [0..`range`) or [`range`..2^k).
        // In the first case result is an acceptable value generated uniformly.
        // In the second case we repeat the process again with the incremented iterations counter.
        // Ref: Practical Cryptography 1st Edition by Niels Ferguson, Bruce Schneier, paragraph 10.8
        let rng = Random::from_seed(Seed::unsafe_new(seed));
        loop {
            let mut raw_res: u64 = rng.gen();
            raw_res = (raw_res) >> (64 - k);

            if raw_res < range {
                return raw_res;
            }
            // Executing this loop does not require a large number of iterations.
            // Check tests for more info
        }
    }
}

impl<V: Value, VS: ValueSelector<V>> LeaderElector<V, VS> for DefaultLeaderElector {
    /// Compute leader in a weighed randomized manner.
    /// Uses seed from the config, making it deterministic.
    fn get_leader(&self, party: &Party<V, VS>) -> Result<u64, BallotError> {
        let seed = DefaultLeaderElector::compute_seed(party);

        let total_weight: u64 = party.cfg.party_weights.iter().sum();
        if total_weight == 0 {
            return Err(BallotError::LeaderElection("Zero weight sum".into()));
        }

        // Generate a random number in the range [0, total_weight)
        let random_value = DefaultLeaderElector::hash_to_range(seed, total_weight);

        // Use binary search to find the corresponding participant
        let mut cumulative_weights = vec![0; party.cfg.party_weights.len()];
        cumulative_weights[0] = party.cfg.party_weights[0];

        for i in 1..party.cfg.party_weights.len() {
            cumulative_weights[i] = cumulative_weights[i - 1] + party.cfg.party_weights[i];
        }

        match cumulative_weights.binary_search_by(|&weight| {
            if random_value < weight {
                Ordering::Greater
            } else {
                Ordering::Less
            }
        }) {
            Ok(index) | Err(index) => Ok(index as u64),
        }
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
    msg_in_receiver: UnboundedReceiver<MessagePacket>,
    msg_out_sender: UnboundedSender<MessagePacket>,

    /// Query to receive and send events that run ballot protocol
    event_receiver: UnboundedReceiver<PartyEvent>,
    event_sender: UnboundedSender<PartyEvent>,

    /// BPCon config (e.g. ballot time bounds, parties weights, etc.).
    cfg: BPConConfig,

    /// Main functional for value selection.
    value_selector: VS,

    /// Main functional for leader election.
    elector: Box<dyn LeaderElector<V, VS>>,

    /// Status of the ballot execution
    status: PartyStatus,

    /// Current ballot number
    ballot: u64,

    /// Current ballot leader
    leader: u64,

    /// Last ballot where party submitted 2b message
    last_ballot_voted: Option<u64>,

    /// Last value for which party submitted 2b message
    last_value_voted: Option<V>,

    /// Local round fields

    /// 1b round state
    ///
    parties_voted_before: HashMap<u64, Option<V>>, // id <-> value
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
        elector: Box<dyn LeaderElector<V, VS>>,
    ) -> (
        Self,
        UnboundedReceiver<MessagePacket>,
        UnboundedSender<MessagePacket>,
    ) {
        let (event_sender, event_receiver) = unbounded_channel();
        let (msg_in_sender, msg_in_receiver) = unbounded_channel();
        let (msg_out_sender, msg_out_receiver) = unbounded_channel();

        (
            Self {
                id,
                msg_in_receiver,
                msg_out_sender,
                event_receiver,
                event_sender,
                cfg,
                value_selector,
                elector,
                status: PartyStatus::None,
                ballot: 0,
                leader: 0,
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

    pub async fn launch_ballot(&mut self) -> Result<Option<V>, BallotError> {
        self.prepare_next_ballot()?;
        time::sleep(self.cfg.launch_timeout).await;

        self.status = PartyStatus::Launched;

        let launch1a_timer = time::sleep(self.cfg.launch1a_timeout);
        let launch1b_timer = time::sleep(self.cfg.launch1b_timeout);
        let launch2a_timer = time::sleep(self.cfg.launch2a_timeout);
        let launch2av_timer = time::sleep(self.cfg.launch2av_timeout);
        let launch2b_timer = time::sleep(self.cfg.launch2b_timeout);
        let finalize_timer = time::sleep(self.cfg.finalize_timeout);

        tokio::pin!(
            launch1a_timer,
            launch1b_timer,
            launch2a_timer,
            launch2av_timer,
            launch2b_timer,
            finalize_timer
        );

        let mut launch1a_fired = false;
        let mut launch1b_fired = false;
        let mut launch2a_fired = false;
        let mut launch2av_fired = false;
        let mut launch2b_fired = false;
        let mut finalize_fired = false;

        while self.is_launched() {
            tokio::select! {
                _ = &mut launch1a_timer, if !launch1a_fired => {
                    self.event_sender.send(PartyEvent::Launch1a).map_err(|_| {
                        self.status = PartyStatus::Failed;
                        BallotError::Communication("Failed to send Launch1a event".into())
                    })?;
                    launch1a_fired = true;
                },
                _ = &mut launch1b_timer, if !launch1b_fired => {
                    self.event_sender.send(PartyEvent::Launch1b).map_err(|_| {
                        self.status = PartyStatus::Failed;
                        BallotError::Communication("Failed to send Launch1b event".into())
                    })?;
                    launch1b_fired = true;
                },
                _ = &mut launch2a_timer, if !launch2a_fired => {
                    self.event_sender.send(PartyEvent::Launch2a).map_err(|_| {
                        self.status = PartyStatus::Failed;
                        BallotError::Communication("Failed to send Launch2a event".into())
                    })?;
                    launch2a_fired = true;
                },
                _ = &mut launch2av_timer, if !launch2av_fired => {
                    self.event_sender.send(PartyEvent::Launch2av).map_err(|_| {
                        self.status = PartyStatus::Failed;
                        BallotError::Communication("Failed to send Launch2av event".into())
                    })?;
                    launch2av_fired = true;
                },
                _ = &mut launch2b_timer, if !launch2b_fired => {
                    self.event_sender.send(PartyEvent::Launch2b).map_err(|_| {
                        self.status = PartyStatus::Failed;
                        BallotError::Communication("Failed to send Launch2b event".into())
                    })?;
                    launch2b_fired = true;
                },
                _ = &mut finalize_timer, if !finalize_fired => {
                    self.event_sender.send(PartyEvent::Finalize).map_err(|_| {
                        self.status = PartyStatus::Failed;
                        BallotError::Communication("Failed to send Finalize event".into())
                    })?;
                    finalize_fired = true;
                },
                msg_wire = self.msg_in_receiver.recv() => {
                    tokio::time::sleep(self.cfg.grace_period).await;
                    if let Some(msg_wire) = msg_wire {
                        if let Err(err) = self.update_state(msg_wire.content_bytes, msg_wire.routing) {
                            self.status = PartyStatus::Failed;
                            return Err(err);
                        }
                    }else if self.msg_in_receiver.is_closed(){
                         self.status = PartyStatus::Failed;
                         return Err(BallotError::Communication("msg-in channel closed".into()));
                    }
                },
                event = self.event_receiver.recv() => {
                    tokio::time::sleep(self.cfg.grace_period).await;
                    if let Some(event) = event {
                        if let Err(err) = self.follow_event(event) {
                            self.status = PartyStatus::Failed;
                            return Err(err);
                        }
                    }else if self.event_receiver.is_closed(){
                        self.status = PartyStatus::Failed;
                        return Err(BallotError::Communication("event receiver channel closed".into()));
                    }
                },
            }
        }

        Ok(self.get_value_selected())
    }

    /// Prepare state before running a ballot.
    fn prepare_next_ballot(&mut self) -> Result<(), BallotError> {
        self.status = PartyStatus::None;
        self.ballot += 1;
        self.leader = self.elector.get_leader(self)?;

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
        Ok(())
    }

    /// Update party's state based on message type.
    fn update_state(&mut self, m: AlignedVec, routing: MessageRouting) -> Result<(), BallotError> {
        match routing.msg_type {
            ProtocolMessage::Msg1a => {
                if self.status != PartyStatus::Launched {
                    return Err(BallotError::InvalidState(
                        "Received Msg1a message, while party status is not <Launched>".into(),
                    ));
                }
                let archived =
                    rkyv::check_archived_root::<Message1aContent>(&m[..]).map_err(|err| {
                        BallotError::MessageParsing(format!("Validation error: {:?}", err))
                    })?;

                let msg: Message1aContent =
                    archived.deserialize(&mut Infallible).map_err(|err| {
                        BallotError::MessageParsing(format!("Deserialization error: {:?}", err))
                    })?;

                if msg.ballot != self.ballot {
                    return Err(BallotError::InvalidState(
                        "Ballot number mismatch in Msg1a".into(),
                    ));
                }

                if routing.sender != self.leader {
                    return Err(BallotError::InvalidState("Invalid leader in Msg1a".into()));
                }

                self.status = PartyStatus::Passed1a;
            }
            ProtocolMessage::Msg1b => {
                if self.status != PartyStatus::Passed1a {
                    return Err(BallotError::InvalidState(
                        "Received Msg1b message, while party status is not <Passed1a>".into(),
                    ));
                }
                let archived =
                    rkyv::check_archived_root::<Message1bContent>(&m[..]).map_err(|err| {
                        BallotError::MessageParsing(format!("Validation error: {:?}", err))
                    })?;

                let msg: Message1bContent =
                    archived.deserialize(&mut Infallible).map_err(|err| {
                        BallotError::MessageParsing(format!("Deserialization error: {:?}", err))
                    })?;

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
                    let value: Option<V> = match msg.last_value_voted {
                        Some(ref data) => Some(bincode::deserialize(data).map_err(|err| {
                            BallotError::ValueParsing(format!("Deserialization error: {:?}", err))
                        })?),
                        None => None,
                    };

                    e.insert(value);

                    self.messages_1b_weight +=
                        self.cfg.party_weights[routing.sender as usize] as u128;

                    if self.messages_1b_weight > self.cfg.threshold {
                        self.status = PartyStatus::Passed1b;
                    }
                }
            }
            ProtocolMessage::Msg2a => {
                if self.status != PartyStatus::Passed1b {
                    return Err(BallotError::InvalidState(
                        "Received Msg2a message, while party status is not <Passed1b>".into(),
                    ));
                }
                let archived =
                    rkyv::check_archived_root::<Message2aContent>(&m[..]).map_err(|err| {
                        BallotError::MessageParsing(format!("Validation error: {:?}", err))
                    })?;

                let msg: Message2aContent =
                    archived.deserialize(&mut Infallible).map_err(|err| {
                        BallotError::MessageParsing(format!("Deserialization error: {:?}", err))
                    })?;

                if msg.ballot != self.ballot {
                    return Err(BallotError::InvalidState(
                        "Ballot number mismatch in Msg2a".into(),
                    ));
                }

                if routing.sender != self.leader {
                    return Err(BallotError::InvalidState("Invalid leader in Msg2a".into()));
                }

                let value_received = bincode::deserialize(&msg.value[..]).map_err(|err| {
                    BallotError::ValueParsing(format!("Failed to parse value in Msg2a: {:?}", err))
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
                if self.status != PartyStatus::Passed2a {
                    return Err(BallotError::InvalidState(
                        "Received Msg2av message, while party status is not <Passed2a>".into(),
                    ));
                }
                let archived =
                    rkyv::check_archived_root::<Message2avContent>(&m[..]).map_err(|err| {
                        BallotError::MessageParsing(format!("Validation error: {:?}", err))
                    })?;

                let msg: Message2avContent =
                    archived.deserialize(&mut Infallible).map_err(|err| {
                        BallotError::MessageParsing(format!("Deserialization error: {:?}", err))
                    })?;

                if msg.ballot != self.ballot {
                    return Err(BallotError::InvalidState(
                        "Ballot number mismatch in Msg2av".into(),
                    ));
                }
                let value_received: V =
                    bincode::deserialize(&msg.received_value[..]).map_err(|err| {
                        BallotError::ValueParsing(format!(
                            "Failed to parse value in Msg2av: {:?}",
                            err
                        ))
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
                if self.status != PartyStatus::Passed2av {
                    return Err(BallotError::InvalidState(
                        "Received Msg2b message, while party status is not <Passed2av>".into(),
                    ));
                }
                let archived =
                    rkyv::check_archived_root::<Message2bContent>(&m[..]).map_err(|err| {
                        BallotError::MessageParsing(format!("Validation error: {:?}", err))
                    })?;

                let msg: Message2bContent =
                    archived.deserialize(&mut Infallible).map_err(|err| {
                        BallotError::MessageParsing(format!("Deserialization error: {:?}", err))
                    })?;

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
                if self.leader == self.id {
                    self.msg_out_sender
                        .send(MessagePacket {
                            content_bytes: rkyv::to_bytes::<_, 256>(&Message1aContent {
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
                    .send(MessagePacket {
                        content_bytes: rkyv::to_bytes::<_, 256>(&Message1bContent {
                            ballot: self.ballot,
                            last_ballot_voted: self.last_ballot_voted,
                            last_value_voted: self
                                .last_value_voted
                                .clone()
                                .map(|inner_data| {
                                    bincode::serialize(&inner_data).map_err(|_| {
                                        BallotError::ValueParsing(
                                            "Failed to serialize value".into(),
                                        )
                                    })
                                })
                                .transpose()?,
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
                if self.leader == self.id {
                    self.msg_out_sender
                        .send(MessagePacket {
                            content_bytes: rkyv::to_bytes::<_, 256>(&Message2aContent {
                                ballot: self.ballot,
                                value: bincode::serialize(&self.get_value()).map_err(|_| {
                                    BallotError::ValueParsing("Failed to serialize value".into())
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
                    .send(MessagePacket {
                        content_bytes: rkyv::to_bytes::<_, 256>(&Message2avContent {
                            ballot: self.ballot,
                            received_value: bincode::serialize(&self.value_2a.clone()).map_err(
                                |_| BallotError::ValueParsing("Failed to serialize value".into()),
                            )?,
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
                    .send(MessagePacket {
                        content_bytes: rkyv::to_bytes::<_, 256>(&Message2bContent {
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

    use std::collections::HashMap;
    use std::thread;
    use rand::Rng;

    // Mock implementation of Value
    #[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    struct MockValue(u64); // Simple mock type wrapping an integer

    impl Value for MockValue {}

    // Mock implementation of ValueSelector
    struct MockValueSelector;

    impl ValueSelector<MockValue> for MockValueSelector {
        fn verify(&self, _v: &MockValue, _m: &HashMap<u64, Option<MockValue>>) -> bool {
            true // For testing, always return true
        }

        fn select(&self, _m: &HashMap<u64, Option<MockValue>>) -> MockValue {
            MockValue(42) // For testing, always return the same value
        }
    }

    fn default_config() -> BPConConfig {
        BPConConfig {
            party_weights: vec![1, 2, 3],
            threshold: 4,
            launch_timeout: Duration::from_secs(0),
            launch1a_timeout: Duration::from_secs(0),
            launch1b_timeout: Duration::from_secs(10),
            launch2a_timeout: Duration::from_secs(20),
            launch2av_timeout: Duration::from_secs(30),
            launch2b_timeout: Duration::from_secs(40),
            finalize_timeout: Duration::from_secs(50),
            grace_period: Duration::from_secs(1),
        }
    }

    #[test]
    fn test_compute_leader_determinism() {
        let cfg = default_config();
        let party = Party::<MockValue, MockValueSelector>::new(
            0,
            cfg,
            MockValueSelector,
            Box::new(DefaultLeaderElector {}),
        )
        .0;

        // Compute the leader multiple times
        let leader1 = party.elector.get_leader(&party).unwrap();
        let leader2 = party.elector.get_leader(&party).unwrap();
        let leader3 = party.elector.get_leader(&party).unwrap();

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
    fn test_compute_leader_zero_weights() {
        let mut cfg = default_config();
        cfg.party_weights = vec![0, 0, 0];

        let party = Party::<MockValue, MockValueSelector>::new(
            0,
            cfg,
            MockValueSelector,
            Box::new(DefaultLeaderElector {}),
        )
        .0;

        match party.elector.get_leader(&party) {
            Err(BallotError::LeaderElection(_)) => {
                // The test passes if the error is of type LeaderElection
            }
            _ => panic!("Expected BallotError::LeaderElection"),
        }
    }

    #[test]
    fn test_update_state_msg1a() {
        let cfg = default_config();
        let mut party = Party::<MockValue, MockValueSelector>::new(
            0,
            cfg,
            MockValueSelector,
            Box::new(DefaultLeaderElector {}),
        )
        .0;
        party.status = PartyStatus::Launched;
        party.ballot = 1;

        // Must send this message from leader of the ballot.
        let leader = party.elector.get_leader(&party).unwrap();

        let msg = Message1aContent { ballot: 1 };
        let routing = MessageRouting {
            sender: leader,
            receivers: vec![2, 3],
            is_broadcast: false,
            msg_type: ProtocolMessage::Msg1a,
        };

        let msg_wire = MessagePacket {
            content_bytes: rkyv::to_bytes::<_, 256>(&msg).unwrap(),
            routing,
        };

        party
            .update_state(msg_wire.content_bytes, msg_wire.routing)
            .unwrap();
        assert_eq!(party.status, PartyStatus::Passed1a);
    }

    #[test]
    fn test_update_state_msg1b() {
        let cfg = default_config();
        let mut party = Party::<MockValue, MockValueSelector>::new(
            0,
            cfg,
            MockValueSelector,
            Box::new(DefaultLeaderElector {}),
        )
        .0;
        party.status = PartyStatus::Passed1a;
        party.ballot = 1;

        // First, send a 1b message from party 1 (weight 2)
        let msg1 = Message1bContent {
            ballot: 1,
            last_ballot_voted: Some(0),
            last_value_voted: bincode::serialize(&MockValue(42)).ok(),
        };
        let routing1 = MessageRouting {
            sender: 1, // Party 1 sends the message
            receivers: vec![0],
            is_broadcast: false,
            msg_type: ProtocolMessage::Msg1b,
        };

        let msg_wire1 = MessagePacket {
            content_bytes: rkyv::to_bytes::<_, 256>(&msg1).unwrap(),
            routing: routing1,
        };

        party
            .update_state(msg_wire1.content_bytes, msg_wire1.routing)
            .unwrap();

        // Now, send a 1b message from party 2 (weight 3)
        let msg2 = Message1bContent {
            ballot: 1,
            last_ballot_voted: Some(0),
            last_value_voted: bincode::serialize(&MockValue(42)).ok(),
        };
        let routing2 = MessageRouting {
            sender: 2, // Party 2 sends the message
            receivers: vec![0],
            is_broadcast: false,
            msg_type: ProtocolMessage::Msg1b,
        };

        let msg_wire2 = MessagePacket {
            content_bytes: rkyv::to_bytes::<_, 256>(&msg2).unwrap(),
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
        let cfg = default_config();
        let mut party = Party::<MockValue, MockValueSelector>::new(
            0,
            cfg,
            MockValueSelector,
            Box::new(DefaultLeaderElector {}),
        )
        .0;
        party.status = PartyStatus::Passed1b;
        party.ballot = 1;

        // Must send this message from leader of the ballot.
        let leader = party.elector.get_leader(&party).unwrap();

        let msg = Message2aContent {
            ballot: 1,
            value: bincode::serialize(&MockValue(42)).unwrap(),
        };
        let routing = MessageRouting {
            sender: leader,
            receivers: vec![0],
            is_broadcast: false,
            msg_type: ProtocolMessage::Msg2a,
        };

        let msg_wire = MessagePacket {
            content_bytes: rkyv::to_bytes::<_, 256>(&msg).unwrap(),
            routing,
        };

        party
            .update_state(msg_wire.content_bytes, msg_wire.routing)
            .unwrap();

        assert_eq!(party.status, PartyStatus::Passed2a);
    }

    #[test]
    fn test_update_state_msg2av() {
        let cfg = default_config();
        let mut party = Party::<MockValue, MockValueSelector>::new(
            0,
            cfg,
            MockValueSelector,
            Box::new(DefaultLeaderElector {}),
        )
        .0;
        party.status = PartyStatus::Passed2a;
        party.ballot = 1;
        party.value_2a = Some(MockValue(42));

        // Send first 2av message from party 2 (weight 3)
        let msg1 = Message2avContent {
            ballot: 1,
            received_value: bincode::serialize(&MockValue(42)).unwrap(),
        };
        let routing1 = MessageRouting {
            sender: 2,
            receivers: vec![0],
            is_broadcast: false,
            msg_type: ProtocolMessage::Msg2av,
        };

        let msg_wire1 = MessagePacket {
            content_bytes: rkyv::to_bytes::<_, 256>(&msg1).unwrap(),
            routing: routing1,
        };

        party
            .update_state(msg_wire1.content_bytes, msg_wire1.routing)
            .unwrap();

        // Now send a second 2av message from party 1 (weight 2)
        let msg2 = Message2avContent {
            ballot: 1,
            received_value: bincode::serialize(&MockValue(42)).unwrap(),
        };
        let routing2 = MessageRouting {
            sender: 1,
            receivers: vec![0],
            is_broadcast: false,
            msg_type: ProtocolMessage::Msg2av,
        };

        let msg_wire2 = MessagePacket {
            content_bytes: rkyv::to_bytes::<_, 256>(&msg2).unwrap(),
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
        let cfg = default_config();
        let mut party = Party::<MockValue, MockValueSelector>::new(
            0,
            cfg,
            MockValueSelector,
            Box::new(DefaultLeaderElector {}),
        )
        .0;
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

        let msg_wire1 = MessagePacket {
            content_bytes: rkyv::to_bytes::<_, 256>(&msg1).unwrap(),
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

        let msg_wire2 = MessagePacket {
            content_bytes: rkyv::to_bytes::<_, 256>(&msg2).unwrap(),
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
        let cfg = default_config();
        let (mut party, _msg_out_receiver, _msg_in_sender) =
            Party::<MockValue, MockValueSelector>::new(
                0,
                cfg,
                MockValueSelector,
                Box::new(DefaultLeaderElector {}),
            );

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
        let cfg = default_config();
        let (mut party, _, _) = Party::<MockValue, MockValueSelector>::new(
            0,
            cfg,
            MockValueSelector,
            Box::new(DefaultLeaderElector {}),
        );

        party.status = PartyStatus::Failed;
        party.ballot = 1;

        party.prepare_next_ballot().unwrap();

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
        let cfg = default_config();

        // This party id is precomputed for this specific party_weights, threshold and ballot.
        // Because we need leader to send 1a.
        let party_id = 0;

        let (mut party, msg_out_receiver, _) = Party::<MockValue, MockValueSelector>::new(
            party_id,
            cfg,
            MockValueSelector,
            Box::new(DefaultLeaderElector {}),
        );

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

    #[tokio::test]
    async fn test_launch_ballot_events() {
        // Pause the Tokio time so we can manipulate it
        time::pause();

        // Set up the Party with necessary configuration
        let cfg = default_config();
        let (event_sender, mut event_receiver) = unbounded_channel();

        // Need to return all 3 values, so that they don't get dropped
        // and associated channels don't get closed.
        let (mut party, _msg_out_receiver, _msg_in_sender) =
            Party::<MockValue, MockValueSelector>::new(
                0,
                cfg.clone(),
                MockValueSelector,
                Box::new(DefaultLeaderElector {}),
            );

        // Same here, we would like to not lose party's event_receiver, so that test doesn't fail.
        let _event_sender = party.event_sender;
        party.event_sender = event_sender;

        // Spawn the launch_ballot function in a separate task
        let _ballot_task = tokio::spawn(async move {
            party.launch_ballot().await.unwrap();
        });

        // Sequential time advance and event check
        time::advance(cfg.launch1a_timeout).await;
        assert_eq!(event_receiver.recv().await.unwrap(), PartyEvent::Launch1a);

        time::advance(cfg.launch1b_timeout - cfg.launch1a_timeout).await;
        assert_eq!(event_receiver.recv().await.unwrap(), PartyEvent::Launch1b);

        time::advance(cfg.launch2a_timeout - cfg.launch1b_timeout).await;
        assert_eq!(event_receiver.recv().await.unwrap(), PartyEvent::Launch2a);

        time::advance(cfg.launch2av_timeout - cfg.launch2a_timeout).await;
        assert_eq!(event_receiver.recv().await.unwrap(), PartyEvent::Launch2av);

        time::advance(cfg.launch2b_timeout - cfg.launch2av_timeout).await;
        assert_eq!(event_receiver.recv().await.unwrap(), PartyEvent::Launch2b);

        time::advance(cfg.finalize_timeout - cfg.launch2b_timeout).await;
        assert_eq!(event_receiver.recv().await.unwrap(), PartyEvent::Finalize);
    }

    use seeded_random::{Random, Seed};

    fn debug_hash_to_range_new(seed: u64, range: u64) -> u64 {
        assert!(range > 1);

        let mut k = 64;
        while 1u64 << (k - 1) >= range {
            k = k - 1;
        }

        let rng = Random::from_seed(Seed::unsafe_new(seed));

        let mut iteration = 1u64;
        loop {
            let mut raw_res: u64 = rng.gen();
            raw_res = (raw_res) >> (64 - k);

            if raw_res < range {
                return raw_res;
            }

            iteration = iteration + 1;
            assert!(iteration <= 50)
        }
    }
    #[test]
    fn test_hash_range_random() {
        // test the uniform distribution

        const N: usize = 37;
        const M: i64 = 10000000;

        let mut cnt1: [i64; N] = [0; N];


        for i in 0..M {
            let mut rng = rand::thread_rng();
            let seed: u64 = rng.gen();

            let res1 = debug_hash_to_range_new(seed, N as u64);
            assert!(res1 < N as u64);

            cnt1[res1 as usize] += 1;
        }


        println!("1: {:?}", cnt1);

        let mut avg1: i64 = 0;

        for i in 0..N {
            avg1 += (M / (N as i64) - cnt1[i]).abs();
        }

        avg1 = avg1 / (N as i64);

        println!("Avg 1: {}", avg1);
    }

    #[test]
    fn test_rng() {
        let rng1 = Random::from_seed(Seed::unsafe_new(123456));
        let rng2 = Random::from_seed(Seed::unsafe_new(123456));

        println!("{}", rng1.gen::<u64>() as u64);
        println!("{}", rng2.gen::<u64>() as u64);

        thread::sleep(Duration::from_secs(2));

        println!("{}", rng1.gen::<u64>() as u64);
        println!("{}", rng2.gen::<u64>() as u64);
    }
}

//! Definition of the BPCon participant structure.

use crate::config::BPConConfig;
use crate::error::FollowEventError::FailedToSendMessage;
use crate::error::LaunchBallotError::{
    EventChannelClosed, FailedToSendEvent, LeaderElectionError, MessageChannelClosed,
};
use crate::error::UpdateStateError::ValueVerificationFailed;
use crate::error::{
    BallotNumberMismatch, DeserializationError, FollowEventError, LaunchBallotError,
    LeaderMismatch, PartyStatusMismatch, SerializationError, UpdateStateError, ValueMismatch,
};
use crate::leader::LeaderElector;
use crate::message::{
    Message1aContent, Message1bContent, Message2aContent, Message2avContent, Message2bContent,
    MessagePacket, MessageRoundState, ProtocolMessage,
};
use crate::{Value, ValueSelector};
use log::{debug, warn};
use std::cmp::PartialEq;
use std::collections::hash_map::Entry::Vacant;
use std::collections::HashMap;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::time::sleep;

/// Party status defines the statuses of the ballot for the particular participant
/// depending on local calculations.
#[derive(PartialEq, Eq, Debug, Copy, Clone)]
pub enum PartyStatus {
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

impl std::fmt::Display for PartyStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PartyStatus: {:?}", self)
    }
}

/// Party events is used for the ballot flow control.
#[derive(PartialEq, Eq, Debug, Copy, Clone)]
pub enum PartyEvent {
    Launch1a,
    Launch1b,
    Launch2a,
    Launch2av,
    Launch2b,
    Finalize,
}

impl std::fmt::Display for PartyEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PartyEvent: {:?}", self)
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
    pub(crate) cfg: BPConConfig,

    /// Main functional for value selection.
    value_selector: VS,

    /// Main functional for leader election.
    elector: Box<dyn LeaderElector<V, VS>>,

    /// Status of the ballot execution
    status: PartyStatus,

    /// Current ballot number
    pub(crate) ballot: u64,

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

    pub async fn launch_ballot(&mut self) -> Result<Option<V>, LaunchBallotError> {
        self.prepare_next_ballot()?;

        sleep(self.cfg.launch_timeout).await;

        let launch1a_timer = sleep(self.cfg.launch1a_timeout);
        let launch1b_timer = sleep(self.cfg.launch1b_timeout);
        let launch2a_timer = sleep(self.cfg.launch2a_timeout);
        let launch2av_timer = sleep(self.cfg.launch2av_timeout);
        let launch2b_timer = sleep(self.cfg.launch2b_timeout);
        let finalize_timer = sleep(self.cfg.finalize_timeout);

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
                    self.event_sender.send(PartyEvent::Launch1a).map_err(|err| {
                        self.status = PartyStatus::Failed;
                        FailedToSendEvent(PartyEvent::Launch1a, err.to_string())
                    })?;
                    launch1a_fired = true;
                },
                _ = &mut launch1b_timer, if !launch1b_fired => {
                    self.event_sender.send(PartyEvent::Launch1b).map_err(|err| {
                        self.status = PartyStatus::Failed;
                        FailedToSendEvent(PartyEvent::Launch1b, err.to_string())
                    })?;
                    launch1b_fired = true;
                },
                _ = &mut launch2a_timer, if !launch2a_fired => {
                    self.event_sender.send(PartyEvent::Launch2a).map_err(|err| {
                        self.status = PartyStatus::Failed;
                        FailedToSendEvent(PartyEvent::Launch2a, err.to_string())
                    })?;
                    launch2a_fired = true;
                },
                _ = &mut launch2av_timer, if !launch2av_fired => {
                    self.event_sender.send(PartyEvent::Launch2av).map_err(|err| {
                        self.status = PartyStatus::Failed;
                         FailedToSendEvent(PartyEvent::Launch2av, err.to_string())
                    })?;
                    launch2av_fired = true;
                },
                _ = &mut launch2b_timer, if !launch2b_fired => {
                    self.event_sender.send(PartyEvent::Launch2b).map_err(|err| {
                        self.status = PartyStatus::Failed;
                        FailedToSendEvent(PartyEvent::Launch2b, err.to_string())
                    })?;
                    launch2b_fired = true;
                },
                _ = &mut finalize_timer, if !finalize_fired => {
                    self.event_sender.send(PartyEvent::Finalize).map_err(|err| {
                        self.status = PartyStatus::Failed;
                        FailedToSendEvent(PartyEvent::Finalize, err.to_string())
                    })?;
                    finalize_fired = true;
                },
                msg = self.msg_in_receiver.recv() => {
                    sleep(self.cfg.grace_period).await;
                    if let Some(msg) = msg {
                        debug!("Party {} received {} from party {}", self.id, msg.routing.msg_type, msg.routing.sender);
                        if let Err(err) = self.update_state(&msg) {
                            // Shouldn't fail the party, since invalid message
                            // may be sent by anyone.
                            warn!("Failed to update state with {}, got error: {err}", msg.routing.msg_type)
                        }
                    }else if self.msg_in_receiver.is_closed(){
                         self.status = PartyStatus::Failed;
                         return Err(MessageChannelClosed)
                    }
                },
                event = self.event_receiver.recv() => {
                    sleep(self.cfg.grace_period).await;
                    if let Some(event) = event {
                        if let Err(err) = self.follow_event(event) {
                            self.status = PartyStatus::Failed;
                            return Err(LaunchBallotError::FollowEventError(event, err));
                        }
                    }else if self.event_receiver.is_closed(){
                        self.status = PartyStatus::Failed;
                         return Err(EventChannelClosed)
                    }
                },
            }
        }

        Ok(self.get_value_selected())
    }

    /// Prepare state before running a ballot.
    fn prepare_next_ballot(&mut self) -> Result<(), LaunchBallotError> {
        self.reset_state();
        self.ballot += 1;
        self.status = PartyStatus::Launched;
        self.leader = self
            .elector
            .elect_leader(self)
            .map_err(|err| LeaderElectionError(err.to_string()))?;

        Ok(())
    }

    fn reset_state(&mut self) {
        self.parties_voted_before = HashMap::new();
        self.messages_1b_weight = 0;
        self.value_2a = None;
        self.messages_2av_state.reset();
        self.messages_2b_state.reset();

        // Cleaning channels
        while self.event_receiver.try_recv().is_ok() {}
        while self.msg_in_receiver.try_recv().is_ok() {}
    }

    /// Update party's state based on message type.
    fn update_state(&mut self, msg: &MessagePacket) -> Result<(), UpdateStateError<V>> {
        let routing = msg.routing;

        match routing.msg_type {
            ProtocolMessage::Msg1a => {
                if self.status != PartyStatus::Launched {
                    return Err(PartyStatusMismatch {
                        party_status: self.status,
                        needed_status: PartyStatus::Launched,
                    }
                    .into());
                }

                let msg = Message1aContent::unpack(msg)?;

                if msg.ballot != self.ballot {
                    return Err(BallotNumberMismatch {
                        party_ballot_number: self.ballot,
                        message_ballot_number: msg.ballot,
                    }
                    .into());
                }

                if routing.sender != self.leader {
                    return Err(LeaderMismatch {
                        party_leader: self.leader,
                        message_sender: routing.sender,
                    }
                    .into());
                }

                self.status = PartyStatus::Passed1a;
            }
            ProtocolMessage::Msg1b => {
                if self.status != PartyStatus::Passed1a {
                    return Err(PartyStatusMismatch {
                        party_status: self.status,
                        needed_status: PartyStatus::Passed1a,
                    }
                    .into());
                }

                let msg = Message1bContent::unpack(msg)?;

                if msg.ballot != self.ballot {
                    return Err(BallotNumberMismatch {
                        party_ballot_number: self.ballot,
                        message_ballot_number: msg.ballot,
                    }
                    .into());
                }

                if let Some(last_ballot_voted) = msg.last_ballot_voted {
                    if last_ballot_voted >= self.ballot {
                        return Err(BallotNumberMismatch {
                            party_ballot_number: self.ballot,
                            message_ballot_number: msg.ballot,
                        }
                        .into());
                    }
                }

                if let Vacant(e) = self.parties_voted_before.entry(routing.sender) {
                    let value: Option<V> = match msg.last_value_voted {
                        Some(ref data) => Some(
                            bincode::deserialize(data)
                                .map_err(|err| DeserializationError::Value(err.to_string()))?,
                        ),
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
                    return Err(PartyStatusMismatch {
                        party_status: self.status,
                        needed_status: PartyStatus::Passed1b,
                    }
                    .into());
                }

                let msg = Message2aContent::unpack(msg)?;

                if msg.ballot != self.ballot {
                    return Err(BallotNumberMismatch {
                        party_ballot_number: self.ballot,
                        message_ballot_number: msg.ballot,
                    }
                    .into());
                }

                if routing.sender != self.leader {
                    return Err(LeaderMismatch {
                        party_leader: self.leader,
                        message_sender: routing.sender,
                    }
                    .into());
                }

                let value_received = bincode::deserialize(&msg.value[..])
                    .map_err(|err| DeserializationError::Value(err.to_string()))?;

                if self
                    .value_selector
                    .verify(&value_received, &self.parties_voted_before)
                {
                    self.status = PartyStatus::Passed2a;
                    self.value_2a = Some(value_received);
                } else {
                    return Err(ValueVerificationFailed);
                }
            }
            ProtocolMessage::Msg2av => {
                if self.status != PartyStatus::Passed2a {
                    return Err(PartyStatusMismatch {
                        party_status: self.status,
                        needed_status: PartyStatus::Passed2a,
                    }
                    .into());
                }

                let msg = Message2avContent::unpack(msg)?;

                if msg.ballot != self.ballot {
                    return Err(BallotNumberMismatch {
                        party_ballot_number: self.ballot,
                        message_ballot_number: msg.ballot,
                    }
                    .into());
                }
                let value_received: V = bincode::deserialize(&msg.received_value[..])
                    .map_err(|err| DeserializationError::Value(err.to_string()))?;

                if value_received != self.value_2a.clone().unwrap() {
                    return Err(ValueMismatch {
                        party_value: self.value_2a.clone().unwrap(),
                        message_value: value_received.clone(),
                    }
                    .into());
                }

                if !self.messages_2av_state.contains_sender(&routing.sender) {
                    self.messages_2av_state.add_sender(
                        routing.sender,
                        self.cfg.party_weights[routing.sender as usize] as u128,
                    );

                    if self.messages_2av_state.get_weight() > self.cfg.threshold {
                        self.status = PartyStatus::Passed2av;
                    }
                }
            }
            ProtocolMessage::Msg2b => {
                if self.status != PartyStatus::Passed2av {
                    return Err(PartyStatusMismatch {
                        party_status: self.status,
                        needed_status: PartyStatus::Passed2av,
                    }
                    .into());
                }

                let msg = Message2bContent::unpack(msg)?;

                if msg.ballot != self.ballot {
                    return Err(BallotNumberMismatch {
                        party_ballot_number: self.ballot,
                        message_ballot_number: msg.ballot,
                    }
                    .into());
                }

                if self.messages_2av_state.contains_sender(&routing.sender)
                    && !self.messages_2b_state.contains_sender(&routing.sender)
                {
                    self.messages_2b_state.add_sender(
                        routing.sender,
                        self.cfg.party_weights[routing.sender as usize] as u128,
                    );

                    if self.messages_2b_state.get_weight() > self.cfg.threshold {
                        self.status = PartyStatus::Passed2b;
                    }
                }
            }
        }
        Ok(())
    }

    /// Executes ballot actions according to the received event.
    fn follow_event(&mut self, event: PartyEvent) -> Result<(), FollowEventError> {
        match event {
            PartyEvent::Launch1a => {
                if self.status != PartyStatus::Launched {
                    return Err(PartyStatusMismatch {
                        party_status: self.status,
                        needed_status: PartyStatus::Launched,
                    }
                    .into());
                }

                if self.leader == self.id {
                    let content = &Message1aContent {
                        ballot: self.ballot,
                    };
                    let msg = content.pack(self.id)?;

                    self.msg_out_sender
                        .send(msg)
                        .map_err(|err| FailedToSendMessage(err.to_string()))?;
                    self.status = PartyStatus::Passed1a;
                }
            }
            PartyEvent::Launch1b => {
                if self.status != PartyStatus::Passed1a {
                    return Err(PartyStatusMismatch {
                        party_status: self.status,
                        needed_status: PartyStatus::Passed1a,
                    }
                    .into());
                }

                let last_value_voted = self
                    .last_value_voted
                    .clone()
                    .map(|inner_data| {
                        bincode::serialize(&inner_data)
                            .map_err(|err| SerializationError::Value(err.to_string()))
                    })
                    .transpose()?;

                let content = &Message1bContent {
                    ballot: self.ballot,
                    last_ballot_voted: self.last_ballot_voted,
                    last_value_voted,
                };
                let msg = content.pack(self.id)?;

                self.msg_out_sender
                    .send(msg)
                    .map_err(|err| FailedToSendMessage(err.to_string()))?;
            }
            PartyEvent::Launch2a => {
                if self.status != PartyStatus::Passed1b {
                    return Err(PartyStatusMismatch {
                        party_status: self.status,
                        needed_status: PartyStatus::Passed1b,
                    }
                    .into());
                }
                if self.leader == self.id {
                    let value = bincode::serialize(&self.get_value())
                        .map_err(|err| SerializationError::Value(err.to_string()))?;

                    let content = &Message2aContent {
                        ballot: self.ballot,
                        value,
                    };
                    let msg = content.pack(self.id)?;

                    self.msg_out_sender
                        .send(msg)
                        .map_err(|err| FailedToSendMessage(err.to_string()))?;

                    self.value_2a = Some(self.get_value());
                    self.status = PartyStatus::Passed2a;
                }
            }
            PartyEvent::Launch2av => {
                if self.status != PartyStatus::Passed2a {
                    return Err(PartyStatusMismatch {
                        party_status: self.status,
                        needed_status: PartyStatus::Passed2a,
                    }
                    .into());
                }

                let received_value = bincode::serialize(&self.value_2a.clone().unwrap())
                    .map_err(|err| SerializationError::Value(err.to_string()))?;

                let content = &Message2avContent {
                    ballot: self.ballot,
                    received_value,
                };
                let msg = content.pack(self.id)?;

                self.msg_out_sender
                    .send(msg)
                    .map_err(|err| FailedToSendMessage(err.to_string()))?;
            }
            PartyEvent::Launch2b => {
                if self.status != PartyStatus::Passed2av {
                    return Err(PartyStatusMismatch {
                        party_status: self.status,
                        needed_status: PartyStatus::Passed2av,
                    }
                    .into());
                }

                let content = &Message2bContent {
                    ballot: self.ballot,
                };
                let msg = content.pack(self.id)?;

                self.msg_out_sender
                    .send(msg)
                    .map_err(|err| FailedToSendMessage(err.to_string()))?;
            }
            PartyEvent::Finalize => {
                if self.status != PartyStatus::Passed2b {
                    return Err(PartyStatusMismatch {
                        party_status: self.status,
                        needed_status: PartyStatus::Passed2av,
                    }
                    .into());
                }

                self.status = PartyStatus::Finished;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    use crate::leader::DefaultLeaderElector;
    use crate::party::PartyStatus::{Launched, Passed1a, Passed1b, Passed2a};
    use std::collections::HashMap;
    use std::fmt::{Display, Formatter};
    use std::time::Duration;
    use tokio::time;

    #[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Debug)]
    pub(crate) struct MockValue(u64);

    impl Display for MockValue {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            write!(f, "MockValue: {}", self.0)
        }
    }

    impl Value for MockValue {}

    #[derive(Clone)]
    pub(crate) struct MockValueSelector;

    impl ValueSelector<MockValue> for MockValueSelector {
        fn verify(&self, _v: &MockValue, _m: &HashMap<u64, Option<MockValue>>) -> bool {
            true // For testing, always return true
        }

        fn select(&self, _m: &HashMap<u64, Option<MockValue>>) -> MockValue {
            MockValue(1) // For testing, always return the same value
        }
    }

    pub(crate) fn default_config() -> BPConConfig {
        BPConConfig::with_default_timeouts(vec![1, 2, 3], 4)
    }

    pub(crate) fn default_party() -> Party<MockValue, MockValueSelector> {
        Party::<MockValue, MockValueSelector>::new(
            0,
            default_config(),
            MockValueSelector,
            Box::new(DefaultLeaderElector::new()),
        )
        .0
    }

    #[test]
    fn test_update_state_msg1a() {
        let mut party = default_party();
        party.status = Launched;
        let content = Message1aContent {
            ballot: party.ballot,
        };
        let msg = content.pack(party.leader).unwrap();

        party.update_state(&msg).unwrap();

        assert_eq!(party.status, Passed1a);
    }

    #[test]
    fn test_update_state_msg1b() {
        let mut party = default_party();
        party.status = Passed1a;

        let content = Message1bContent {
            ballot: party.ballot,
            last_ballot_voted: None,
            last_value_voted: bincode::serialize(&MockValue(42)).ok(),
        };

        // First, send a 1b message from party 1 (weight 2)
        let msg = content.pack(1).unwrap();
        party.update_state(&msg).unwrap();

        // Then, send a 1b message from party 2 (weight 3)
        let msg = content.pack(2).unwrap();
        party.update_state(&msg).unwrap();

        // After both messages, the cumulative weight is 2 + 3 = 5, which exceeds the threshold
        assert_eq!(party.status, Passed1b);
    }

    #[test]
    fn test_update_state_msg2a() {
        let mut party = default_party();
        party.status = Passed1b;
        party.leader = 1;

        let content = Message2aContent {
            ballot: party.ballot,
            value: bincode::serialize(&MockValue(42)).unwrap(),
        };
        let msg = content.pack(1).unwrap();
        party.update_state(&msg).unwrap();

        assert_eq!(party.status, Passed2a);
    }

    #[test]
    fn test_update_state_msg2av() {
        let mut party = default_party();
        party.status = Passed2a;
        party.value_2a = Some(MockValue(1));

        let content = Message2avContent {
            ballot: party.ballot,
            received_value: bincode::serialize(&MockValue(1)).unwrap(),
        };

        // Send first 2av message from party 1 (weight 2)
        let msg = content.pack(1).unwrap();
        party.update_state(&msg).unwrap();

        // Now send a second 2av message from party 2 (weight 3)
        let msg = content.pack(2).unwrap();
        party.update_state(&msg).unwrap();

        // The cumulative weight (2 + 3) should exceed the threshold of 4
        assert_eq!(party.status, PartyStatus::Passed2av);
    }

    #[test]
    fn test_update_state_msg2b() {
        let mut party = default_party();
        party.status = PartyStatus::Passed2av;

        // Simulate that both party 1 and party 2 already sent 2av messages
        party.messages_2av_state.add_sender(1, 2);
        party.messages_2av_state.add_sender(2, 3);

        let content = Message2bContent {
            ballot: party.ballot,
        };

        // Send first 2b message from party 1 (weight 2)
        let msg = content.pack(1).unwrap();
        party.update_state(&msg).unwrap();

        // Print the current state and weight
        println!(
            "After first Msg2b: Status = {}, 2b Weight = {}",
            party.status,
            party.messages_2b_state.get_weight()
        );

        // Now send a second 2b message from party 2 (weight 3)
        let msg = content.pack(2).unwrap();
        party.update_state(&msg).unwrap();

        // Print the current state and weight
        println!(
            "After second Msg2b: Status = {}, 2b Weight = {}",
            party.status,
            party.messages_2b_state.get_weight()
        );

        // The cumulative weight (3 + 2) should exceed the threshold of 4
        assert_eq!(party.status, PartyStatus::Passed2b);
    }

    #[test]
    fn test_follow_event_launch1a() {
        let cfg = default_config();
        // Need to take ownership of msg_out_receiver, so that sender doesn't close,
        // since otherwise msg_out_receiver will be dropped.
        let (mut party, _msg_out_receiver, _) = Party::<MockValue, MockValueSelector>::new(
            0,
            cfg,
            MockValueSelector,
            Box::new(DefaultLeaderElector {}),
        );

        party.status = Launched;
        party.leader = party.id;

        party
            .follow_event(PartyEvent::Launch1a)
            .expect("Failed to follow Launch1a event");

        // If the party is the leader and in the Launched state, the event should trigger a message.
        // And it's status shall update to Passed1a after sending 1a message,
        // contrary to other participants, whose `Passed1a` updates only after receiving 1a message.
        assert_eq!(party.status, Passed1a);
    }

    #[test]
    fn test_follow_event_communication_failure() {
        // msg_out_receiver channel, bound to corresponding sender, which will try to use
        // follow event, is getting dropped since we don't take ownership of it
        // upon creation of the party
        let mut party = default_party();
        party.status = Launched;
        party.leader = party.id;

        let result = party.follow_event(PartyEvent::Launch1a);

        match result {
            Err(FailedToSendMessage(_)) => {
                // this is expected outcome
            }
            _ => panic!(
                "Expected FollowEventError::FailedToSendMessage, got {:?}",
                result
            ),
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
                Box::new(DefaultLeaderElector::new()),
            );

        // Same here, we would like to not lose party's event_receiver, so that test doesn't fail.
        let _event_sender = party.event_sender;
        party.event_sender = event_sender;

        // Spawn the launch_ballot function in a separate task
        let _ballot_task = tokio::spawn(async move {
            party.launch_ballot().await.unwrap();
        });

        time::advance(cfg.launch_timeout).await;

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

    #[tokio::test]
    async fn test_end_to_end_ballot() {
        // Configuration for the parties
        let cfg = BPConConfig::with_default_timeouts(vec![1, 1, 1, 1], 2);

        // ValueSelector and LeaderElector instances
        let value_selector = MockValueSelector;
        let leader_elector = Box::new(DefaultLeaderElector::new());

        // Create 4 parties
        let (mut party0, msg_out_receiver0, msg_in_sender0) =
            Party::<MockValue, MockValueSelector>::new(
                0,
                cfg.clone(),
                value_selector.clone(),
                leader_elector.clone(),
            );
        let (mut party1, msg_out_receiver1, msg_in_sender1) =
            Party::<MockValue, MockValueSelector>::new(
                1,
                cfg.clone(),
                value_selector.clone(),
                leader_elector.clone(),
            );
        let (mut party2, msg_out_receiver2, msg_in_sender2) =
            Party::<MockValue, MockValueSelector>::new(
                2,
                cfg.clone(),
                value_selector.clone(),
                leader_elector.clone(),
            );
        let (mut party3, msg_out_receiver3, msg_in_sender3) =
            Party::<MockValue, MockValueSelector>::new(
                3,
                cfg.clone(),
                value_selector.clone(),
                leader_elector.clone(),
            );

        // Channels for receiving the selected values
        let (value_sender0, value_receiver0) = tokio::sync::oneshot::channel();
        let (value_sender1, value_receiver1) = tokio::sync::oneshot::channel();
        let (value_sender2, value_receiver2) = tokio::sync::oneshot::channel();
        let (value_sender3, value_receiver3) = tokio::sync::oneshot::channel();

        // Launch ballot tasks for each party
        let ballot_task0 = tokio::spawn(async move {
            match party0.launch_ballot().await {
                Ok(Some(value)) => {
                    value_sender0.send(value).unwrap();
                }
                Ok(None) => {
                    eprintln!("Party 0: No value was selected");
                }
                Err(err) => {
                    eprintln!("Party 0 encountered an error: {:?}", err);
                }
            }
        });

        let ballot_task1 = tokio::spawn(async move {
            match party1.launch_ballot().await {
                Ok(Some(value)) => {
                    value_sender1.send(value).unwrap();
                }
                Ok(None) => {
                    eprintln!("Party 1: No value was selected");
                }
                Err(err) => {
                    eprintln!("Party 1 encountered an error: {:?}", err);
                }
            }
        });

        let ballot_task2 = tokio::spawn(async move {
            match party2.launch_ballot().await {
                Ok(Some(value)) => {
                    value_sender2.send(value).unwrap();
                }
                Ok(None) => {
                    eprintln!("Party 2: No value was selected");
                }
                Err(err) => {
                    eprintln!("Party 2 encountered an error: {:?}", err);
                }
            }
        });

        let ballot_task3 = tokio::spawn(async move {
            match party3.launch_ballot().await {
                Ok(Some(value)) => {
                    value_sender3.send(value).unwrap();
                }
                Ok(None) => {
                    eprintln!("Party 3: No value was selected");
                }
                Err(err) => {
                    eprintln!("Party 3 encountered an error: {:?}", err);
                }
            }
        });

        // Simulate message passing between the parties
        tokio::spawn(async move {
            let mut receivers = [
                msg_out_receiver0,
                msg_out_receiver1,
                msg_out_receiver2,
                msg_out_receiver3,
            ];
            let senders = [
                msg_in_sender0,
                msg_in_sender1,
                msg_in_sender2,
                msg_in_sender3,
            ];

            loop {
                for (i, receiver) in receivers.iter_mut().enumerate() {
                    if let Ok(msg) = receiver.try_recv() {
                        // Broadcast the message to all other parties
                        for (j, sender) in senders.iter().enumerate() {
                            if i != j {
                                sender.send(msg.clone()).unwrap();
                            }
                        }
                    }
                }

                // Delay to simulate network latency
                sleep(Duration::from_millis(100)).await;
            }
        });

        // Await the completion of ballot tasks
        ballot_task0.await.unwrap();
        ballot_task1.await.unwrap();
        ballot_task2.await.unwrap();
        ballot_task3.await.unwrap();

        // Await results from each party
        let value0 = value_receiver0.await.unwrap();
        let value1 = value_receiver1.await.unwrap();
        let value2 = value_receiver2.await.unwrap();
        let value3 = value_receiver3.await.unwrap();

        // Check that all parties reached the same consensus value
        assert_eq!(value0, value1, "Party 0 and 1 agreed on the same value");
        assert_eq!(value1, value2, "Party 1 and 2 agreed on the same value");
        assert_eq!(value2, value3, "Party 2 and 3 agreed on the same value");
    }
}

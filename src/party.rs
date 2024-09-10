//! Definitions central to BPCon participant.
//!
//! This module contains the implementation of a `Party` in the BPCon consensus protocol.
//! The `Party` manages the execution of the ballot, handles incoming messages, and coordinates with other participants
//! to reach a consensus. It uses various components such as leader election and value selection to perform its duties.

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
use crate::value::{Value, ValueSelector};
use log::{debug, warn};
use std::cmp::PartialEq;
use std::collections::hash_map::Entry::Vacant;
use std::collections::{HashMap, HashSet};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::time::sleep;

/// Represents the status of a `Party` in the BPCon consensus protocol.
///
/// The status indicates the current phase or outcome of the ballot execution for this party.
/// It transitions through various states as the party progresses through the protocol.
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

/// Represents the events that control the flow of the ballot process in a `Party`.
///
/// These events trigger transitions between different phases of the protocol.
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

/// A participant in the BPCon protocol responsible for executing ballots.
///
/// A `Party` manages the execution of ballots by communicating with other parties, processing incoming messages,
/// and following the protocol's steps. It uses an internal state machine to track its progress through the ballot process.
///
/// # Communication
/// - `msg_in_receiver`: Receives incoming messages from other parties.
/// - `msg_out_sender`: Sends outgoing messages to other parties.
/// - `event_receiver`: Receives events that drive the ballot process.
/// - `event_sender`: Sends events to trigger actions in the ballot process.
///
/// The `Party` operates within a BPCon configuration and relies on a `ValueSelector` to choose values during the consensus process.
/// A `LeaderElector` is used to determine the leader for each ballot.
pub struct Party<V: Value, VS: ValueSelector<V>> {
    /// The identifier of this party.
    pub id: u64,

    /// Queue for receiving incoming messages.
    msg_in_receiver: UnboundedReceiver<MessagePacket>,
    /// Queue for sending outgoing messages.
    msg_out_sender: UnboundedSender<MessagePacket>,

    /// Queue for receiving events that control the ballot process.
    event_receiver: UnboundedReceiver<PartyEvent>,
    /// Queue for sending events that control the ballot process.
    event_sender: UnboundedSender<PartyEvent>,

    /// BPCon configuration settings, including timeouts and party weights.
    pub(crate) cfg: BPConConfig,

    /// Component responsible for selecting values during the consensus process.
    value_selector: VS,

    /// Component responsible for electing a leader for each ballot.
    elector: Box<dyn LeaderElector<V, VS>>,

    /// The current status of the ballot execution for this party.
    status: PartyStatus,

    /// The current ballot number.
    pub(crate) ballot: u64,

    /// The leader for the current ballot.
    leader: u64,

    /// The last ballot number where this party submitted a 2b message.
    last_ballot_voted: Option<u64>,

    /// The last value for which this party submitted a 2b message.
    last_value_voted: Option<V>,

    /// DDoS prevention mechanism - we allow each party to send one message type per ballot.
    rate_limiter: HashSet<(ProtocolMessage, u64)>,

    // Local round fields
    /// The state of 1b round, tracking which parties have voted and their corresponding values.
    parties_voted_before: HashMap<u64, Option<V>>,
    /// The cumulative weight of 1b messages received.
    messages_1b_weight: u128,

    /// The state of 2a round, storing the value proposed by this party.
    value_2a: Option<V>,

    /// The state of 2av round, tracking which parties have confirmed the 2a value.
    messages_2av_state: MessageRoundState,

    /// The state of 2b round, tracking which parties have sent 2b messages.
    messages_2b_state: MessageRoundState,
}

impl<V: Value, VS: ValueSelector<V>> Party<V, VS> {
    /// Creates a new `Party` instance.
    ///
    /// This constructor sets up the party with the given ID, BPCon configuration, value selector, and leader elector.
    /// It also initializes communication channels for receiving and sending messages and events.
    ///
    /// # Parameters
    /// - `id`: The unique identifier for this party.
    /// - `cfg`: The BPCon configuration settings.
    /// - `value_selector`: The component responsible for selecting values during the consensus process.
    /// - `elector`: The component responsible for electing the leader for each ballot.
    ///
    /// # Returns
    /// - A tuple containing:
    ///   - The new `Party` instance.
    ///   - The `UnboundedReceiver` for outgoing messages.
    ///   - The `UnboundedSender` for incoming messages.
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
                rate_limiter: HashSet::new(),
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

    /// Returns the current ballot number.
    pub fn ballot(&self) -> u64 {
        self.ballot
    }

    /// Checks if the ballot process has been launched.
    ///
    /// # Returns
    /// - `true` if the ballot is currently active; `false` otherwise.
    pub fn is_launched(&self) -> bool {
        !self.is_stopped()
    }

    /// Checks if the ballot process has been stopped.
    ///
    /// # Returns
    /// - `true` if the ballot has finished or failed; `false` otherwise.
    pub fn is_stopped(&self) -> bool {
        self.status == PartyStatus::Finished || self.status == PartyStatus::Failed
    }

    /// Retrieves the selected value if the ballot process has finished successfully.
    ///
    /// # Returns
    /// - `Some(V)` if the ballot reached a consensus and the value was selected.
    /// - `None` if the ballot did not reach consensus or is still ongoing.
    pub fn get_value_selected(&self) -> Option<V> {
        // Only `Finished` status means reached BFT agreement
        if self.status == PartyStatus::Finished {
            return self.value_2a.clone();
        }

        None
    }

    /// Selects a value based on the current state of the party.
    ///
    /// This method delegates to the `ValueSelector` to determine the value that should be selected
    /// based on the votes received in the 1b round.
    ///
    /// # Returns
    /// - The value selected by the `ValueSelector`.
    fn get_value(&self) -> V {
        self.value_selector.select(&self.parties_voted_before)
    }

    /// Launches the ballot process.
    ///
    /// This method initiates the ballot process, advancing through the different phases of the protocol
    /// by sending and receiving events and messages. It handles timeouts for each phase and processes
    /// incoming messages to update the party's state.
    ///
    /// # Returns
    /// - `Ok(Some(V))`: The selected value if the ballot reaches consensus.
    /// - `Ok(None)`: If the ballot process is terminated without reaching consensus.
    /// - `Err(LaunchBallotError)`: If an error occurs during the ballot process.
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
                        let meta = (msg.routing.msg_type, msg.routing.sender);
                        if !self.rate_limiter.contains(&meta){
                            debug!("Party {} received {} from party {}", self.id, meta.0, meta.1);
                            if let Err(err) = self.update_state(&msg) {
                                // Shouldn't fail the party, since invalid message
                                // may be sent by anyone. Furthermore, since in consensus
                                // we are relying on redundancy of parties, we actually may need
                                // less messages than from every party to transit to next status.
                                warn!("Failed to update state for party {} with {}, got error: {err}", self.id, meta.0)
                            }
                            self.rate_limiter.insert(meta);
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

    /// Prepares the party's state for the next ballot.
    ///
    /// This method resets the party's state, increments the ballot number, and elects a new leader.
    ///
    /// # Returns
    /// - `Ok(())`: If the preparation is successful.
    /// - `Err(LaunchBallotError)`: If an error occurs during leader election.
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

    /// Resets the party's state for a new round of ballot execution.
    ///
    /// This method clears the state associated with previous rounds and prepares the party for the next ballot.
    fn reset_state(&mut self) {
        self.rate_limiter = HashSet::new();
        self.parties_voted_before = HashMap::new();
        self.messages_1b_weight = 0;
        self.value_2a = None;
        self.messages_2av_state.reset();
        self.messages_2b_state.reset();

        // Cleaning channels
        while self.event_receiver.try_recv().is_ok() {}
        while self.msg_in_receiver.try_recv().is_ok() {}
    }

    /// Updates the party's state based on an incoming message.
    ///
    /// This method processes a message according to its type and updates the party's internal state accordingly.
    /// It performs validation checks to ensure that the message is consistent with the current ballot and protocol rules.
    ///
    /// # Parameters
    /// - `msg`: The incoming `MessagePacket` to be processed.
    ///
    /// # Returns
    /// - `Ok(())`: If the state is successfully updated.
    /// - `Err(UpdateStateError<V>)`: If an error occurs during the update, such as a mismatch in the ballot number or leader.
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

                    let self_weight = self.cfg.party_weights[self.id as usize] as u128;
                    if self.messages_1b_weight >= self.cfg.threshold - self_weight {
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

                    let self_weight = self.cfg.party_weights[self.id as usize] as u128;
                    if self.messages_2av_state.get_weight() >= self.cfg.threshold - self_weight {
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

                    let self_weight = self.cfg.party_weights[self.id as usize] as u128;
                    if self.messages_2b_state.get_weight() >= self.cfg.threshold - self_weight {
                        self.status = PartyStatus::Passed2b;
                    }
                }
            }
        }
        Ok(())
    }

    /// Executes ballot actions according to the received event.
    ///
    /// This method processes an event and triggers the corresponding action in the ballot process,
    /// such as launching a new phase or finalizing the ballot.
    ///
    /// # Parameters
    /// - `event`: The `PartyEvent` to process.
    ///
    /// # Returns
    /// - `Ok(())`: If the event is successfully processed.
    /// - `Err(FollowEventError)`: If an error occurs while processing the event.
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
    use tokio::task::JoinHandle;
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

    // Here each party/receiver/sender shall correspond at equal indexes.
    type PartiesWithChannels = (
        Vec<Party<MockValue, MockValueSelector>>,
        Vec<UnboundedReceiver<MessagePacket>>,
        Vec<UnboundedSender<MessagePacket>>,
    );

    // Create test parties with predefined generics, based on config.
    fn create_parties(cfg: BPConConfig) -> PartiesWithChannels {
        let value_selector = MockValueSelector;
        let leader_elector = Box::new(DefaultLeaderElector::new());

        // Note: each weight corresponds to party, ergo their lengths are equal.
        (0..cfg.party_weights.len())
            .map(|i| {
                Party::<MockValue, MockValueSelector>::new(
                    i as u64,
                    cfg.clone(),
                    value_selector.clone(),
                    leader_elector.clone(),
                )
            })
            .fold(
                (Vec::new(), Vec::new(), Vec::new()),
                |(mut parties, mut receivers, mut senders), (p, r, s)| {
                    parties.push(p);
                    receivers.push(r);
                    senders.push(s);
                    (parties, receivers, senders)
                },
            )
    }

    // Begin ballot process for each party.
    fn launch_parties(
        parties: Vec<Party<MockValue, MockValueSelector>>,
    ) -> Vec<JoinHandle<Result<Option<MockValue>, LaunchBallotError>>> {
        parties
            .into_iter()
            .map(|mut party| tokio::spawn(async move { party.launch_ballot().await }))
            .collect()
    }

    // Collect messages from receivers.
    fn collect_messages(receivers: &mut [UnboundedReceiver<MessagePacket>]) -> Vec<MessagePacket> {
        receivers
            .iter_mut()
            .filter_map(|receiver| receiver.try_recv().ok())
            .collect()
    }

    // Broadcast collected messages to other parties, skipping the sender.
    fn broadcast_messages(
        messages: Vec<MessagePacket>,
        senders: &[UnboundedSender<MessagePacket>],
    ) {
        messages.iter().for_each(|msg| {
            senders
                .iter()
                .enumerate()
                .filter(|(i, _)| msg.routing.sender != *i as u64) // Skip the current party (sender).
                .for_each(|(_, sender_into)| {
                    sender_into.send(msg.clone()).unwrap();
                });
        });
    }

    // Propagate messages peer-to-peer between parties using their channels.
    fn propagate_messages_p2p(
        mut receivers: Vec<UnboundedReceiver<MessagePacket>>,
        senders: Vec<UnboundedSender<MessagePacket>>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                let messages = collect_messages(receivers.as_mut_slice());
                broadcast_messages(messages, &senders);

                // Delay to simulate network latency.
                sleep(Duration::from_millis(100)).await;
            }
        })
    }

    // Await completion of each party's process and aggregate ok results.
    async fn extract_values_from_ballot_tasks(
        tasks: Vec<JoinHandle<Result<Option<MockValue>, LaunchBallotError>>>,
    ) -> Vec<MockValue> {
        let mut values = Vec::new();
        for (i, task) in tasks.into_iter().enumerate() {
            let result = task.await.unwrap();

            match result {
                Ok(Some(value)) => {
                    values.push(value);
                }
                Ok(None) => {
                    // This shall never happen for finished party.
                    eprintln!("Party {}: No value was selected", i);
                }
                Err(err) => {
                    eprintln!("Party {} encountered an error: {:?}", i, err);
                }
            }
        }
        values
    }

    // Use returned values from each party to analyze how ballot passed.
    fn analyze_ballot_result(values: Vec<MockValue>) {
        match values.as_slice() {
            [] => {
                eprintln!("No consensus could be reached, as no values were selected by any party")
            }
            [first_value, ..] => {
                match values.iter().all(|v| v == first_value) {
                    true => println!(
                        "All parties reached the same consensus value: {:?}",
                        first_value
                    ),
                    false => eprintln!("Not all parties agreed on the same value"),
                }
                println!("Consensus agreed on value {first_value:?}");
            }
        }
    }

    #[tokio::test]
    async fn test_ballot_happy_case() {
        let cfg = BPConConfig::with_default_timeouts(vec![1, 1, 1, 1], 3);

        let (parties, receivers, senders) = create_parties(cfg);

        let ballot_tasks = launch_parties(parties);

        let p2p_task = propagate_messages_p2p(receivers, senders);

        let values = extract_values_from_ballot_tasks(ballot_tasks).await;

        p2p_task.abort();

        analyze_ballot_result(values);
    }

    #[tokio::test]
    async fn test_ballot_faulty_party() {
        let cfg = BPConConfig::with_default_timeouts(vec![1, 1, 1, 1], 3);

        let (mut parties, mut receivers, mut senders) = create_parties(cfg);

        let leader = parties[0].leader;

        assert_ne!(3, leader, "Should not fail the leader for the test to pass");

        // Simulate failure of `f` faulty participants:
        let parties = parties.drain(..3).collect();
        let receivers = receivers.drain(..3).collect();
        let senders = senders.drain(..3).collect();

        let ballot_tasks = launch_parties(parties);

        let p2p_task = propagate_messages_p2p(receivers, senders);

        let values = extract_values_from_ballot_tasks(ballot_tasks).await;

        p2p_task.abort();

        analyze_ballot_result(values);
    }

    #[tokio::test]
    async fn test_ballot_malicious_party() {
        let cfg = BPConConfig::with_default_timeouts(vec![1, 1, 1, 1], 3);

        let (parties, mut receivers, senders) = create_parties(cfg);

        let leader = parties[0].leader;
        const MALICIOUS_PARTY_ID: u64 = 1;

        assert_ne!(
            MALICIOUS_PARTY_ID, leader,
            "Should not make malicious the leader for the test to pass"
        );

        // We will be simulating malicious behaviour
        // sending 1b message (meaning, 5/6 times at incorrect stage) with the wrong data.
        let content = &Message1bContent {
            ballot: parties[0].ballot + 1, // divergent ballot number
            last_ballot_voted: Some(parties[0].ballot + 1), // early ballot number
            // shouldn't put malformed serialized value, because we won't be able to pack it
            last_value_voted: None,
        };
        let malicious_msg = content.pack(MALICIOUS_PARTY_ID).unwrap();

        let ballot_tasks = launch_parties(parties);

        let p2p_task = tokio::spawn(async move {
            let mut last_malicious_message_time = time::Instant::now();
            let malicious_message_interval = Duration::from_millis(3000);
            loop {
                // Collect all messages first.
                let mut messages: Vec<_> = receivers
                    .iter_mut()
                    .enumerate()
                    .filter_map(|(i, receiver)| {
                        // Skip receiving messages from the malicious party
                        // to substitute it with invalid one to be propagated.
                        (i != MALICIOUS_PARTY_ID as usize)
                            .then(|| receiver.try_recv().ok())
                            .flatten()
                    })
                    .collect();

                // Push the malicious message at intervals
                // to mitigate bloating inner receiving channel.
                if last_malicious_message_time.elapsed() >= malicious_message_interval {
                    messages.push(malicious_msg.clone());
                    last_malicious_message_time = time::Instant::now();
                }

                broadcast_messages(messages, &senders);

                // Delay to simulate network latency.
                sleep(Duration::from_millis(100)).await;
            }
        });

        let values = extract_values_from_ballot_tasks(ballot_tasks).await;

        p2p_task.abort();

        analyze_ballot_result(values);
    }
}

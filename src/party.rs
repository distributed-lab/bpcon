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
use tokio::time::{sleep, sleep_until};

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
        !(self.is_stopped() || self.status == PartyStatus::None)
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
    pub async fn launch_ballot(&mut self) -> Result<V, LaunchBallotError> {
        self.prepare_next_ballot()?;

        sleep_until(self.cfg.launch_at).await;

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

        while self.status != PartyStatus::Finished {
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
                        debug!("Party {} received {} from party {}", self.id, meta.0, meta.1);

                        if self.id == meta.1{
                            warn!("Received own message {}, intended to be broadcasted.", meta.0);
                            continue
                        }
                        if self.rate_limiter.contains(&meta){
                            warn!("Party {} hit rate limit in party {} for message {}", meta.1, self.id, meta.0);
                            continue
                        }

                        if let Err(err) = self.update_state(&msg) {
                            // Shouldn't fail the party, since invalid message
                            // may be sent by anyone. Furthermore, since in consensus
                            // we are relying on redundancy of parties, we actually may need
                            // less messages than from every party to transit to next status.
                            warn!("Failed to update state for party {} with {}, got error: {err}", self.id, meta.0)
                        }
                        self.rate_limiter.insert(meta);

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

        Ok(self.get_value_selected().unwrap())
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
mod tests {
    use super::*;

    use crate::leader::DefaultLeaderElector;
    use crate::test_mocks::{MockParty, MockValue};
    use tokio::time;

    #[test]
    fn test_update_state_msg1a() {
        // We wish to simulate behavior of leader
        // sending message to another participant.
        let mut party = MockParty {
            status: PartyStatus::Launched,
            id: 0,
            leader: 1,
            ..Default::default()
        };
        let content = Message1aContent {
            ballot: party.ballot,
        };
        let msg = content.pack(party.leader).unwrap();

        party.update_state(&msg).unwrap();

        assert_eq!(party.status, PartyStatus::Passed1a);
    }

    #[test]
    fn test_update_state_msg1b() {
        let mut party = MockParty {
            status: PartyStatus::Passed1a,
            ..Default::default()
        };

        let content = Message1bContent {
            ballot: party.ballot,
            last_ballot_voted: None,
            last_value_voted: bincode::serialize(&MockValue::default()).ok(),
        };

        // First, send a 1b message from party 1.
        let msg = content.pack(1).unwrap();
        party.update_state(&msg).unwrap();

        // Then, send a 1b message from party 2.
        let msg = content.pack(2).unwrap();
        party.update_state(&msg).unwrap();

        // After both messages, the cumulative weight is
        // 1 (own) + 1 (party 1) + 1 (party 2) = 3, which satisfies the threshold.
        assert_eq!(party.status, PartyStatus::Passed1b);
    }

    #[test]
    fn test_update_state_msg2a() {
        let mut party = MockParty {
            status: PartyStatus::Passed1b,
            ..Default::default()
        };

        let content = Message2aContent {
            ballot: party.ballot,
            value: bincode::serialize(&MockValue::default()).unwrap(),
        };
        let msg = content.pack(party.leader).unwrap();

        party.update_state(&msg).unwrap();

        assert_eq!(party.status, PartyStatus::Passed2a);
    }

    #[test]
    fn test_update_state_msg2av() {
        let value_to_verify = MockValue::default();
        let mut party = MockParty {
            status: PartyStatus::Passed2a,
            value_2a: Some(value_to_verify.clone()),
            ..Default::default()
        };

        let content = Message2avContent {
            ballot: party.ballot,
            received_value: bincode::serialize(&value_to_verify).unwrap(),
        };

        // First, send a 2av message from party 1.
        let msg = content.pack(1).unwrap();
        party.update_state(&msg).unwrap();

        // Then, send a 2av message from party 2.
        let msg = content.pack(2).unwrap();
        party.update_state(&msg).unwrap();

        // After both messages, the cumulative weight is
        // 1 (own) + 1 (party 1) + 1 (party 2) = 3, which satisfies the threshold.
        assert_eq!(party.status, PartyStatus::Passed2av);
    }

    #[test]
    fn test_update_state_msg2b() {
        let mut party = MockParty {
            status: PartyStatus::Passed2av,
            ..Default::default()
        };

        // Simulate that both party 1 and party 2 have already sent 2av messages.
        party.messages_2av_state.add_sender(1, 1);
        party.messages_2av_state.add_sender(2, 1);

        let content = Message2bContent {
            ballot: party.ballot,
        };

        // First, send 2b message from party 1.
        let msg = content.pack(1).unwrap();
        party.update_state(&msg).unwrap();

        // Then, send 2b message from party 2.
        let msg = content.pack(2).unwrap();
        party.update_state(&msg).unwrap();

        // After both messages, the cumulative weight is
        // 1 (own) + 1 (party 1) + 1 (party 2) = 3, which satisfies the threshold.
        assert_eq!(party.status, PartyStatus::Passed2b);
    }

    #[test]
    fn test_follow_event_launch1a() {
        // Need to take ownership of msg_out_receiver, so that sender doesn't close,
        // since otherwise msg_out_receiver will be dropped and party will fail.
        let (mut party, _receiver_from, _) = MockParty::new(
            Default::default(),
            Default::default(),
            Default::default(),
            Box::new(DefaultLeaderElector::default()),
        );

        party.status = PartyStatus::Launched;
        party.leader = party.id;

        party.follow_event(PartyEvent::Launch1a).unwrap();

        // If the party is the leader and in the Launched state, the event should trigger a message.
        // And it's status shall update to Passed1a after sending 1a message,
        // contrary to other participants, whose `Passed1a` updates only after receiving 1a message.
        assert_eq!(party.status, PartyStatus::Passed1a);
    }

    #[tokio::test]
    async fn test_launch_ballot_events() {
        // Pause the Tokio time so we can manipulate it.
        time::pause();

        let cfg = BPConConfig::default();

        // Returning both channels so that party won't fail,
        // because associated channels will close otherwise.
        let (mut party, _receiver_from, _sender_into) = MockParty::new(
            Default::default(),
            cfg.clone(),
            Default::default(),
            Box::new(DefaultLeaderElector::default()),
        );

        let (event_sender, mut event_receiver) = unbounded_channel();
        // Keeping, so that associated party's event_receiver won't close
        // and it doesn't fail.
        let _event_sender = party.event_sender;
        party.event_sender = event_sender;

        // Spawn the launch_ballot function in a separate task.
        _ = tokio::spawn(async move {
            _ = party.launch_ballot().await;
        });

        // Sequential time advance and event check.

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
}

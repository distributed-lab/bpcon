pub trait SignatureAlgorithm {
    fn sign_message(private_key: &[u8], message: &[u8]) -> Vec<u8>;
    fn verify_signature(public_key: &[u8], message: &[u8], signature: &[u8]) -> bool;
}

pub trait Identifier: Clone + std::cmp::Eq{
    fn public_key(&self) -> &[u8];
}

pub trait State<I: Identifier>: Clone{
    type Vote: Clone + std::cmp::Eq + std::hash::Hash;
    
    fn get_sys_info(&self) -> Vec<u8>;
    fn get_highest_ballot(&self) -> u64;
    fn get_vote(&self) -> Option<Self::Vote>;
    fn get_1b_messages(&self) -> Vec<Msg1b<I, Self>>;
    fn get_2av_messages(&self) -> Vec<Msg2av<I, Self>>;
    fn get_current_leader(&self) -> I;
    fn get_quorum_size(&self) -> usize;
    
    fn set_vote(&mut self, vote: Self::Vote);
}

pub trait ProtocolMessage<I: Identifier, S: State<I>, SA: SignatureAlgorithm>: Clone {
    fn set_content(&mut self, sender: &I, receiver: &I, sender_state: &S);
    fn sign(&mut self, sender_prk: &[u8]);
    fn verify_signature(&self, sender: &I) -> bool;
    fn verify_content(&self, receiver_state: &S) -> bool;
    fn apply_content(&self, receiver_state: &mut S);
    fn serialize_for_signing(&self) -> Vec<u8>;
}

#[derive(Clone)]
pub struct BaseMessage<I: Identifier>{
    sender: I,
    receiver: I,
    sender_sig: Vec<u8>,
    sys_info: Vec<u8>,
}
#[derive(Clone)]
pub struct Msg1a<I: Identifier> {
    pub base: BaseMessage<I>,
}
#[derive(Clone)]
pub struct Msg1b<I: Identifier, S: State<I> + ?Sized> {
    pub base: BaseMessage<I>,
    pub highest_ballot: u64,
    pub vote: Option<S::Vote>,
}
#[derive(Clone)]
pub struct Msg1c<I: Identifier, S: State<I>> {
    pub base: BaseMessage<I>,
    pub proofs: Vec<Msg1b<I, S>>,
    pub vote: S::Vote,
}
#[derive(Clone)]
pub struct Msg2a<I: Identifier, S: State<I>> {
    pub base: BaseMessage<I>,
    pub vote: S::Vote,
}
#[derive(Clone)]
pub struct Msg2av<I: Identifier, S: State<I>> {
    pub base: BaseMessage<I>,
    pub vote: S::Vote,
}
#[derive(Clone)]
pub struct Msg2b<I: Identifier, S: State<I>> {
    pub base: BaseMessage<I>,
    pub vote: S::Vote,
}
#[derive(Clone)]
pub enum Message<I: Identifier, S: State<I>>{
    Msg1a(Msg1a<I>),
    Msg1b(Msg1b<I, S>),
    Msg1c(Msg1c<I, S>),
    Msg2a(Msg2a<I, S>),
    Msg2av(Msg2av<I, S>),
    Msg2b(Msg2b<I, S>),
}

impl<I: Identifier, S: State<I>, SA: SignatureAlgorithm> ProtocolMessage<I, S, SA> for Message<I, S>{
    fn set_content(&mut self, sender: &I, receiver: &I, sender_state: &S) {
        let base_message = BaseMessage {
            sender: sender.clone(),
            receiver: receiver.clone(),
            sender_sig: vec![],
            sys_info: sender_state.get_sys_info(),
        };
        match self {
            Message::Msg1a(msg) => {
                msg.base = base_message;
            },
            Message::Msg1b(msg) => {
                msg.base = base_message;
                msg.highest_ballot = sender_state.get_highest_ballot();
                msg.vote = sender_state.get_vote();
            },
            Message::Msg1c(msg) => {
                msg.base = base_message;
                msg.proofs = sender_state.get_1b_messages();

                let mut highest_ballot = 0;
                let mut vote: Option<S::Vote> = None;

                for proof in &msg.proofs {
                    if proof.highest_ballot > highest_ballot {
                        highest_ballot = proof.highest_ballot;
                        vote = proof.vote.clone();
                    }
                }

                msg.vote = vote.expect("No valid vote found");
            },
            Message::Msg2a(msg) => {
                msg.base = base_message;
                msg.vote = sender_state.get_vote().unwrap();
            },
            Message::Msg2av(msg) => {
                msg.base = base_message;
                msg.vote = sender_state.get_vote().unwrap();
            },
            Message::Msg2b(msg) => {
                msg.base = base_message;
                msg.vote = sender_state.get_vote().unwrap();
            }
        }
    }

    fn sign(&mut self, sender_prk: &[u8]) {
        let serialized_data = <Message<I, S> as ProtocolMessage<I, S, SA>>::serialize_for_signing(self);
        match self {
            Message::Msg1a(msg) => msg.base.sender_sig = SA::sign_message(sender_prk, &serialized_data),
            Message::Msg1b(msg) => msg.base.sender_sig = SA::sign_message(sender_prk, &serialized_data),
            Message::Msg1c(msg) => msg.base.sender_sig = SA::sign_message(sender_prk, &serialized_data),
            Message::Msg2a(msg) => msg.base.sender_sig = SA::sign_message(sender_prk, &serialized_data),
            Message::Msg2av(msg) => msg.base.sender_sig = SA::sign_message(sender_prk, &serialized_data),
            Message::Msg2b(msg) => msg.base.sender_sig = SA::sign_message(sender_prk, &serialized_data),
        }
    }

    fn verify_signature(&self, sender: &I) -> bool {
        let serialized_data = <Message<I, S> as ProtocolMessage<I, S, SA>>::serialize_for_signing(self);
        match self {
            Message::Msg1a(msg) => SA::verify_signature(sender.public_key(), &serialized_data, &msg.base.sender_sig),
            Message::Msg1b(msg) => SA::verify_signature(sender.public_key(), &serialized_data, &msg.base.sender_sig),
            Message::Msg1c(msg) => SA::verify_signature(sender.public_key(), &serialized_data, &msg.base.sender_sig),
            Message::Msg2a(msg) => SA::verify_signature(sender.public_key(), &serialized_data, &msg.base.sender_sig),
            Message::Msg2av(msg) => SA::verify_signature(sender.public_key(), &serialized_data, &msg.base.sender_sig),
            Message::Msg2b(msg) => SA::verify_signature(sender.public_key(), &serialized_data, &msg.base.sender_sig),
        }
    }

    fn verify_content(&self, receiver_state: &S) -> bool {
        match self {
            Message::Msg1a(msg) => msg.base.sender == receiver_state.get_current_leader(),
            Message::Msg1b(msg) => true,
            Message::Msg1c(msg) => {
                let mut highest_ballot = 0;
                let mut vote: Option<S::Vote> = None;

                for proof in &msg.proofs {
                    if proof.highest_ballot > highest_ballot {
                        highest_ballot = proof.highest_ballot;
                        vote = proof.vote.clone();
                    }
                }

                msg.vote == vote.expect("No valid vote found")
            },
            Message::Msg2a(msg) => true,
            Message::Msg2av(msg) => true,
            Message::Msg2b(msg) => true,
        }
    }

    fn apply_content(&self, receiver_state: &mut S) {
        match self {
            Message::Msg1a(msg) => {},
            Message::Msg1b(msg) => {/* TODO */},
            Message::Msg1c(msg) => {
                let mut highest_ballot = 0;
                let mut vote: Option<S::Vote> = None;

                for proof in &msg.proofs {
                    if proof.highest_ballot > highest_ballot {
                        highest_ballot = proof.highest_ballot;
                        vote = proof.vote.clone();
                    }
                }
                let vote = vote.expect("No valid vote found");
                receiver_state.set_vote(vote);
            },
            Message::Msg2a(msg) => {},
            Message::Msg2av(msg) => {
                let mut messages = receiver_state.get_2av_messages();
                messages.push(msg.clone());

                use std::collections::HashMap;
                let mut vote_counts: HashMap<S::Vote, usize> = HashMap::new();
                for message in messages.iter() {
                    *vote_counts.entry(message.vote.clone()).or_insert(0) += 1;
                }

                // Check if any vote reaches the quorum size
                if let Some(vote) = vote_counts.iter().find(|(_, &count)| count >= receiver_state.get_quorum_size()).map(|(vote, _)| vote.clone()) {
                    receiver_state.set_vote(vote);
                }
            },
            Message::Msg2b(msg) => {},
        }
    }

    fn serialize_for_signing(&self) -> Vec<u8> {
        match self {
            Message::Msg1a(msg) => {vec![]},
            Message::Msg1b(msg) => {vec![]},
            Message::Msg1c(msg) => {vec![]},
            Message::Msg2a(msg) => {vec![]},            
            Message::Msg2av(msg) => {vec![]},
            Message::Msg2b(msg) => {vec![]},
        }
    }
}
use std::sync::mpsc::{Receiver, Sender};
use std::thread;
use crate::Identifier;
use crate::message::{Message, ProtocolMessage};

/// Party of the Ballot.
struct Party<I: Identifier, M: Message<I>> {
    /// This party's identifier.
    pub id: I,
    /// Other ballot parties' ids.
    pub party_ids: Vec<I>,
    /// Other ballot parties.
    pub parties: Vec<Self>,

    in_tx: Sender<M>,
    in_rx: Receiver<M>,

    out_tx: Sender<M>,
    out_rx: Receiver<M>,

    cancel: bool,
}

impl<I: Identifier, M: Message<I>> Party<I, M> {
    /// Start the party.
    pub fn start(&mut self){
        thread::spawn(move || {
            loop{
                match self.cancel{
                    true => break,
                    false => {
                        for msg in self.out_rx{

                        }
                    },
                }
            }
        });
    }

    /// Receive message.
    pub fn receive_message<M: Message<I>>(&mut self, msg: &M){
        self.in_tx.send(msg).unwrap();

        thread::spawn(move || {
            for received in self.in_rx {
                self.update_state(received);
            }
        });
    }

    /// Update party's state based on message type.
    fn update_state<M: Message<I>>(&mut self, msg: &M){
        // TODO: Implement according to protocol rules.
        match msg{
            ProtocolMessage::Msg1a => {},
            ProtocolMessage::Msg1b => {},
            ProtocolMessage::Msg1c => {},
            ProtocolMessage::Msg2a => {},
            ProtocolMessage::Msg2av => {},
            ProtocolMessage::Msg2b => {},
            _ => {panic!("unknown message type")},
        }
    }

}
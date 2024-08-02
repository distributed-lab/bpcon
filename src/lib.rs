use std::collections::HashMap;
use serde::{Deserialize, Serialize};

pub mod message;
pub mod party;
mod error;

/// General trait for value itself.
pub trait Value: Eq + Serialize + for<'a> Deserialize<'a> + Clone {}

/// Trait for value selector and verificator.
/// Value selection and verification may depend on different conditions for different values.
/// Note that value selection should follow the rules of BPCon: only safe values can be selected.
/// Party can not vote for different values, even in different ballots.
pub trait ValueSelector<V: Value> {
    /// Verifies if a value is selected correctly. Accepts 2b messages from parties.
    fn verify(&self, v: &V, m: &HashMap<u64, Option<Vec<u8>>>) -> bool;

    /// Select value depending on inner conditions. Accepts 2b messages from parties.
    fn select(&self, m: &HashMap<u64, Option<Vec<u8>>>) -> V;

    // TODO: add other fields to update selector state.
}

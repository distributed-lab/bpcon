//! Definitions central to BPCon values.
//!
//! This module defines traits and structures related to the values used within the BPCon consensus protocol.
//! It includes a general `Value` trait for handling values in the protocol, as well as a `ValueSelector` trait
//! for implementing value selection and verification logic.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::{Debug, Display};

/// A general trait representing a value in the BPCon consensus protocol.
///
/// Implementing this trait ensures that values can be safely transmitted and logged during the
/// consensus process.
pub trait Value: Eq + Serialize + for<'a> Deserialize<'a> + Clone + Debug + Display {}

/// A trait for selecting and verifying values in the BPCon consensus protocol.
///
/// The `ValueSelector` trait provides the functionality for verifying that a value has been
/// selected according to the rules of the protocol and for selecting a value based on
/// specific conditions.
///
/// ## Important Rules:
/// - **Safety**: Value selection must adhere to the BPCon protocol rules, meaning only "safe" values
///   can be selected. This implies that a party should not vote for different values, even across
///   different ballots.
/// - **Consensus Compliance**: The selection process should consider the state of messages (typically
///   from 1b messages) sent by other parties, ensuring the selected value is compliant with the
///   collective state of the consensus.
///
/// # Type Parameters
/// - `V`: The type of value being selected and verified. This must implement the `Value` trait.
pub trait ValueSelector<V: Value>: Clone {
    /// Verifies if a value has been correctly selected.
    ///
    /// This method checks if the provided value `v` is selected according to the protocol's rules.
    /// The verification process typically involves examining 1b messages from other parties, contained
    /// within the `HashMap`.
    ///
    /// # Parameters
    /// - `v`: The value to verify.
    /// - `m`: A `HashMap` mapping party IDs (`u64`) to their corresponding optional values (`Option<V>`).
    ///
    /// # Returns
    /// `true` if the value is correctly selected; otherwise, `false`.
    fn verify(&self, v: &V, m: &HashMap<u64, Option<V>>) -> bool;

    /// Selects a value based on internal conditions and messages from other parties.
    ///
    /// This method determines the value to be selected for the consensus process, based on the state
    /// of 1b messages received from other parties. The selection must comply with the BPCon protocol's
    /// safety and consistency requirements.
    ///
    /// # Parameters
    /// - `m`: A `HashMap` mapping party IDs (`u64`) to their corresponding optional values (`Option<V>`).
    ///
    /// # Returns
    /// The selected value of type `V`.
    fn select(&self, m: &HashMap<u64, Option<V>>) -> V;
}

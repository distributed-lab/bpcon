//! Definitions central to BPCon leader election logic.
//!
//! This module defines the traits and structures involved in leader election within the BPCon consensus protocol.
//! The leader election process is crucial in BPCon, ensuring that a single leader is chosen to coordinate the consensus rounds.

use crate::party::Party;
use crate::value::{Value, ValueSelector};
use seeded_random::{Random, Seed};
use std::cmp::Ordering;
use std::hash::{DefaultHasher, Hash, Hasher};
use thiserror::Error;

/// A trait that encapsulates the logic for leader election in the BPCon protocol.
///
/// Implementors of this trait provide the mechanism to elect a leader from the participating parties
/// for a given ballot round. The elected leader is responsible for driving the consensus process
/// during that round.
///
/// # Type Parameters
/// - `V`: The type of values being proposed and agreed upon in the consensus process, which must implement the `Value` trait.
/// - `VS`: The type of value selector used to choose values during the consensus process, which must implement the `ValueSelector<V>` trait.
pub trait LeaderElector<V: Value, VS: ValueSelector<V>>: Send {
    /// Elects a leader for the current ballot.
    ///
    /// This method returns the ID of the party elected as the leader for the current ballot round.
    ///
    /// # Parameters
    /// - `party`: A reference to the `Party` instance containing information about the current ballot and participating parties.
    ///
    /// # Returns
    /// - `Ok(u64)`: The ID of the elected party.
    /// - `Err(Box<dyn std::error::Error>)`: An error if leader election fails.
    fn elect_leader(&self, party: &Party<V, VS>) -> Result<u64, Box<dyn std::error::Error>>;
}

/// A default implementation of the `LeaderElector` trait for BPCon.
///
/// This struct provides a standard mechanism for leader election based on weighted randomization,
/// ensuring deterministic leader selection across multiple rounds.
#[derive(Clone, Debug, Default)]
pub struct DefaultLeaderElector {}

impl DefaultLeaderElector {
    /// Creates a new `DefaultLeaderElector` instance.
    pub fn new() -> Self {
        Self::default()
    }

    /// Computes a seed for randomized leader election.
    ///
    /// This method generates a seed value based on the party configuration and the current ballot,
    /// ensuring that leader election is deterministic but randomized.
    ///
    /// # Parameters
    /// - `party`: A reference to the `Party` instance containing information about the current ballot and participating parties.
    ///
    /// # Returns
    /// A `u64` seed value for use in leader election.
    fn compute_seed<V: Value, VS: ValueSelector<V>>(party: &Party<V, VS>) -> u64 {
        let mut hasher = DefaultHasher::new();

        // Hash each field that should contribute to the seed
        party.cfg.party_weights.hash(&mut hasher);
        party.cfg.threshold.hash(&mut hasher);
        party.ballot.hash(&mut hasher);

        // Generate the seed from the hash
        hasher.finish()
    }

    /// Hashes the seed to a value within a specified range.
    ///
    /// This method uses the computed seed to generate a value within the range [0, range).
    /// The algorithm ensures uniform distribution of the resulting value, which is crucial
    /// for fair leader election.
    ///
    /// # Parameters
    /// - `seed`: The seed value used for randomization.
    /// - `range`: The upper limit for the random value generation, typically the sum of party weights.
    ///
    /// # Returns
    /// A `u64` value within the specified range.
    fn hash_to_range(seed: u64, range: u64) -> u64 {
        // Determine the number of bits required to represent the range
        let mut k = 64;
        while 1u64 << (k - 1) >= range {
            k -= 1;
        }

        // Use a seeded random generator to produce a value within the desired range
        let rng = Random::from_seed(Seed::unsafe_new(seed));
        loop {
            let mut raw_res: u64 = rng.gen();
            raw_res >>= 64 - k;

            if raw_res < range {
                return raw_res;
            }
            // If the generated value is not within range, repeat the process
        }
    }
}

/// Errors that can occur during leader election using the `DefaultLeaderElector`.
#[derive(Error, Debug)]
pub enum DefaultLeaderElectorError {
    /// Error indicating that the sum of party weights is zero, making leader election impossible.
    #[error("zero weight sum")]
    ZeroWeightSum,
}

impl<V: Value, VS: ValueSelector<V>> LeaderElector<V, VS> for DefaultLeaderElector {
    /// Elects a leader in a weighted randomized manner.
    ///
    /// This method uses the computed seed and party weights to select a leader. The process is deterministic
    /// due to the use of a fixed seed but allows for randomized leader selection based on the weight distribution.
    ///
    /// # Parameters
    /// - `party`: A reference to the `Party` instance containing information about the current ballot and participating parties.
    ///
    /// # Returns
    /// - `Ok(u64)`: The ID of the elected party.
    /// - `Err(Box<dyn std::error::Error>)`: An error if the leader election process fails, such as when the total weight is zero.
    fn elect_leader(&self, party: &Party<V, VS>) -> Result<u64, Box<dyn std::error::Error>> {
        let seed = DefaultLeaderElector::compute_seed(party);

        let total_weight: u64 = party.cfg.party_weights.iter().sum();
        if total_weight == 0 {
            return Err(DefaultLeaderElectorError::ZeroWeightSum.into());
        }

        // Generate a random number in the range [0, total_weight)
        let random_value = DefaultLeaderElector::hash_to_range(seed, total_weight);

        // Use binary search to find the corresponding participant based on the cumulative weight
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::party::tests::{default_config, default_party};
    use rand::Rng;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_default_leader_elector_determinism() {
        let party = default_party();
        let elector = DefaultLeaderElector::new();

        let leader1 = elector.elect_leader(&party).unwrap();

        // Test multiple iterations to ensure the leader remains the same
        for i in 2..=10 {
            let leader = elector.elect_leader(&party).unwrap();
            assert_eq!(
                leader1, leader,
                "Leaders should be consistent on repeated calls (iteration {})",
                i
            );
        }
    }

    #[test]
    fn test_default_leader_elector_fail_with_zero_weights() {
        let mut party = default_party();
        let mut cfg = default_config();
        cfg.party_weights = vec![0, 0, 0];
        party.cfg = cfg;

        let elector = DefaultLeaderElector::new();

        match elector.elect_leader(&party) {
            Err(_) => {} // This is the expected behavior
            _ => panic!("Expected DefaultLeaderElectorError::ZeroWeightSum"),
        }
    }

    fn debug_hash_to_range_new(seed: u64, range: u64) -> u64 {
        assert!(range > 1);

        let mut k = 64;
        while 1u64 << (k - 1) >= range {
            k -= 1;
        }

        let rng = Random::from_seed(Seed::unsafe_new(seed));

        let mut iteration = 1u64;
        loop {
            let mut raw_res: u64 = rng.gen();
            raw_res >>= 64 - k;

            if raw_res < range {
                return raw_res;
            }

            iteration += 1;
            assert!(iteration <= 50)
        }
    }

    #[test]
    #[ignore] // Ignoring since it takes a while to run
    fn test_hash_range_random() {
        // Test the uniform distribution

        const N: usize = 37;
        const M: i64 = 10000000;

        let mut cnt1: [i64; N] = [0; N];

        for _ in 0..M {
            let mut rng = rand::thread_rng();
            let seed: u64 = rng.random();

            let res1 = debug_hash_to_range_new(seed, N as u64);
            assert!(res1 < N as u64);

            cnt1[res1 as usize] += 1;
        }

        println!("1: {:?}", cnt1);

        let mut avg1: i64 = 0;

        for item in cnt1.iter().take(N) {
            avg1 += (M / (N as i64) - item).abs();
        }

        avg1 /= N as i64;

        println!("Avg 1: {}", avg1);
    }

    #[test]
    fn test_rng() {
        let rng1 = Random::from_seed(Seed::unsafe_new(123456));
        let rng2 = Random::from_seed(Seed::unsafe_new(123456));

        println!("{}", rng1.gen::<u64>());
        println!("{}", rng2.gen::<u64>());

        thread::sleep(Duration::from_secs(2));

        println!("{}", rng1.gen::<u64>());
        println!("{}", rng2.gen::<u64>());
    }
}

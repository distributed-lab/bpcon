//! Definitions central to BPCon leader election logic.
//!
//! This module defines the traits and structures involved in leader election within the BPCon consensus protocol.
//! The leader election process is crucial in BPCon, ensuring that a single leader is chosen to coordinate the consensus rounds.

use crate::party::Party;
use crate::value::{Value, ValueSelector};
use rand_chacha::rand_core::{RngCore, SeedableRng};
use rand_chacha::ChaCha20Rng;
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
    /// This method uses the computed seed to generate a value within the range [0, range].
    /// The algorithm ensures uniform distribution of the resulting value, which is crucial
    /// for fair leader election.
    ///
    /// # Parameters
    /// - `seed`: The seed value used for randomization.
    /// - `range`: The upper limit for the random value generation, typically the sum of party weights.
    ///
    /// # Returns
    /// A `u128` value within the specified range.
    fn hash_to_range(seed: u64, range: u128) -> u128 {
        // Determine the number of bits required to represent the range
        let mut k = 128;
        while 1u128 << (k - 1) > range {
            k -= 1;
        }

        // Use a seeded random generator to produce a value within the desired range
        let mut rng = ChaCha20Rng::seed_from_u64(seed);
        loop {
            let mut raw_res = ((rng.next_u64() as u128) << 64) | (rng.next_u64() as u128);
            raw_res >>= 128 - k;

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

        let total_weight: u128 = party.cfg.party_weights.iter().map(|&x| x as u128).sum();
        if total_weight == 0 {
            return Err(DefaultLeaderElectorError::ZeroWeightSum.into());
        }

        // Generate a random number in the range [0, total_weight]
        let random_value = DefaultLeaderElector::hash_to_range(seed, total_weight);

        let mut cumulative_sum = 0u128;
        for (index, &weight) in party.cfg.party_weights.iter().enumerate() {
            cumulative_sum += weight as u128;
            if random_value <= cumulative_sum {
                return Ok(index as u64);
            }
        }

        unreachable!("Index is guaranteed to be returned in a loop.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BPConConfig;
    use crate::test_mocks::MockParty;
    use rand::Rng;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_default_leader_elector_weight_one() {
        let mut party = MockParty::default();
        party.cfg.party_weights = vec![0, 1, 0, 0];

        let elector = DefaultLeaderElector::new();

        let leader = elector.elect_leader(&party).unwrap();
        println!("leader: {}", leader);
    }

    #[test]
    fn test_default_leader_elector_determinism() {
        let party = MockParty::default();
        let elector = DefaultLeaderElector::new();

        const ITERATIONS: usize = 10;

        // Collect multiple leaders
        let leaders: Vec<_> = (0..ITERATIONS)
            .map(|_| elector.elect_leader(&party).unwrap())
            .collect();

        // Match the first leader and ensure all others are the same
        match &leaders[..] {
            [first_leader, rest @ ..] => {
                assert!(
                    rest.iter().all(|leader| leader == first_leader),
                    "All leaders should be the same across multiple iterations."
                );
            }
            _ => panic!("No leaders were collected!"),
        }
    }

    #[test]
    fn test_default_leader_elector_fail_with_zero_weights() {
        let mut party = MockParty::default();
        let cfg = BPConConfig {
            party_weights: vec![0, 0, 0],
            ..Default::default()
        };
        party.cfg = cfg;
        let elector = DefaultLeaderElector::new();

        assert!(
            elector.elect_leader(&party).is_err(),
            "Expected DefaultLeaderElectorError::ZeroWeightSum"
        );
    }

    fn debug_hash_to_range_new(seed: u64, range: u64) -> u64 {
        assert!(range > 1);

        let mut k = 64;
        while 1u64 << (k - 1) >= range {
            k -= 1;
        }

        let mut rng = ChaCha20Rng::seed_from_u64(seed);

        let mut iteration = 1u64;
        loop {
            let mut raw_res: u64 = rng.next_u64();
            raw_res >>= 64 - k;

            if raw_res < range {
                return raw_res;
            }

            iteration += 1;
            assert!(iteration <= 50)
        }
    }

    #[test]
    #[ignore = "takes too long to run, launch manually"]
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
        let mut rng1 = ChaCha20Rng::seed_from_u64(123456);
        let mut rng2 = ChaCha20Rng::seed_from_u64(123456);

        println!("{}", rng1.next_u64());
        println!("{}", rng2.next_u64());

        thread::sleep(Duration::from_secs(2));

        println!("{}", rng1.next_u64());
        println!("{}", rng2.next_u64());
    }
}

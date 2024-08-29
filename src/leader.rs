use crate::party::Party;
use crate::value::{Value, ValueSelector};
use seeded_random::{Random, Seed};
use std::cmp::Ordering;
use std::hash::{DefaultHasher, Hash, Hasher};
use thiserror::Error;

/// Trait incorporating logic for leader election.
pub trait LeaderElector<V: Value, VS: ValueSelector<V>>: Send {
    /// Get leader for current ballot.
    /// Returns id of the elected party or error.
    fn elect_leader(&self, party: &Party<V, VS>) -> Result<u64, Box<dyn std::error::Error>>;
}

#[derive(Clone, Debug, Default)]
pub struct DefaultLeaderElector {}

impl DefaultLeaderElector {
    pub fn new() -> Self {
        Self::default()
    }

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
            k -= 1;
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
            raw_res >>= 64 - k;

            if raw_res < range {
                return raw_res;
            }
            // Executing this loop does not require a large number of iterations.
            // Check tests for more info
        }
    }
}

#[derive(Error, Debug)]
pub enum DefaultLeaderElectorError {
    #[error("zero weight sum")]
    ZeroWeightSum,
}

impl<V: Value, VS: ValueSelector<V>> LeaderElector<V, VS> for DefaultLeaderElector {
    /// Compute leader in a weighed randomized manner.
    /// Uses seed from the config, making it deterministic.
    fn elect_leader(&self, party: &Party<V, VS>) -> Result<u64, Box<dyn std::error::Error>> {
        let seed = DefaultLeaderElector::compute_seed(party);

        let total_weight: u64 = party.cfg.party_weights.iter().sum();
        if total_weight == 0 {
            return Err(DefaultLeaderElectorError::ZeroWeightSum.into());
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
            Err(_) => {} // this is expected behaviour
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
        // test the uniform distribution

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

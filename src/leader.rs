use crate::party::Party;
use crate::{Value, ValueSelector};
use seeded_random::{Random, Seed};
use std::cmp::Ordering;
use std::hash::{DefaultHasher, Hash, Hasher};

/// Trait incorporating logic for leader election.
pub trait LeaderElector<V: Value, VS: ValueSelector<V>>: Send {
    type LeaderElectorError;

    /// Get leader for current ballot.
    /// Returns id of the elected party or error.
    fn get_leader(&self, party: &Party<V, VS>) -> Result<u64, Self::LeaderElectorError>;
}

#[derive(Clone, Debug)]
pub struct DefaultLeaderElector {}

impl DefaultLeaderElector {
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

pub enum DefaultLeaderElectorError {
    ZeroWeightSum,
}
//
// impl Display for DefaultLeaderElectorError {
//     fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
//         match *self {
//             DefaultLeaderElectorError::ZeroWeightSum => write!(f, "Leader election error: zero weight sum"),
//        }
//     }
// }

impl<V: Value, VS: ValueSelector<V>> LeaderElector<V, VS> for DefaultLeaderElector {
    type LeaderElectorError = DefaultLeaderElectorError;

    /// Compute leader in a weighed randomized manner.
    /// Uses seed from the config, making it deterministic.
    fn get_leader(&self, party: &Party<V, VS>) -> Result<u64, Self::LeaderElectorError> {
        let seed = DefaultLeaderElector::compute_seed(party);

        let total_weight: u64 = party.cfg.party_weights.iter().sum();
        if total_weight == 0 {
            return Err(DefaultLeaderElectorError::ZeroWeightSum);
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

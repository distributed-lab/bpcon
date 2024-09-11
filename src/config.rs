//! Definitions central to BPCon configuration.

use std::time::Duration;

/// Configuration structure for BPCon.
///
/// `BPConConfig` holds the various timeouts and weight configurations necessary
/// for the BPCon consensus protocol. This configuration controls the timing
/// of different stages in the protocol and the distribution of party weights.
#[derive(PartialEq, Eq, Debug, Clone)]
pub struct BPConConfig {
    /// Parties weights: `party_weights[i]` corresponds to the i-th party's weight.
    ///
    /// These weights determine the influence each party has in the consensus process.
    /// The sum of these weights is used to calculate the `threshold` for reaching a
    /// Byzantine Fault Tolerant (BFT) quorum.
    pub party_weights: Vec<u64>,

    /// Threshold weight to define BFT quorum.
    ///
    /// This value must be greater than or equal to 2/3 of the total weight of all parties combined.
    /// The quorum is the minimum weight required to make decisions in the BPCon protocol.
    pub threshold: u128,

    /// Timeout before the ballot is launched.
    ///
    /// This timeout differs from `launch1a_timeout` as it applies to a distinct status
    /// and does not involve listening to external events and messages.
    pub launch_timeout: Duration,

    /// Timeout before the 1a stage is launched.
    ///
    /// The 1a stage is the first phase of the BPCon consensus process, where leader
    /// is informing other participants about the start of the ballot.
    /// This timeout controls the delay before starting this stage.
    pub launch1a_timeout: Duration,

    /// Timeout before the 1b stage is launched.
    ///
    /// In the 1b stage, participants exchange their last voted ballot number and elected value.
    /// This timeout controls the delay before starting this stage.
    pub launch1b_timeout: Duration,

    /// Timeout before the 2a stage is launched.
    ///
    /// The 2a stage involves leader proposing a selected value and
    /// other participants verifying it.
    /// This timeout controls the delay before starting this stage.
    pub launch2a_timeout: Duration,

    /// Timeout before the 2av stage is launched.
    ///
    /// The 2av stage is where 2a obtained value shall gather needed weight to pass.
    /// This timeout controls the delay before starting this stage.
    pub launch2av_timeout: Duration,

    /// Timeout before the 2b stage is launched.
    ///
    /// The 2b stage is where the final value is chosen and broadcasted.
    /// This timeout controls the delay before starting this stage.
    pub launch2b_timeout: Duration,

    /// Timeout before the finalization stage is launched.
    ///
    /// The finalization stage is not a part of protocol, but rather internal-centric mechanics
    /// to conclude ballot.
    /// This timeout controls the delay before starting this stage.
    pub finalize_timeout: Duration,

    /// Timeout for a graceful period to accommodate parties with latency.
    ///
    /// This grace period allows parties with slower communication or processing times
    /// to catch up, helping to ensure that all parties can participate fully in the
    /// consensus process.
    pub grace_period: Duration,
}

impl BPConConfig {
    /// Create a new `BPConConfig` with default timeouts.
    ///
    /// This method initializes a `BPConConfig` instance using default timeout values for each
    /// stage of the BPCon consensus protocol. These defaults are placeholders and should be
    /// tuned according to the specific needs and characteristics of the network.
    ///
    /// # Parameters
    ///
    /// - `party_weights`: A vector of weights corresponding to each party involved in the consensus.
    /// - `threshold`: The weight threshold required to achieve a BFT quorum.
    ///
    /// # Returns
    ///
    /// A new `BPConConfig` instance with the provided `party_weights` and `threshold`,
    /// and default timeouts for all stages.
    pub fn with_default_timeouts(party_weights: Vec<u64>, threshold: u128) -> Self {
        Self {
            party_weights,
            threshold,
            // TODO: deduce actually good defaults.
            launch_timeout: Duration::from_millis(0),
            launch1a_timeout: Duration::from_millis(200),
            launch1b_timeout: Duration::from_millis(400),
            launch2a_timeout: Duration::from_millis(600),
            launch2av_timeout: Duration::from_millis(800),
            launch2b_timeout: Duration::from_millis(1000),
            finalize_timeout: Duration::from_millis(1200),
            grace_period: Duration::from_millis(0),
        }
    }

    /// Compute the Byzantine Fault Tolerance (BFT) threshold for the consensus protocol.
    ///
    /// This function calculates the minimum weight required to achieve a BFT quorum.
    /// In BFT systems, consensus is typically reached when more than two-thirds
    /// of the total weight is gathered from non-faulty parties.
    ///
    /// # Parameters
    ///
    /// - `party_weights`: A vector of weights corresponding to each party involved in the consensus.
    ///   These weights represent the voting power or influence of each party in the protocol.
    ///
    /// # Returns
    ///
    /// The BFT threshold as a `u128` value, which represents the minimum total weight
    /// required to achieve consensus in a Byzantine Fault Tolerant system. This is calculated
    /// as two-thirds of the total party weights.
    ///
    /// # Example
    ///
    /// ```
    /// use bpcon::config::BPConConfig;
    ///
    /// let party_weights = vec![10, 20, 30, 40, 50];
    /// let threshold = BPConConfig::compute_bft_threshold(party_weights);
    /// assert_eq!(threshold, 100);
    /// ```
    ///
    /// In the example above, the total weight is 150, and the BFT threshold is calculated as `2/3 * 150 = 100`.
    pub fn compute_bft_threshold(party_weights: Vec<u64>) -> u128 {
        let total_weight: u128 = party_weights.iter().map(|&w| w as u128).sum();
        (2 * total_weight + 2) / 3 // adding 2 to keep division ceiling.
    }
}

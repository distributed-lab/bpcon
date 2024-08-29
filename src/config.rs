use std::time::Duration;

/// BPCon configuration. Includes ballot time bounds and other stuff.
#[derive(PartialEq, Eq, Debug, Clone)]
pub struct BPConConfig {
    /// Parties weights: `party_weights[i]` corresponds to the i-th party weight
    pub party_weights: Vec<u64>,

    /// Threshold weight to define BFT quorum: should be > 2/3 of total weight
    pub threshold: u128,

    /// Timeout before ballot is launched.
    /// Differs from `launch1a_timeout` having another status and not listening
    /// to external events and messages.
    pub launch_timeout: Duration,

    /// Timeout before 1a stage is launched.
    pub launch1a_timeout: Duration,

    /// Timeout before 1b stage is launched.
    pub launch1b_timeout: Duration,

    /// Timeout before 2a stage is launched.
    pub launch2a_timeout: Duration,

    /// Timeout before 2av stage is launched.
    pub launch2av_timeout: Duration,

    /// Timeout before 2b stage is launched.
    pub launch2b_timeout: Duration,

    /// Timeout before finalization stage is launched.
    pub finalize_timeout: Duration,

    /// Timeout for a graceful period to help parties with latency.
    pub grace_period: Duration,
}

impl BPConConfig {
    pub fn with_default_timeouts(party_weights: Vec<u64>, threshold: u128) -> Self {
        Self {
            party_weights,
            threshold,
            // TODO: deduce actually good defaults
            launch_timeout: Duration::from_secs(0),
            launch1a_timeout: Duration::from_secs(5),
            launch1b_timeout: Duration::from_secs(10),
            launch2a_timeout: Duration::from_secs(15),
            launch2av_timeout: Duration::from_secs(20),
            launch2b_timeout: Duration::from_secs(25),
            finalize_timeout: Duration::from_secs(30),
            grace_period: Duration::from_secs(1),
        }
    }
}

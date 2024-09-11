use crate::config::BPConConfig;
use crate::leader::DefaultLeaderElector;
use crate::party::Party;
use crate::value::{Value, ValueSelector};
use std::collections::HashMap;
use std::fmt;

#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Debug, Default)]
pub struct MockValue(u64);

impl fmt::Display for MockValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MockValue: {}", self.0)
    }
}

impl Value for MockValue {}

#[derive(Clone, Default)]
pub struct MockValueSelector;

impl ValueSelector<MockValue> for MockValueSelector {
    fn verify(&self, _: &MockValue, _: &HashMap<u64, Option<MockValue>>) -> bool {
        true // For testing, always return true.
    }

    fn select(&self, _: &HashMap<u64, Option<MockValue>>) -> MockValue {
        MockValue(1) // For testing, always return the same value.
    }
}

impl Default for BPConConfig {
    fn default() -> Self {
        let weights = vec![1, 1, 1, 1];
        let threshold = BPConConfig::compute_bft_threshold(weights.clone());
        BPConConfig::with_default_timeouts(weights, threshold)
    }
}

pub type MockParty = Party<MockValue, MockValueSelector>;

impl Default for MockParty {
    fn default() -> Self {
        MockParty::new(
            Default::default(),
            Default::default(),
            Default::default(),
            Box::new(DefaultLeaderElector::default()),
        )
        .0
    }
}

# Weighted BPCon Rust Library

[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](https://www.gnu.org/licenses/gpl-3.0)
[![Rust CI🌌](https://github.com/distributed-lab/bpcon/actions/workflows/rust.yml/badge.svg)](https://github.com/distributed-lab/bpcon/actions/workflows/rust.yml)
[![Docs 🌌](https://github.com/distributed-lab/bpcon/actions/workflows/docs.yml/badge.svg)](https://github.com/distributed-lab/bpcon/actions/workflows/docs.yml)

> This is a rust library implementing weighted BPCon consensus.

## Documentation

Library is documented with `rustdoc`.
Compiled documentation for `main` branch is available at [GitBook](https://distributed-lab.github.io/bpcon).

## Usage

### Add dependency in your `Cargo.toml`

```toml
[dependencies]
bpcon = {version = "0.1.0", git = "https://github.com/distributed-lab/bpcon"}
```

### Implement [Value](https://distributed-lab.github.io/bpcon/bpcon/value/trait.Value.html) trait

This is a core trait, which defines what type are you selecting in your consensus.
It may be the next block in blockchain, or leader for some operation, or anything else you need.

Below is a simple example, where we will operate on selection for `u64` type.
Using it you may interpret `ID` for leader of distributed operation, for instance.

```rust
...
use bpcon::value::Value;

#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Debug, Hash)]
pub(crate) struct MyValue(u64);

impl Value for MyValue {}
```

### Implement [ValueSelector](https://distributed-lab.github.io/bpcon/bpcon/value/trait.ValueSelector.html) trait

`BPCon` allows you to define specific conditions how proposer (leader) will select value
and how other members will verify its selection.

Here is a simple example:

```rust
...
use bpcon::value::ValueSelector;

#[derive(Clone)]
pub struct MyValueSelector;

impl ValueSelector<MyValue> for MyValueSelector {
    /// Verifies if the given value `v` has been correctly selected according to the protocol rules.
    ///
    /// In this basic implementation, we'll consider a value as "correctly selected" if it matches
    /// the majority of the values in the provided `HashMap`.
    fn verify(&self, v: &MyValue, m: &HashMap<u64, Option<MyValue>>) -> bool {
        // Count how many times the value `v` appears in the `HashMap`
        let count = m.values().filter(|&val| val.as_ref() == Some(v)).count();

        // For simplicity, consider the value verified if it appears in more than half of the entries
        count > m.len() / 2
    }

    /// Selects a value based on the provided `HashMap` of party votes.
    ///
    /// This implementation selects the value that appears most frequently in the `HashMap`.
    /// If there is a tie, it selects the smallest value (as per the natural ordering of `u64`).
    fn select(&self, m: &HashMap<u64, Option<MyValue>>) -> MyValue {
        let mut frequency_map = HashMap::new();

        // Count the frequency of each value in the `HashMap`
        for value in m.values().flatten() {
            *frequency_map.entry(value).or_insert(0) += 1;
        }

        // Find the value with the highest frequency. In case of a tie, select the smallest value.
        frequency_map
            .into_iter()
            .max_by_key(|(value, count)| (*count, value.0))
            .map(|(value, _)| value.clone())
            .expect("No valid value found")
    }
}
```

### Decide on a [LeaderElector](https://distributed-lab.github.io/bpcon/bpcon/leader/trait.LeaderElector.html)

`LeaderElector` trait allows you to define specific conditions, how to select leader for consensus.

**NOTE: it is important to provide deterministic mechanism,
because each participant will compute leader for itself,
and in case it is not deterministic, state divergence occurs.**

We also provide ready-to-use
[DefaultLeaderElector](https://distributed-lab.github.io/bpcon/bpcon/leader/struct.DefaultLeaderElector.html)
which is using weighted randomization.

### Configure your ballot

As a next step, you need to decide on parameters for your ballot:

1. Amount of parties and their weight.
2. Threshold weight.
3. Time bounds.

Example:

```rust
use bpcon::config::BPConConfig;

let cfg = BPConConfig::with_default_timeouts(vec![1, 1, 1, 1, 1, 1], 4);
```

Feel free to explore [config.rs](https://distributed-lab.github.io/bpcon/bpcon/config/struct.BPConConfig.html)
for more information.

### Create parties

Having `BPConConfig`, `ValueSelector` and `LeaderElector` defined, instantiate your parties.
Check out [new](https://distributed-lab.github.io/bpcon/bpcon/party/struct.Party.html#method.new)
method on a `Party` struct.

### Launch ballot on parties and handle messages

Each party interfaces communication with external system via channels.
In a way, you shall propagate outgoing messages to other parties like:

1. Listen for outgoing message using `msg_out_receiver`.
2. Forward it to other parties using `msg_in_sender`.

We welcome you to check our [integration tests](./tests) for examples.

## Security Considerations 🔐

### Categories of parties

In real world applications, we may categorize parties by their behavior to following:

1. Good - party sends messages to other participants based on following events,
   and correctly receives and processes messages from other parties.

2. Faulty - party has troubles receiving/sending messages.
   These are simply mitigated by the weighed threshold and redundancy of consensus participants.

3. Malicious - party launches DDoS attack using unbounded sending of messages -
   to deal with this, we introduce rate-limiting mechanism in accepting messages inside the `Party`,
   however it is also ❗️ required by integrating 'external' system ❗️, which handles `P2P`, to attest to this, because otherwise receiving channel may get flooded by malicious messages and block messages from other parties.
   Another way to cause trouble is by sending invalid messages. For this, each party has
   a set of checks for certain fields like current ballot number, status, etc.
   Additionally, if the state transition caused by incoming message errored, it does not impact the party in either way.

### Note on the leader 👑

If the `leader` of the ballot is faulty or malicious, the ballot deterministically fails and needs to be relaunched.

### Note on the communication discrepancies 🔇

Each party has a certain period in which it may accept particular messages for a certain stage
(example: having passed 1a stage, it is open for accepting only 1b messages for 2 seconds).
These periods are configurable using `BPConConfig`, meaning you can program certain ranges
to allow slow parties to catch up, while others are waiting, before transiting to the next stage.

In addition it is possible to schedule parties to launch at specific absolute time.

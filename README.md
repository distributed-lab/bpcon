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

### Implement `Value` trait for consensus subject

```rust
...
use bpcon::value::Value;

#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Debug, Hash)]
pub(crate) struct MyValue(u64);

impl Value for MyValue {}
```

### Implement `ValueSelector` trait

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

### Decide on a `LeaderElector`

We provide functionality to define your own rules for leader election using corresponding trait.
Note, that there is already default `LeaderElector` available.

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

Feel free to explore `config.rs` for more information.

### Create parties

Having `BPConConfig`, `ValueSelector` and `LeaderElector` defined, instantiate your parties. 
Check out `new` method on a `Party` struct.

### Launch ballot on parties and handle messages

Each party interfaces communication with external system via channels. 
In a way, you shall propagate outgoing messages to other parties like:

1. Listen for outgoing message using `msg_out_receiver`.
2. Forward it to other parties using `msg_in_sender`.

We welcome you to check `test_end_to_end_ballot` in `party.rs` for example.

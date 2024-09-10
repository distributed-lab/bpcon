use bpcon::config::BPConConfig;
use bpcon::error::LaunchBallotError;
use bpcon::leader::{DefaultLeaderElector, LeaderElector};
use bpcon::message::{Message1bContent, MessagePacket};

use bpcon::test_mocks::{MockParty, MockValue, MockValueSelector};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration, Instant};

use futures::future::join_all;

/// Here each party/receiver/sender shall correspond at equal indexes.
type PartiesWithChannels = (
    Vec<MockParty>,
    Vec<UnboundedReceiver<MessagePacket>>,
    Vec<UnboundedSender<MessagePacket>>,
);

/// Create test parties with predefined generics, based on config.
fn create_parties(cfg: BPConConfig) -> PartiesWithChannels {
    (0..cfg.party_weights.len())
        .map(|i| {
            MockParty::new(
                i as u64,
                cfg.clone(),
                MockValueSelector,
                Box::new(DefaultLeaderElector::new()),
            )
        })
        .fold(
            (Vec::new(), Vec::new(), Vec::new()),
            |(mut parties, mut receivers, mut senders), (p, r, s)| {
                parties.push(p);
                receivers.push(r);
                senders.push(s);
                (parties, receivers, senders)
            },
        )
}

/// Begin ballot process for each party.
fn launch_parties(
    parties: Vec<MockParty>,
) -> Vec<JoinHandle<Result<MockValue, LaunchBallotError>>> {
    parties
        .into_iter()
        .map(|mut party| tokio::spawn(async move { party.launch_ballot().await }))
        .collect()
}

/// Collect messages from receivers.
fn collect_messages(receivers: &mut [UnboundedReceiver<MessagePacket>]) -> Vec<MessagePacket> {
    receivers
        .iter_mut()
        .filter_map(|receiver| receiver.try_recv().ok())
        .collect()
}

/// Broadcast collected messages to other parties, skipping the sender.
fn broadcast_messages(messages: Vec<MessagePacket>, senders: &[UnboundedSender<MessagePacket>]) {
    messages.iter().for_each(|msg| {
        senders
            .iter()
            .enumerate()
            .filter(|(i, _)| msg.routing.sender != *i as u64) // Skip the current party (sender).
            .for_each(|(_, sender_into)| {
                sender_into.send(msg.clone()).unwrap();
            });
    });
}

/// Propagate messages peer-to-peer between parties using their channels.
fn propagate_p2p(
    mut receivers: Vec<UnboundedReceiver<MessagePacket>>,
    senders: Vec<UnboundedSender<MessagePacket>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let messages = collect_messages(receivers.as_mut_slice());
            broadcast_messages(messages, &senders);

            // Delay to simulate network latency and reduce processor load.
            sleep(Duration::from_millis(100)).await;
        }
    })
}

/// Await completion of each party's process and aggregate results.
async fn await_results(
    tasks: Vec<JoinHandle<Result<MockValue, LaunchBallotError>>>,
) -> Vec<Result<MockValue, LaunchBallotError>> {
    join_all(tasks)
        .await
        .into_iter()
        .map(|res| res.unwrap())
        .collect()
}

/// Assert consensus reached and log all errors.
fn analyze_ballot(results: Vec<Result<MockValue, LaunchBallotError>>) {
    // Partition the results into successful values and errors.
    let (successful, errors): (Vec<_>, Vec<_>) = results.into_iter().partition(|res| res.is_ok());

    // Log errors if any.
    if !errors.is_empty() {
        for err in errors.into_iter() {
            eprintln!("Error during ballot: {:?}", err.unwrap_err());
        }
    }

    if successful.is_empty() {
        panic!("No consensus, all parties failed.");
    }

    // Extract the successful values.
    let values: Vec<MockValue> = successful.into_iter().map(|res| res.unwrap()).collect();

    // Check if we reached consensus: all values should be the same.
    if let Some((first_value, rest)) = values.split_first() {
        let all_agreed = rest.iter().all(|v| v == first_value);
        assert!(
            all_agreed,
            "No consensus, different values found: {:?}",
            values
        );
        println!("Consensus reached with value: {:?}", first_value);
    }
}

/// Run ballot on given parties, simulating faulty behavior for given `faulty_ids`.
async fn run_ballot_faulty_party(
    parties: PartiesWithChannels,
    faulty_ids: Vec<usize>,
) -> Vec<Result<MockValue, LaunchBallotError>> {
    let (mut parties, mut receivers, mut senders) = parties;

    // Simulate failure excluding faulty parties from all processes:
    for id in faulty_ids {
        parties.remove(id);
        receivers.remove(id);
        senders.remove(id);
    }

    let ballot_tasks = launch_parties(parties);
    let p2p_task = propagate_p2p(receivers, senders);
    let results = await_results(ballot_tasks).await;
    p2p_task.abort();
    results
}

#[tokio::test]
async fn test_ballot_happy_case() {
    let (parties, receivers, senders) = create_parties(BPConConfig::default());
    let ballot_tasks = launch_parties(parties);
    let p2p_task = propagate_p2p(receivers, senders);
    let results = await_results(ballot_tasks).await;
    p2p_task.abort();

    analyze_ballot(results);
}

#[tokio::test]
async fn test_ballot_faulty_party_common() {
    let parties = create_parties(BPConConfig::default());
    let elector = DefaultLeaderElector::new();
    let leader = elector.elect_leader(&parties.0[0]).unwrap();
    let faulty_ids: Vec<usize> = vec![3];
    for id in faulty_ids.iter() {
        assert_ne!(
            *id as u64, leader,
            "Should not fail the leader for the test to pass"
        );
    }
    let results = run_ballot_faulty_party(parties, faulty_ids).await;

    analyze_ballot(results);
}

#[tokio::test]
async fn test_ballot_faulty_party_leader() {
    let parties = create_parties(BPConConfig::default());
    let elector = DefaultLeaderElector::new();
    let leader = elector.elect_leader(&parties.0[0]).unwrap();
    let faulty_ids = vec![leader as usize];

    let results = run_ballot_faulty_party(parties, faulty_ids).await;

    assert!(
        results.into_iter().all(|res| res.is_err()),
        "All parties should have failed having a faulty leader in the consensus."
    );
}

#[tokio::test]
async fn test_ballot_malicious_party() {
    let (parties, mut receivers, senders) = create_parties(BPConConfig::default());

    let elector = DefaultLeaderElector::new();
    let leader = elector.elect_leader(&parties[0]).unwrap();
    const MALICIOUS_PARTY_ID: u64 = 1;

    assert_ne!(
        MALICIOUS_PARTY_ID, leader,
        "Should not make malicious the leader for the test to pass"
    );

    // We will be simulating malicious behaviour
    // sending 1b message (meaning, 5/6 times at incorrect stage) with the wrong data.
    let content = &Message1bContent {
        ballot: parties[0].ballot() + 1, // divergent ballot number
        last_ballot_voted: Some(parties[0].ballot() + 1), // early ballot number
        // shouldn't put malformed serialized value, because we won't be able to pack it
        last_value_voted: None,
    };
    let malicious_msg = content.pack(MALICIOUS_PARTY_ID).unwrap();

    let ballot_tasks = launch_parties(parties);
    let p2p_task = tokio::spawn(async move {
        // It is responsibility of the external to party code - p2p module
        // to rate-limit channels, because otherwise malicious
        // actors would be able to DDoS ballot, bloating all the channel with malicious ones.
        // For this test to pass, we will send malicious messages once in a while.
        let mut last_malicious_message_time = Instant::now();
        let malicious_message_interval = Duration::from_secs(3);
        loop {
            // Collect all messages first.
            let mut messages: Vec<_> = receivers
                .iter_mut()
                .enumerate()
                .filter_map(|(i, receiver)| {
                    // Skip receiving messages from the malicious party
                    // to substitute it with invalid one to be propagated.
                    (i != MALICIOUS_PARTY_ID as usize)
                        .then(|| receiver.try_recv().ok())
                        .flatten()
                })
                .collect();

            // Push the malicious message at intervals.
            if last_malicious_message_time.elapsed() >= malicious_message_interval {
                messages.push(malicious_msg.clone());
                last_malicious_message_time = Instant::now();
            }

            broadcast_messages(messages, &senders);

            // Delay to simulate network latency.
            sleep(Duration::from_millis(100)).await;
        }
    });

    let results = await_results(ballot_tasks).await;
    p2p_task.abort();

    analyze_ballot(results);
}

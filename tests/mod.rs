use bpcon::config::BPConConfig;
use bpcon::error::LaunchBallotError;
use bpcon::leader::DefaultLeaderElector;
use bpcon::message::{Message1bContent, MessagePacket};
use bpcon::party::Party;

use bpcon::test_mocks::{MockParty, MockValue, MockValueSelector};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use tokio::time;

// Here each party/receiver/sender shall correspond at equal indexes.
type PartiesWithChannels = (
    Vec<MockParty>,
    Vec<UnboundedReceiver<MessagePacket>>,
    Vec<UnboundedSender<MessagePacket>>,
);

// Create test parties with predefined generics, based on config.
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

// Begin ballot process for each party.
fn launch_parties(
    parties: Vec<Party<MockValue, MockValueSelector>>,
) -> Vec<JoinHandle<Result<Option<MockValue>, LaunchBallotError>>> {
    parties
        .into_iter()
        .map(|mut party| tokio::spawn(async move { party.launch_ballot().await }))
        .collect()
}

// Collect messages from receivers.
fn collect_messages(receivers: &mut [UnboundedReceiver<MessagePacket>]) -> Vec<MessagePacket> {
    receivers
        .iter_mut()
        .filter_map(|receiver| receiver.try_recv().ok())
        .collect()
}

// Broadcast collected messages to other parties, skipping the sender.
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

// Propagate messages peer-to-peer between parties using their channels.
fn propagate_messages_p2p(
    mut receivers: Vec<UnboundedReceiver<MessagePacket>>,
    senders: Vec<UnboundedSender<MessagePacket>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let messages = collect_messages(receivers.as_mut_slice());
            broadcast_messages(messages, &senders);

            // Delay to simulate network latency.
            time::sleep(time::Duration::from_millis(100)).await;
        }
    })
}

// Await completion of each party's process and aggregate ok results.
async fn extract_values_from_ballot_tasks(
    tasks: Vec<JoinHandle<Result<Option<MockValue>, LaunchBallotError>>>,
) -> Vec<MockValue> {
    let mut values = Vec::new();
    for (i, task) in tasks.into_iter().enumerate() {
        let result = task.await.unwrap();

        match result {
            Ok(Some(value)) => {
                values.push(value);
            }
            Ok(None) => {
                // This shall never happen for finished party.
                eprintln!("Party {}: No value was selected", i);
            }
            Err(err) => {
                eprintln!("Party {} encountered an error: {:?}", i, err);
            }
        }
    }
    values
}

// Use returned values from each party to analyze how ballot passed.
fn analyze_ballot_result(values: Vec<MockValue>) {
    match values.as_slice() {
        [] => {
            eprintln!("No consensus could be reached, as no values were selected by any party")
        }
        [first_value, ..] => {
            match values.iter().all(|v| v == first_value) {
                true => println!(
                    "All parties reached the same consensus value: {:?}",
                    first_value
                ),
                false => eprintln!("Not all parties agreed on the same value"),
            }
            println!("Consensus agreed on value {first_value:?}");
        }
    }
}

#[tokio::test]
async fn test_ballot_happy_case() {
    let (parties, receivers, senders) = create_parties(BPConConfig::default());

    let ballot_tasks = launch_parties(parties);

    let p2p_task = propagate_messages_p2p(receivers, senders);

    let values = extract_values_from_ballot_tasks(ballot_tasks).await;

    p2p_task.abort();

    analyze_ballot_result(values);
}

#[tokio::test]
async fn test_ballot_faulty_party() {
    let (mut parties, mut receivers, mut senders) = create_parties(BPConConfig::default());

    let leader = parties[0].ballot();

    assert_ne!(3, leader, "Should not fail the leader for the test to pass");

    // Simulate failure of `f` faulty participants:
    let parties = parties.drain(..3).collect();
    let receivers = receivers.drain(..3).collect();
    let senders = senders.drain(..3).collect();

    let ballot_tasks = launch_parties(parties);

    let p2p_task = propagate_messages_p2p(receivers, senders);

    let values = extract_values_from_ballot_tasks(ballot_tasks).await;

    p2p_task.abort();

    analyze_ballot_result(values);
}

#[tokio::test]
async fn test_ballot_malicious_party() {
    let (parties, mut receivers, senders) = create_parties(BPConConfig::default());

    let leader = parties[0].ballot();
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
        let mut last_malicious_message_time = time::Instant::now();
        let malicious_message_interval = time::Duration::from_millis(3000);
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

            // Push the malicious message at intervals
            // to mitigate bloating inner receiving channel.
            if last_malicious_message_time.elapsed() >= malicious_message_interval {
                messages.push(malicious_msg.clone());
                last_malicious_message_time = time::Instant::now();
            }

            broadcast_messages(messages, &senders);

            // Delay to simulate network latency.
            time::sleep(time::Duration::from_millis(100)).await;
        }
    });

    let values = extract_values_from_ballot_tasks(ballot_tasks).await;

    p2p_task.abort();

    analyze_ballot_result(values);
}

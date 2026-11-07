use tokio::io::{BufReader, duplex};
use tokio::time::timeout;

use super::*;

const KEY: Key = [7; blake3::KEY_LEN];
const OTHER: Key = [8; blake3::KEY_LEN];

/// The gateway checks the same tags in another repository and pins these
/// values too, which b3sum gives for this key and nonce.
#[test]
fn tags_are_keyed_blake3_of_nonce_then_claim() {
    assert_eq!(
        tag(&KEY, &[9; NONCE_LEN], "uid 2001").to_hex().as_str(),
        "0b346bc239a5621e087194715abb7698e6e72161480cbb0a962272c42ed80984",
    );
    assert_eq!(
        tag(&KEY, &[9; NONCE_LEN], Service::Gateway.accepted()).to_hex().as_str(),
        "9a86f6875380db50f0168a833724b6b4441386f2056d9cceedfaa79f5862e980",
    );
}

async fn exchange(
    opener: Key,
    dialled: Service,
    acceptor: Key,
    is: Service,
    claim: &str,
) -> (Result<()>, Result<String>) {
    let (accepting, mut opening) = duplex(1024);
    let mut accepting = BufReader::new(accepting);
    tokio::join!(async move { prove(&mut opening, &opener, dialled, claim).await }, async move {
        challenge(&mut accepting, &acceptor, is).await
    })
}

#[tokio::test]
async fn both_ends_holding_the_key_agree_on_the_claim() {
    let (proved, claim) = exchange(KEY, Service::Control, KEY, Service::Control, WAKE).await;
    proved.unwrap();
    assert_eq!(claim.unwrap(), WAKE);
}

#[tokio::test]
async fn an_opener_without_the_key_is_refused() {
    let (_, claim) = exchange(OTHER, Service::Control, KEY, Service::Control, WAKE).await;
    assert!(claim.is_err());
}

#[tokio::test]
async fn an_acceptor_without_the_key_is_refused() {
    let (proved, _) = exchange(KEY, Service::Gateway, OTHER, Service::Gateway, "uid 2001").await;
    assert!(proved.is_err());
}

#[tokio::test]
async fn an_acceptor_of_another_service_is_refused() {
    let (proved, _) = exchange(KEY, Service::Gateway, KEY, Service::Control, "uid 2001").await;
    assert!(proved.is_err());
}

#[tokio::test]
async fn an_opener_claiming_to_be_an_acceptor_is_refused() {
    let (_, claim) =
        exchange(KEY, Service::Control, KEY, Service::Control, "accepted control").await;
    assert!(claim.is_err());
}

#[tokio::test(start_paused = true)]
async fn a_silent_peer_times_out() {
    let (accepting, _opening) = duplex(1024);
    let mut accepting = BufReader::new(accepting);
    assert!(challenge(&mut accepting, &KEY, Service::Control).await.is_err());
}

/// Each ask reaches a daemon that answers the next of `answers`.
#[tokio::test]
async fn a_host_asks_again_while_agentd_is_busy() {
    let mut answers = [r#""busy""#, r#""busy""#, r#""injected""#].into_iter();
    let mut told = Vec::new();
    let ask = || {
        let answer = answers.next().expect("asked after it was answered");
        async move {
            let (host, mut daemon) = duplex(64);
            daemon.write_all(format!("{answer}\n").as_bytes()).await?;
            anyhow::Ok(BufReader::new(host).lines())
        }
    };
    let (first, _) = answered(ask, |line| told.push(line.to_owned())).await.unwrap();
    assert_eq!(first, r#""injected""#);
    assert_eq!(told, [r#""busy""#, r#""busy""#]);
    assert_eq!(answers.next(), None);
}

/// agentd closes the connection without a word.
#[tokio::test]
async fn a_host_reports_an_agentd_that_hangs_up_and_asks_no_more() {
    let mut asks = 0;
    let ask = || {
        asks += 1;
        assert_eq!(asks, 1, "asked again after a hang-up");
        async { anyhow::Ok(BufReader::new(duplex(64).0).lines()) }
    };
    assert!(answered(ask, |_| {}).await.is_err());
}

/// agentd's bound starts once it has read the request, a little after the host's.
#[tokio::test(start_paused = true)]
async fn a_host_takes_the_busy_answer_that_reaches_it_past_agentds_bound() {
    let (host, mut daemon) = duplex(64);
    let mut replies = BufReader::new(host).lines();
    let answering = async move {
        tokio::time::sleep(FIRST_REPLY + Duration::from_secs(1)).await;
        daemon.write_all(b"\"busy\"\n").await.unwrap();
        daemon
    };
    let (reply, _daemon) = tokio::join!(first_reply(&mut replies), answering);
    assert_eq!(reply.unwrap(), r#""busy""#);
}

#[tokio::test(start_paused = true)]
async fn a_host_gives_up_on_an_agentd_silent_past_its_bound() {
    let (host, _daemon) = duplex(64);
    let mut replies = BufReader::new(host).lines();
    assert!(timeout(FIRST_REPLY * 2, first_reply(&mut replies)).await.unwrap().is_err());
}

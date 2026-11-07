use tokio::io::{BufReader, duplex};

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

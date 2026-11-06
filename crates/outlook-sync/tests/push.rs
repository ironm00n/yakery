use std::convert::Infallible;

use bytes::Bytes;
use http_body_util::channel::{Channel, Sender};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use private::Dir;
use tokio::net::TcpListener;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio::time::{Instant, sleep};

use super::*;

const SESSION: &str = r#"{"accounts":{"h":{"name":"owen@example.test"}}}"#;
const HOUR: Duration = Duration::from_secs(3600);

#[test]
fn only_a_state_change_naming_the_owner_wakes_the_loop() {
    // the forms Stalwart 0.16.27 sends
    let owners = "event: state\ndata: {\"@type\":\"StateChange\",\"changed\":{\"h\":{\"CalendarEvent\":\"saq\"}}}\n\n";
    let others = "event: state\ndata: {\"@type\":\"StateChange\",\"changed\":{\"g\":{\"CalendarEvent\":\"sam\"}}}\n\n";
    assert!(changed(owners, "h"));
    assert!(!changed(others, "h"));
    assert!(!changed("event: ping\ndata: {\"interval\": 60}\n\n", "h"));
}

/// What the test's Stalwart does with a request.
#[derive(Clone, Copy)]
enum Reply {
    Unanswered,
    Whole(&'static str),
    /// a head, then the body the test feeds through the sender it is handed
    Fed,
}

struct Stalwart {
    policy: Policy,
    account: Account,
    asked: UnboundedReceiver<String>,
    fed: UnboundedReceiver<Sender<Bytes>>,
}

/// What a test's script sees: each path Stalwart is asked for, each push body it hands over to be
/// fed, and each wake of the sync.
struct Seen {
    asked: UnboundedReceiver<String>,
    fed: UnboundedReceiver<Sender<Bytes>>,
    woke: UnboundedReceiver<Wake>,
}

impl Stalwart {
    /// At a loopback port, answering the session with `session` and the push with `push`, and
    /// passing on each path it is asked for.
    async fn serving(session: Reply, push: Reply) -> Stalwart {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut policy: serde_json::Value =
            serde_json::from_str(include_str!("policy.json")).unwrap();
        policy["stalwart"] = listener.local_addr().unwrap().to_string().into();
        let policy: Policy = serde_json::from_value(policy).unwrap();
        let credentials = tempfile::tempdir().unwrap();
        std::fs::write(credentials.path().join("app-password"), "p").unwrap();
        let account = Account::read(&policy, &Dir::open(credentials.path(), &[]).unwrap()).unwrap();
        let ((ask, asked), (feed, fed)) = (unbounded_channel(), unbounded_channel());
        tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let (ask, feed) = (ask.clone(), feed.clone());
                let answer = service_fn(move |request: hyper::Request<_>| {
                    let path = request.uri().path().to_owned();
                    let reply = if path == "/jmap/session" { session } else { push };
                    let _ = ask.send(path);
                    let feed = feed.clone();
                    async move {
                        let (mut sender, body) = Channel::<Bytes>::new(1);
                        match reply {
                            Reply::Unanswered => std::future::pending().await,
                            Reply::Whole(whole) => sender.send_data(whole.into()).await.unwrap(),
                            Reply::Fed => feed.send(sender).unwrap(),
                        }
                        Ok::<_, Infallible>(hyper::Response::new(body))
                    }
                });
                tokio::spawn(http1::Builder::new().serve_connection(TokioIo::new(stream), answer));
            }
        });
        Stalwart { policy, account, asked, fed }
    }

    /// How the sync's stream fails while `script` runs, keeping what the script ends with, and how
    /// many seconds in; "hung" if it doesn't within the hour.
    async fn failure<F: Future>(self, script: impl FnOnce(Seen) -> F) -> (String, u64) {
        let Stalwart { policy, account, asked, fed } = self;
        let (wake, woke) = unbounded_channel();
        let start = Instant::now();
        let failed = tokio::select! {
            failed = stream(&policy, &account, &wake) => failed.unwrap_err().to_string(),
            _ = async {
                let _kept = script(Seen { asked, fed, woke }).await;
                std::future::pending::<()>().await
            } => unreachable!(),
            () = sleep(HOUR) => "hung".into(),
        };
        (failed, start.elapsed().as_secs())
    }
}

/// `step`, taking no time: a paused clock jumps to the next timer once every task waits, even on
/// bytes in flight over loopback.
async fn instantly<T>(step: impl Future<Output = T>) -> T {
    tokio::select! {
        biased;
        done = step => done,
        _ = async { loop { tokio::task::yield_now().await } } => unreachable!(),
    }
}

#[tokio::test(start_paused = true)]
async fn a_request_stalwart_never_answers_fails_after_three_pings() {
    let rows: [(Reply, Reply, &[&str]); 2] = [
        (Reply::Unanswered, Reply::Unanswered, &["/jmap/session"]),
        (Reply::Whole(SESSION), Reply::Unanswered, &["/jmap/session", "/jmap/eventsource/"]),
    ];
    let mut got = vec![];
    for (session, push, paths) in rows {
        let failure = Stalwart::serving(session, push).await.failure(|mut seen| async move {
            for path in paths {
                assert_eq!(instantly(seen.asked.recv()).await.as_deref(), Some(*path));
            }
        });
        got.push(failure.await);
    }
    let want = [("stalwart JMAP session", 180), ("stalwart JMAP push", 180)];
    assert_eq!(got, want.map(|(step, secs)| (step.to_owned(), secs)));
}

#[tokio::test(start_paused = true)]
async fn a_stream_silent_for_three_pings_is_dead() {
    let edit = "event: state\ndata: {\"@type\":\"StateChange\",\"changed\":{\"h\":{\"CalendarEvent\":\"sar\"}}}\n\n";
    let stalwart = Stalwart::serving(Reply::Whole(SESSION), Reply::Fed).await;
    let failure = stalwart.failure(|mut seen| async move {
        let mut events = instantly(seen.fed.recv()).await.unwrap();
        assert_eq!(instantly(seen.woke.recv()).await, Some(Wake::Edited));
        sleep(Duration::from_secs(179)).await;
        events.send_data(edit.into()).await.unwrap();
        assert_eq!(instantly(seen.woke.recv()).await, Some(Wake::Edited));
        events
    });
    assert_eq!(failure.await, ("the JMAP push stream".into(), 359));
}

use std::{
    collections::HashMap,
    convert::Infallible,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full};
use hyper::{Request as HttpRequest, Response, body::Bytes, service::service_fn};
use hyper_util::rt::TokioIo;
use private::{Dir, Secret};
use reqwest::Url;
use serde_json::{Value, json};
use tokio::{
    net::{TcpListener, UnixListener, UnixStream, unix::OwnedReadHalf, unix::OwnedWriteHalf},
    sync::mpsc,
    time::timeout,
};
use tokio_tungstenite::tungstenite::Message as Frame;

use crate::{
    api::{Line, Reply, Request},
    daemon::{Bounds, Daemon, MAX_MARKDOWN},
    slack::Slack,
    *,
};

const HOME: &str = "C0EXAMPLE1";
const OTHER: &str = "C0EXAMPLE2";
const BOT: &str = "U0BOT";
const PERSON: &str = "U0PERSON";
const SOON: Duration = Duration::from_secs(5);
const QUICK: Bounds = Bounds {
    down: Duration::from_secs(1),
    recheck: Duration::from_millis(50),
    reconnect: Duration::from_millis(20),
    ping: Duration::from_millis(200),
};

fn ts(s: &str) -> Ts {
    s.parse().unwrap()
}

fn with(mut base: Value, more: Value) -> Value {
    base.as_object_mut().unwrap().extend(more.as_object().unwrap().clone());
    base
}

fn said(text: &str, ts: &str, more: Value) -> Value {
    with(json!({ "type": "message", "user": PERSON, "text": text, "ts": ts }), more)
}

#[test]
fn only_plain_messages_from_people_in_listed_channels_count() {
    let listed = channels(&format!("{HOME}\n\n {OTHER} \n")).unwrap();
    let bot: User = BOT.parse().unwrap();
    let admitted = |more: Value| {
        let raw =
            serde_json::from_value(said("hi", "1.000001", with(json!({ "channel": HOME }), more)));
        admit(&raw.unwrap(), &listed, &bot).map(|m| m.thread_ts.map(|t| t.to_string()))
    };
    assert_eq!(admitted(json!({})), Some(None));
    assert_eq!(admitted(json!({ "channel": OTHER })), Some(None));
    assert_eq!(admitted(json!({ "thread_ts": "0.000009" })), Some(Some("0.000009".into())));
    assert_eq!(admitted(json!({ "thread_ts": "1.000001" })), Some(None));
    let broadcast = json!({ "subtype": "thread_broadcast", "thread_ts": "0.000009" });
    assert_eq!(admitted(broadcast), Some(Some("0.000009".into())));
    for dropped in [
        json!({ "channel": "C0UNLISTED" }),
        json!({ "user": BOT }),
        json!({ "bot_id": "B0APP" }),
        json!({ "subtype": "bot_message", "user": null, "bot_id": "B0APP" }),
        json!({ "subtype": "message_changed" }),
        json!({ "subtype": "message_deleted" }),
        json!({ "subtype": "channel_join" }),
        json!({ "type": "reaction_added" }),
        json!({ "user": null }),
    ] {
        assert_eq!(admitted(dropped.clone()), None, "{dropped}");
    }
    assert!(channels("C0OK\nnot-a-channel\n").is_err());
    assert_eq!(listed.first().map(ToString::to_string).as_deref(), Some(HOME));
}

#[test]
fn timestamps_parse_strictly_and_order_numerically() {
    assert!(ts("9.000001") < ts("10.000000"));
    assert_eq!(ts("1700000000.000100").to_string(), "1700000000.000100");
    for refused in ["1", "1.1", "1.0000001", ".000001", "a.000001", "1.00000a"] {
        assert!(refused.parse::<Ts>().is_err(), "{refused}");
    }
}

#[test]
fn the_inbox_replays_keeps_one_of_each_and_hands_out_the_oldest_untaken() {
    let m = |n: u32| Message {
        channel: HOME.parse().unwrap(),
        ts: ts(&format!("5.{n:06}")),
        thread_ts: None,
        user: PERSON.parse().unwrap(),
        text: format!("m{n}"),
    };
    let (a, b) = (m(2), m(1));
    let entries = [
        Entry::Message(a.clone()),
        Entry::Message(a.clone()),
        Entry::Message(b.clone()),
        Entry::Taken(a.key()),
    ];
    let file: String = entries.iter().map(|e| serde_json::to_string(e).unwrap() + "\n").collect();
    let inbox = Inbox::replay(&file).unwrap();
    assert_eq!(inbox.next(), Some(&b));
    assert_eq!(inbox.newest(&a.channel), Some(a.ts));
}

#[test]
fn long_bodies_split_at_newlines_within_the_limit() {
    assert_eq!(chunks("abcdef", 3), ["abc", "def"]);
    assert_eq!(chunks("ab\ncdef", 4), ["ab\n", "cdef"]);
    assert_eq!(chunks("äöü", 2), ["äö", "ü"]);
    assert!(chunks("", 3).is_empty());
    assert_eq!(unescape("a &lt;b&gt; &amp;lt;"), "a <b> &lt;");
}

/// Slack as the daemon sees it: a web API that records posts and serves history, and a Socket Mode socket that sends
/// what a test queues and passes on what the daemon sends back.
#[derive(Default)]
struct Fake {
    ws: Mutex<Option<SocketAddr>>,
    posts: Mutex<Vec<HashMap<String, String>>>,
    history: Mutex<Vec<Value>>,
    replies: Mutex<HashMap<String, Vec<Value>>>,
    connections: Mutex<usize>,
    /// apps.connections.open answers this error instead of a URL.
    refused: Mutex<Option<&'static str>>,
}

impl Fake {
    fn answer(&self, method: &str, token: &str, form: HashMap<String, String>) -> Value {
        let app = method == "apps.connections.open";
        let refused = *self.refused.lock().unwrap();
        assert_eq!(token, if app { "Bearer xapp-test" } else { "Bearer xoxb-test" }, "{method}");
        let oldest = form.get("oldest").map(|o| ts(o));
        let after = |list: &[Value]| -> Vec<Value> {
            let newer = |m: &&Value| oldest.is_none_or(|o| ts(m["ts"].as_str().unwrap()) > o);
            list.iter().filter(newer).cloned().collect()
        };
        match method {
            "auth.test" => json!({ "ok": true, "user_id": BOT }),
            "apps.connections.open" if refused.is_some() => {
                json!({ "ok": false, "error": refused })
            }
            "apps.connections.open" => {
                json!({ "ok": true, "url": format!("ws://{}/", self.ws.lock().unwrap().unwrap()) })
            }
            "chat.postMessage" => {
                let mut posts = self.posts.lock().unwrap();
                posts.push(form);
                json!({ "ok": true, "ts": format!("1800000000.{:06}", posts.len()) })
            }
            "conversations.history" => {
                let mut top = after(&self.history.lock().unwrap());
                top.reverse();
                json!({ "ok": true, "messages": top, "has_more": false })
            }
            "conversations.replies" => {
                let thread =
                    self.replies.lock().unwrap().get(&form["ts"]).cloned().unwrap_or_default();
                json!({ "ok": true, "messages": after(&thread), "has_more": false })
            }
            _ => json!({ "ok": false, "error": "unknown_method" }),
        }
    }
}

async fn web(fake: Arc<Fake>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (tcp, _) = listener.accept().await.unwrap();
            let fake = fake.clone();
            let service = service_fn(move |req: HttpRequest<hyper::body::Incoming>| {
                let fake = fake.clone();
                async move {
                    let method = req.uri().path().trim_start_matches("/api/").to_owned();
                    let token = req.headers()["authorization"].to_str().unwrap().to_owned();
                    let body = req.into_body().collect().await.unwrap().to_bytes();
                    let query =
                        Url::parse(&format!("http://x/?{}", String::from_utf8_lossy(&body)));
                    let form = query.unwrap().query_pairs().into_owned().collect();
                    let answer = fake.answer(&method, &token, form).to_string();
                    Ok::<_, Infallible>(Response::new(Full::new(Bytes::from(answer))))
                }
            });
            let http = hyper::server::conn::http1::Builder::new();
            tokio::spawn(http.serve_connection(TokioIo::new(tcp), service));
        }
    });
    addr
}

/// One connection at a time, as the daemon holds; `{"close": true}` ends the current one, and `{"mute": true}` keeps it
/// open, never read again, so it answers no ping, and takes the next.
async fn socket_mode(
    fake: Arc<Fake>,
    mut send: mpsc::UnboundedReceiver<Value>,
    acks: mpsc::UnboundedSender<String>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    *fake.ws.lock().unwrap() = Some(listener.local_addr().unwrap());
    tokio::spawn(async move {
        let mut muted = Vec::new();
        loop {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            *fake.connections.lock().unwrap() += 1;
            ws.send(Frame::text(json!({ "type": "hello" }).to_string())).await.unwrap();
            let mute = loop {
                tokio::select! {
                    out = send.recv() => match out {
                        Some(v) if v["close"] == true => break false,
                        Some(v) if v["mute"] == true => break true,
                        Some(v) => ws.send(Frame::text(v.to_string())).await.unwrap(),
                        None => return,
                    },
                    back = ws.next() => match back {
                        Some(Ok(Frame::Text(t))) => acks.send(t.to_string()).unwrap(),
                        Some(Ok(_)) => {}
                        _ => break false,
                    },
                }
            };
            if mute {
                muted.push(ws);
            }
        }
    });
}

async fn slack(fake: Arc<Fake>) -> Slack {
    let api: Url = format!("http://{}/api/", web(fake).await).parse().unwrap();
    Slack::new(api, Secret::new("xoxb-test"), Secret::new("xapp-test")).unwrap()
}

type Client = Line<OwnedReadHalf, OwnedWriteHalf>;

struct Harness {
    fake: Arc<Fake>,
    send: mpsc::UnboundedSender<Value>,
    acks: mpsc::UnboundedReceiver<String>,
    socket: PathBuf,
    list: PathBuf,
    state: tempfile::TempDir,
}

impl Harness {
    async fn new() -> Harness {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let fake = Arc::new(Fake::default());
        let (send, sent) = mpsc::unbounded_channel();
        let (acked, acks) = mpsc::unbounded_channel();
        socket_mode(fake.clone(), sent, acked).await;
        let state = tempfile::tempdir().unwrap();
        let list = state.path().join("channels");
        std::fs::write(&list, format!("{HOME}\n{OTHER}\n")).unwrap();
        let socket = state.path().join("api.sock");
        let h = Harness { fake, send, acks, socket, list, state };
        h.start().await;
        h
    }

    /// A daemon on this harness's state, as after a restart.
    async fn start(&self) {
        let slack = slack(self.fake.clone()).await;
        let inbox = Dir::open(&self.state.path().join("inbox"), &[]).unwrap();
        let daemon = Daemon::start(slack, inbox, self.list.clone(), QUICK).await.unwrap();
        let _ = std::fs::remove_file(&self.socket);
        tokio::spawn(daemon.run(UnixListener::bind(&self.socket).unwrap()));
    }

    fn envelope(&self, id: &str, kind: &str, payload: Value) {
        self.send.send(json!({ "envelope_id": id, "type": kind, "payload": payload })).unwrap();
    }

    fn message(&self, id: &str, event: Value) {
        let e =
            with(json!({ "type": "message", "channel": HOME, "user": PERSON, "text": id }), event);
        self.envelope(id, "events_api", json!({ "event": e }));
    }

    async fn acked(&mut self) -> String {
        let ack = timeout(SOON, self.acks.recv()).await.unwrap().unwrap();
        serde_json::from_str::<Value>(&ack).unwrap()["envelope_id"].as_str().unwrap().to_owned()
    }

    fn inbox(&self) -> String {
        std::fs::read_to_string(self.state.path().join("inbox/inbox")).unwrap_or_default()
    }

    async fn client(&self, request: Request) -> Client {
        let (read, write) = UnixStream::connect(&self.socket).await.unwrap().into_split();
        let mut line = Line::new(read, write);
        line.send(&request).await.unwrap();
        line
    }

    async fn ask(&self, request: Request) -> Reply {
        timeout(SOON, self.client(request).await.next()).await.unwrap().unwrap().unwrap()
    }

    async fn connected(&self, n: usize) {
        let start = Instant::now();
        while *self.fake.connections.lock().unwrap() < n {
            assert!(start.elapsed() < SOON, "no connection {n}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// One whole handover, as `agent-slack wait` does it.
    async fn take(&self) -> Message {
        let mut wait = self.client(Request::Wait).await;
        let m = handed(&mut wait).await;
        wait.send(&Request::Taken(m.key())).await.unwrap();
        assert!(matches!(wait.next().await.unwrap(), Some(Reply::Taken)));
        m
    }
}

async fn handed(line: &mut Client) -> Message {
    match timeout(SOON, line.next()).await.unwrap().unwrap() {
        Some(Reply::Message(m)) => m,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn events_are_on_disk_before_their_ack_and_only_checked_ones_count() {
    let mut h = Harness::new().await;
    h.connected(1).await;
    h.message("first", json!({ "ts": "1900000001.000001" }));
    assert_eq!(h.acked().await, "first");
    assert!(h.inbox().contains(r#""text":"first""#), "{}", h.inbox());
    h.message("again", json!({ "ts": "1900000001.000001" }));
    h.message("bot", json!({ "ts": "1900000002.000001", "bot_id": "B0APP" }));
    h.message("own", json!({ "ts": "1900000003.000001", "user": BOT }));
    h.message("edit", json!({ "ts": "1900000004.000001", "subtype": "message_changed" }));
    h.message("away", json!({ "ts": "1900000005.000001", "channel": "C0UNLISTED" }));
    h.envelope("click", "interactive", json!({ "type": "block_actions", "actions": [] }));
    h.envelope("odd", "events_api", json!({ "event": { "type": "message", "ts": 7 } }));
    for id in ["again", "bot", "own", "edit", "away", "click", "odd"] {
        assert_eq!(h.acked().await, id);
    }
    assert_eq!(h.inbox().matches(r#"{"message""#).count(), 1, "{}", h.inbox());
    let m = h.take().await;
    assert_eq!((m.text.as_str(), m.ts), ("first", ts("1900000001.000001")));
}

#[tokio::test]
async fn a_message_is_taken_only_once_a_wait_confirms_it_and_one_wait_runs_at_a_time() {
    let mut h = Harness::new().await;
    h.connected(1).await;
    h.message("hello", json!({ "ts": "1900000001.000001", "thread_ts": "1900000000.000001" }));
    h.acked().await;
    let mut dropped = h.client(Request::Wait).await;
    let m = handed(&mut dropped).await;
    assert_eq!(m.thread_ts, Some(ts("1900000000.000001")));
    match h.ask(Request::Wait).await {
        Reply::Refused(why) => assert!(why.ends_with("is waiting already"), "{why}"),
        other => panic!("{other:?}"),
    }
    drop(dropped);
    assert_eq!(h.take().await, m);
    let mut next = h.client(Request::Wait).await;
    assert!(timeout(Duration::from_millis(300), next.next::<Reply>()).await.is_err());
    h.message("later", json!({ "ts": "1900000002.000001" }));
    assert_eq!(handed(&mut next).await.text, "later");
    drop(next);
    h.start().await;
    assert_eq!(h.take().await.text, "later");
}

#[tokio::test]
async fn each_connect_reads_history_since_the_newest_message_held() {
    let mut h = Harness::new().await;
    h.fake.history.lock().unwrap().push(said("before the daemon", "1000000000.000001", json!({})));
    h.connected(1).await;
    h.message("held", json!({ "ts": "1900000001.000001" }));
    h.acked().await;
    let parent = said(
        "parent",
        "1900000002.000001",
        json!({ "reply_count": 1, "latest_reply": "1900000004.000001" }),
    );
    let reply = said("reply", "1900000004.000001", json!({ "thread_ts": "1900000002.000001" }));
    let bot = said("own post", "1900000003.000001", json!({ "user": BOT }));
    *h.fake.history.lock().unwrap() = vec![
        said("held", "1900000001.000001", json!({})),
        parent,
        bot,
        said("top", "1900000005.000001", json!({})),
    ];
    h.fake.replies.lock().unwrap().insert("1900000002.000001".into(), vec![reply]);
    h.send.send(json!({ "close": true })).unwrap();
    h.connected(2).await;
    let mut texts = Vec::new();
    for _ in 0..4 {
        texts.push(h.take().await.text);
    }
    assert_eq!(texts, ["held", "parent", "reply", "top"]);
    assert!(!h.inbox().contains("before the daemon") && !h.inbox().contains("own post"));
}

#[tokio::test]
async fn send_posts_markdown_split_into_one_thread_and_read_prints_it() {
    let h = Harness::new().await;
    let body = "a".repeat(MAX_MARKDOWN - 1) + "\n" + &"b".repeat(MAX_MARKDOWN + 5);
    let start = Instant::now();
    let Reply::Sent((channel, first)) = h.ask(Request::Send { to: None, thread: None, body }).await
    else {
        panic!()
    };
    assert!(start.elapsed() >= Duration::from_secs(2));
    assert_eq!((channel.to_string(), first), (HOME.to_owned(), ts("1800000000.000001")));
    let posts = h.fake.posts.lock().unwrap().clone();
    let shape: Vec<_> =
        posts.iter().map(|p| (p["markdown_text"].len(), p.get("thread_ts").cloned())).collect();
    let thread = Some(first.to_string());
    assert_eq!(shape, [(MAX_MARKDOWN, None), (MAX_MARKDOWN, thread.clone()), (5, thread)]);
    let to_other =
        Request::Send { to: Some(OTHER.parse().unwrap()), thread: Some(first), body: "x".into() };
    assert!(matches!(h.ask(to_other).await, Reply::Sent(_)));
    assert_eq!(h.fake.posts.lock().unwrap()[3]["channel"], OTHER);
    let unlisted =
        Request::Send { to: Some("C0UNLISTED".parse().unwrap()), thread: None, body: "x".into() };
    let Reply::Refused(why) = h.ask(unlisted).await else { panic!() };
    assert!(why.starts_with("C0UNLISTED is not on"), "{why}");
    let thread = vec![
        said("start", "1800000000.000001", json!({ "user": BOT })),
        said("answer", "1800000009.000001", json!({})),
    ];
    h.fake.replies.lock().unwrap().insert(first.to_string(), thread);
    let Reply::Thread(read) = h.ask(Request::Read { to: None, thread: first }).await else {
        panic!()
    };
    let read: Vec<_> =
        read.iter().map(|s| (s.text.as_str(), s.user.as_ref().map(ToString::to_string))).collect();
    assert_eq!(read, [("start", Some(BOT.into())), ("answer", Some(PERSON.into()))]);
}

fn refused(reply: Reply) -> String {
    match reply {
        Reply::Refused(why) => why,
        other => panic!("{other:?}"),
    }
}

fn post() -> Request {
    Request::Send { to: None, thread: None, body: "x".into() }
}

#[tokio::test]
async fn with_no_live_connection_past_the_bound_every_request_is_refused_until_one_is_back() {
    let mut h = Harness::new().await;
    h.connected(1).await;
    let mut waiting = h.client(Request::Wait).await;
    *h.fake.refused.lock().unwrap() = Some("internal_error");
    h.send.send(json!({ "close": true })).unwrap();
    assert!(matches!(h.ask(post()).await, Reply::Sent(_)), "below the bound");
    let start = Instant::now();
    let why = match timeout(SOON, waiting.next()).await.unwrap().unwrap() {
        Some(Reply::Refused(why)) => why,
        other => panic!("{other:?}"),
    };
    assert!(start.elapsed() < 2 * QUICK.down, "{:?}", start.elapsed());
    let down = "agent-slackd has had no live Socket Mode connection for ";
    assert!(why.starts_with(down), "{why}");
    assert!(why.ends_with("s: apps.connections.open: internal_error"), "{why}");
    assert!(refused(h.ask(post()).await).starts_with(down));
    let thread = ts("1800000000.000001");
    assert!(refused(h.ask(Request::Read { to: None, thread }).await).starts_with(down));
    *h.fake.refused.lock().unwrap() = None;
    h.message("back", json!({ "ts": "1900000001.000001" }));
    assert_eq!(h.acked().await, "back");
    assert_eq!(h.take().await.text, "back");
    assert!(matches!(h.ask(post()).await, Reply::Sent(_)));
}

#[tokio::test]
async fn a_channel_list_that_does_not_read_refuses_wait_and_send_at_once_and_loses_no_message() {
    let mut h = Harness::new().await;
    h.connected(1).await;
    std::fs::write(&h.list, "not-a-channel\n").unwrap();
    let why = refused(h.ask(Request::Wait).await);
    assert!(why.starts_with(&format!("the channel list {}: ", h.list.display())), "{why}");
    assert_eq!(refused(h.ask(post()).await), why);
    std::fs::write(&h.list, "").unwrap();
    assert!(refused(h.ask(post()).await).ends_with(": it lists no channel"));
    h.fake.history.lock().unwrap().push(said("kept", "1900000001.000001", json!({})));
    h.message("kept", json!({ "ts": "1900000001.000001" }));
    let acked = timeout(Duration::from_millis(300), h.acks.recv()).await;
    assert!(acked.is_err(), "unacked, so Slack still holds it: {acked:?}");
    std::fs::write(&h.list, format!("{HOME}\n")).unwrap();
    h.connected(2).await;
    assert_eq!(h.take().await.text, "kept", "caught up from history");
}

#[tokio::test]
async fn a_connection_that_answers_no_ping_is_dropped_and_another_opened() {
    let h = Harness::new().await;
    h.connected(1).await;
    h.send.send(json!({ "mute": true })).unwrap();
    let start = Instant::now();
    h.connected(2).await;
    assert!(start.elapsed() >= 2 * QUICK.ping, "{:?}", start.elapsed());
}

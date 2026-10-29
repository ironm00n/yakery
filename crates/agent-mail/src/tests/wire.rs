use std::{
    cell::RefCell,
    collections::VecDeque,
    convert::Infallible,
    pin::Pin,
    process::ExitCode,
    rc::Rc,
    task::{Context as Waker, Poll},
};

use bytes::Bytes;
use http_body::Frame;
use private::{Secret, now};
use reqwest::{Method, Request, Response};
use submission_signature::{HEADER, Signer, Stamp};
use tokio::time::Instant;

use super::*;
use crate::{
    RECHECK,
    address::SessionId,
    auth::{Auth, Http},
    cmd::wait,
    listeners::Heartbeat,
    mail::{Conn, Current, Email, Keyword, Mail},
    out::{Leftover, Outbox},
    policy::Recipients,
    store::{Access, PasswordFile, Store, Tokens},
    web::{Body, Origin, Web},
};

type Reply = http::Response<reqwest::Body>;

type Answer = Box<dyn FnMut(&Request) -> Reply>;

thread_local! {
    static WIRE: RefCell<Option<Answer>> = const { RefCell::new(None) };
}

/// The answer a test gives in place of the network; with none given, a request fails rather than leave the host.
pub fn wire(request: &Request) -> Option<Response> {
    let answered = WIRE.with_borrow_mut(|wire| wire.as_mut().map(|answer| answer(request)));
    Some(answered.unwrap_or_else(|| reply(599, "no wire in this test")).into())
}

pub struct Wired;

fn wired(answer: impl FnMut(&Request) -> Reply + 'static) -> Wired {
    let _ = rustls::crypto::ring::default_provider().install_default();
    WIRE.set(Some(Box::new(answer)));
    Wired
}

impl Drop for Wired {
    fn drop(&mut self) {
        WIRE.set(None);
    }
}

fn reply(status: u16, body: impl Into<String>) -> Reply {
    let kind = if status == 400 || status == 429 {
        "application/problem+json"
    } else {
        "application/json"
    };
    let reply = http::Response::builder().status(status).header("content-type", kind);
    reply.body(body.into().into()).unwrap()
}

fn moved(status: u16, to: &str) -> Reply {
    http::Response::builder().status(status).header("location", to).body("".into()).unwrap()
}

const PING: &str = "event: ping\ndata: {\"interval\": 30}\n\n";

/// A push stream that sends `events`, then stays open, pinging every 30 s as Stalwart does unless it has no timer.
struct Open(VecDeque<&'static str>, Option<Pin<Box<tokio::time::Sleep>>>);

impl http_body::Body for Open {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        waker: &mut Waker<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        let ping = match (self.0.pop_front(), self.1.as_mut()) {
            (Some(event), _) => event,
            (None, None) => return Poll::Pending,
            (None, Some(timer)) => {
                std::task::ready!(timer.as_mut().poll(waker));
                timer.as_mut().reset(Instant::now() + Duration::from_secs(30));
                PING
            }
        };
        Poll::Ready(Some(Ok(Frame::data(Bytes::from(ping)))))
    }
}

fn open(events: &[&'static str], pings: bool) -> Reply {
    let timer = pings.then(|| Box::pin(tokio::time::sleep(Duration::from_secs(30))));
    let body = reqwest::Body::wrap(Open(events.iter().copied().collect(), timer));
    http::Response::builder().header("content-type", "text/event-stream").body(body).unwrap()
}

fn stream(events: &[&'static str]) -> Reply {
    open(events, true)
}

struct Trickle(usize, Duration, Pin<Box<tokio::time::Sleep>>);

impl http_body::Body for Trickle {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        waker: &mut Waker<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        if self.0 == 0 {
            return Poll::Ready(None);
        }
        std::task::ready!(self.2.as_mut().poll(waker));
        let next = Instant::now() + self.1;
        self.2.as_mut().reset(next);
        self.0 -= 1;
        Poll::Ready(Some(Ok(Frame::data(Bytes::from(vec![0; 1024])))))
    }
}

fn trickle(n: usize, gap: u64) -> Reply {
    let gap = Duration::from_secs(gap);
    http::Response::new(reqwest::Body::wrap(Trickle(n, gap, Box::pin(tokio::time::sleep(gap)))))
}

/// How a request looked: its method and path, the JMAP method it calls, its body otherwise, and its token.
fn seen(r: &Request) -> String {
    let body = r.body().and_then(|b| b.as_bytes()).map(String::from_utf8_lossy).unwrap_or_default();
    let call: Value = serde_json::from_str(&body).unwrap_or_default();
    let what = call["methodCalls"][0][0].as_str().map_or(body.to_string(), String::from);
    let token =
        r.headers().get("authorization").map(|t| format!(" @{}", &t.to_str().unwrap()[7..]));
    format!("{} {} {what}{}", r.method(), r.url().path(), token.unwrap_or_default())
        .replace("  ", " ")
}

const LIMIT: &str =
    r#"{"type":"urn:ietf:params:jmap:error:limit","limit":"maxConcurrentRequests"}"#;

pub enum Canned {
    Answer(Value),
    Http(u16),
    /// Answered from the inbox.
    Pass,
    /// Written, but answered 504.
    Lost,
    /// Answered with this method error, after being written if the flag says so.
    Error(&'static str, bool),
    /// Another client sets this keyword alone on each updated mail first.
    Rival(&'static str),
}

/// Stalwart over the wire, with one account's inbox, and ZITADEL's token endpoint granting `t2`, `t3`, ...
#[derive(Default)]
pub struct Stalwart {
    pub emails: Vec<Value>,
    /// Ids a thread merge left, and the email each names now.
    pub stale: HashMap<String, String>,
    state: u32,
    /// Tokens answered 401.
    refused: HashSet<String>,
    /// The token in use expires once the push stream opens.
    expire_on_stream: bool,
    streams: VecDeque<Reply>,
    downloads: VecDeque<Reply>,
    /// Answers in place of the inbox's for a method, in turn.
    pub canned: Vec<(&'static str, Canned)>,
    granted: u32,
    refresh_refused: bool,
    pub session: Value,
    pub log: Vec<String>,
    /// Ids an Email/get asked attachments of, which Stalwart parses the message for.
    parsed: BTreeSet<String>,
    opened: Vec<Duration>,
    start: Option<Instant>,
    /// Seconds after the start during which every request to the mail host is answered 503.
    down: std::ops::Range<u64>,
}

pub type Shared = Rc<RefCell<Stalwart>>;

impl Stalwart {
    pub fn new(emails: Vec<Value>) -> Stalwart {
        let at = |path: &str| format!("https://mail.example.org/jmap/{path}");
        let limits = json!({ "maxObjectsInGet": 500, "maxObjectsInSet": 500 });
        let session = json!({
            "apiUrl": at(""), "username": "agent@example.org",
            "uploadUrl": at("upload/{accountId}/"),
            "downloadUrl": at("download/{accountId}/{blobId}/{name}?accept={type}"),
            "eventSourceUrl": at("eventsource/?types={types}&closeafter={closeafter}&ping={ping}"),
            "primaryAccounts": { "urn:ietf:params:jmap:mail": "a" },
            "capabilities": { "urn:ietf:params:jmap:core": limits },
        });
        Stalwart { emails, session, start: Some(Instant::now()), ..Stalwart::default() }
    }

    fn wire(self) -> (Wired, Shared) {
        let shared = Rc::new(RefCell::new(self));
        let server = shared.clone();
        (wired(move |r| server.borrow_mut().answer(r)), shared)
    }

    fn answer(&mut self, r: &Request) -> Reply {
        self.log.push(seen(r));
        let down = self.down.contains(&self.start.unwrap().elapsed().as_secs());
        if down && r.url().host_str() == Some("mail.example.org") {
            return reply(503, "");
        }
        let token = r.headers().get("authorization").map(|t| t.to_str().unwrap()[7..].to_owned());
        if token.as_ref().is_some_and(|t| self.refused.contains(t)) {
            return reply(401, "");
        }
        match (r.url().host_str().unwrap(), r.url().path()) {
            ("auth.example.org", "/oauth/v2/token") if self.refresh_refused => {
                reply(400, r#"{"error":"invalid_grant"}"#)
            }
            ("auth.example.org", "/oauth/v2/token") => {
                self.granted += 1;
                let (access, refresh) =
                    (format!("t{}", self.granted + 1), format!("r{}", self.granted + 1));
                reply(
                    200,
                    json!({ "access_token": access, "refresh_token": refresh, "expires_in": 3600 })
                        .to_string(),
                )
            }
            (_, "/.well-known/jmap") => reply(200, self.session.to_string()),
            (_, "/jmap/eventsource/") => {
                self.opened.push(self.start.unwrap().elapsed());
                if std::mem::take(&mut self.expire_on_stream) {
                    self.refused.extend(token);
                    return reply(401, "");
                }
                self.streams.pop_front().unwrap_or_else(|| stream(&[]))
            }
            (_, "/jmap/") => {
                let body: Value =
                    serde_json::from_slice(r.body().unwrap().as_bytes().unwrap()).unwrap();
                let (mut replies, mut lost) = (vec![], false);
                for call in body["methodCalls"].as_array().unwrap() {
                    let (method, args, id) = (call[0].as_str().unwrap(), &call[1], &call[2]);
                    let canned = self.canned.iter().position(|c| c.0 == method);
                    let answered = match canned.map(|i| self.canned.remove(i).1) {
                        Some(Canned::Http(status)) => {
                            return reply(status, if status == 429 { LIMIT } else { "{}" });
                        }
                        Some(Canned::Answer(args)) => Ok(args),
                        Some(Canned::Lost) => {
                            lost = true;
                            self.jmap(method, args)
                        }
                        Some(Canned::Error(kind, written)) => {
                            if written {
                                self.jmap(method, args).ok();
                            }
                            Err(kind)
                        }
                        Some(Canned::Rival(keyword)) => {
                            let ids = args["update"].as_object().into_iter().flatten();
                            let patch = json!({ format!("keywords/{keyword}"): true });
                            let rival = ids.map(|(id, _)| (id.clone(), patch.clone()));
                            let rival = json!({ "update": rival.collect::<Map<_, _>>() });
                            self.jmap(method, &rival).and(self.jmap(method, args))
                        }
                        Some(Canned::Pass) | None => self.jmap(method, args),
                    };
                    replies.push(match answered {
                        Ok(args) => json!([method, args, id]),
                        Err(kind) => json!(["error", { "type": kind }, id]),
                    });
                }
                if lost {
                    return reply(504, "{}");
                }
                reply(200, json!({ "methodResponses": replies }).to_string())
            }
            (_, path) if path.starts_with("/jmap/download/") => {
                self.downloads.pop_front().unwrap_or_else(|| reply(404, ""))
            }
            _ => reply(404, ""),
        }
    }

    /// The email an id names now: Stalwart reads and writes a stale id as the email it names, and answers under it.
    fn now(&self, id: &str) -> String {
        self.stale.get(id).cloned().unwrap_or_else(|| id.into())
    }

    fn jmap(&mut self, method: &str, args: &Value) -> Result<Value, &'static str> {
        let state = self.state.to_string();
        Ok(match method {
            "Mailbox/get" => {
                let list = ["inbox", "sent", "drafts"].map(|r| json!({ "id": r, "role": r }));
                json!({ "list": list })
            }
            "Identity/get" => json!({ "list": [{ "id": "i1", "email": "agent@example.org" }] }),
            "Email/get" => {
                let under = |id: &Value| {
                    let now = self.now(id.as_str()?);
                    let mut m = self.emails.iter().find(|m| m["id"] == now)?.clone();
                    m["id"] = id.clone();
                    Some(m)
                };
                let list: Vec<Value> = match args["ids"].as_array() {
                    Some(ids) => ids.iter().filter_map(under).collect(),
                    None => self.emails.clone(),
                };
                let asked = args["properties"].as_array().cloned().unwrap_or_default();
                if asked.contains(&json!("attachments")) {
                    self.parsed.extend(list.iter().map(|m| m["id"].as_str().unwrap().to_owned()));
                }
                json!({ "state": state, "list": list })
            }
            "Email/query" => {
                let ids = self.emails.iter().map(|m| &m["id"]);
                let now =
                    |i: &Value| self.stale.get(i.as_str().unwrap()).map_or(i.clone(), |c| json!(c));
                let ids: Vec<&Value> = match args["filter"]["id"].as_array() {
                    Some(asked) => ids.filter(|i| asked.iter().any(|a| now(a) == **i)).collect(),
                    None => ids.collect(),
                };
                json!({ "ids": ids })
            }
            "Thread/get" => {
                let threads = args["ids"].as_array().unwrap().iter();
                let members = |t: &Value| {
                    self.emails
                        .iter()
                        .filter(|m| m["threadId"] == *t)
                        .map(|m| m["id"].clone())
                        .collect()
                };
                let list: Vec<Value> = threads
                    .map(|t| json!({ "id": t, "emailIds": Value::Array(members(t)) }))
                    .collect();
                json!({ "list": list })
            }
            "Email/changes" => return Err("cannotCalculateChanges"),
            "Email/set" => {
                if args["ifInState"].as_str().is_some_and(|s| s != state) {
                    return Err("stateMismatch");
                }
                let mut updated = Map::new();
                for (id, patch) in args["update"].as_object().into_iter().flatten() {
                    let now = self.now(id);
                    let m = self.emails.iter_mut().find(|m| m["id"] == now).ok_or("notFound")?;
                    for keyword in patch
                        .as_object()
                        .unwrap()
                        .keys()
                        .filter_map(|k| k.strip_prefix("keywords/"))
                    {
                        if m["keywords"][keyword] != true {
                            (m["keywords"][keyword], self.state) = (json!(true), self.state + 1);
                        }
                    }
                    updated.insert(id.clone(), Value::Null);
                }
                let destroy = args["destroy"].as_array().cloned().unwrap_or_default();
                self.emails.retain(|m| !destroy.contains(&m["id"]));
                let new = self.state.to_string();
                json!({
                    "oldState": state, "newState": new, "updated": updated, "destroyed": destroy,
                })
            }
            _ => return Err("unknownMethod"),
        })
    }
}

pub async fn mail(dir: &Scratch, stalwart: Stalwart) -> (Wired, Shared, Result<Mail>) {
    let (wired, server) = stalwart.wire();
    let login = login("agent@example.org");
    let store =
        Store { state: dir.0.join("state"), locks: dir.0.join("locks"), login: login.clone() };
    let access = Access { token: Secret::new("t1"), expires_at: now() + 3600 };
    let tokens = Tokens { access: Some(access), refresh: Some(Secret::new("r1")) };
    store.save(tokens, &store.held().await.unwrap()).unwrap();
    let password = PasswordFile(dir.0.join("password"));
    let auth = Auth { zitadel: Http(Web::new(false).unwrap()), store, login, password };
    let mail = async { Mail::over(Conn::open(auth, Web::new(false)?).await?).await }.await;
    (wired, server, mail)
}

#[tokio::test]
async fn a_redirect_is_followed_only_on_the_requests_origin() {
    let log = Rc::new(RefCell::new(vec![]));
    let seen_by = log.clone();
    let _wire = wired(move |r| {
        seen_by.borrow_mut().push(seen(r));
        match r.url().path() {
            "/found" => moved(302, "/other"),
            "/kept" => moved(307, "/other"),
            "/off" => moved(302, "https://evil.com/collect"),
            "/loop" => moved(303, "loop"),
            _ => reply(200, "{}"),
        }
    });
    let web = Web::new(true).unwrap();
    let token = Secret::new("tok");
    let form = [("password".to_owned(), "pw".to_owned())];
    let send = async |path: &str| {
        let sent =
            web.send(Method::POST, Origin::Auth.url(path), Body::Form(&form), Some(&token)).await;
        let log = log.take().join(", ");
        format!("{} | {log}", shown(sent.map(|page| page.url.path().to_owned())))
    };
    assert_eq!(send("/found").await, "/other | POST /found password=pw @tok, GET /other @tok");
    assert_eq!(
        send("/kept").await,
        "/other | POST /kept password=pw @tok, POST /other password=pw @tok"
    );
    assert_eq!(
        send("/off").await,
        "error: refusing https://evil.com/collect: it is not on https://auth.example.org | POST /off password=pw @tok"
    );
    assert!(send("/loop").await.starts_with(
        "error: /loop: more than 10 redirects | POST /loop password=pw @tok, GET /loop @tok, "
    ));
    let fetched =
        web.fetch(Method::GET, &Origin::Mail.url("/found"), &token, |r| r).await.unwrap_err();
    let failure = (format!("{fetched:#}"), Failure::of(&fetched));
    assert_eq!(failure, ("stalwart: /found: HTTP 302 Found".into(), Failure::Transient));
}

#[tokio::test]
async fn a_refused_token_signs_in_again_once_and_never_under_the_lock() {
    let dir = Scratch::new("refused-token");
    let mut stalwart = Stalwart::new(vec![email("a1", json!({}))]);
    stalwart.refused.insert("t1".into());
    let (_wire, server, mail) = mail(&dir, stalwart).await;
    let mail = mail.unwrap();
    let log = std::mem::take(&mut server.borrow_mut().log);
    assert_eq!(
        log,
        [
            "GET /.well-known/jmap @t1",
            "POST /oauth/v2/token client_id=agent-mail-test&grant_type=refresh_token&refresh_token=r1",
            "GET /.well-known/jmap @t2",
            "POST /jmap/ Mailbox/get @t2"
        ]
    );
    server.borrow_mut().refused.insert("t2".into());
    let claimed = mail.each(true, &[(Current::test("a1"), &[Keyword::Seen][..])]).await;
    assert_eq!(Failure::of(&claimed.unwrap_err()), Failure::TokenRefused);
    assert_eq!(std::mem::take(&mut server.borrow_mut().log), ["POST /jmap/ Email/set @t2"]);
    server.borrow_mut().refused.insert("t3".into());
    let again = mail.get::<Email>(&["a1"]).await;
    assert_eq!(
        Failure::of(&again.unwrap_err()),
        Failure::TokenRefused,
        "a second refusal comes back"
    );
    assert_eq!(server.borrow().log.len(), 3);
}

#[tokio::test(start_paused = true)]
async fn stalwart_over_its_limit_is_waited_out_then_given_up_on() {
    let dir = Scratch::new("busy");
    let (_wire, server, mail) = mail(&dir, Stalwart::new(vec![])).await;
    let (mail, start) = (mail.unwrap(), Instant::now());
    let busy = || ("Email/get", Canned::Http(429));
    server.borrow_mut().canned.extend((0..3).map(|_| busy()));
    assert!(mail.get::<Email>(&["a1"]).await.is_ok());
    assert!(
        start.elapsed() >= Duration::from_millis(3500) && start.elapsed() < Duration::from_secs(5)
    );
    server.borrow_mut().canned.extend((0..7).map(|_| busy()));
    assert_eq!(Failure::of(&mail.get::<Email>(&["a1"]).await.unwrap_err()), Failure::Busy);
}

#[tokio::test]
async fn the_session_must_stay_on_the_mail_origin_and_be_the_logins() {
    let dir = Scratch::new("session");
    let open = async |field: &str, value: &str| {
        let mut stalwart = Stalwart::new(vec![]);
        stalwart.session[field] = json!(value);
        let (_wire, _server, mail) = mail(&dir, stalwart).await;
        mail.err().map(|e| (format!("{e:#}"), Failure::of(&e).permanent()))
    };
    assert_eq!(open("username", "agent@example.org").await, None);
    assert_eq!(
        open("username", "ada@agent.example.org").await,
        Some((
            "the token is for ada@agent.example.org, not the configured agent@example.org".into(),
            true
        ))
    );
    for url in ["apiUrl", "uploadUrl", "downloadUrl", "eventSourceUrl"] {
        let off = open(url, "https://evil.com/{accountId}").await.unwrap();
        assert!(
            off.0.starts_with(
                "the JMAP session names a URL off the mail origin: refusing https://evil.com/"
            ),
            "{url}: {off:?}"
        );
    }
}

#[tokio::test]
async fn gets_go_in_the_chunks_the_server_allows() {
    let dir = Scratch::new("chunks");
    let mut stalwart = Stalwart::new((1..=5).map(|i| email(&format!("m{i}"), json!({}))).collect());
    stalwart.session["capabilities"]["urn:ietf:params:jmap:core"]["maxObjectsInGet"] = json!(2);
    let (_wire, server, mail) = mail(&dir, stalwart).await;
    let got: Vec<Email> = mail.unwrap().get(&["m1", "m2", "m3", "m4", "m5"]).await.unwrap();
    assert_eq!(got.len(), 5);
    assert_eq!(server.borrow().log.iter().filter(|l| l.contains("Email/get")).count(), 3);
}

#[tokio::test]
async fn delivery_keeps_the_copy_only_while_it_may_have_gone_out() {
    let dir = Scratch::new("deliver");
    let created = || {
        ("EmailSubmission/set", Canned::Answer(json!({ "created": { "send": { "id": "S1" } } })))
    };
    let status = |delivered: &str| {
        let reply = json!({ "delivered": delivered, "smtpReply": "550 5.1.2 no such" });
        let status = json!({ "nosuch@example.org": reply });
        ("EmailSubmission/get", Canned::Answer(json!({ "list": [{ "deliveryStatus": status }] })))
    };
    let not_created = || {
        (
            "EmailSubmission/set",
            Canned::Answer(json!({ "notCreated": { "send": { "type": "forbiddenFrom" } } })),
        )
    };
    let found = |ids: Value| ("EmailSubmission/query", Canned::Answer(json!({ "ids": ids })));
    let cases = [
        (
            vec![created(), status("no")],
            "error: removed: submission S1: every recipient was refused, nothing was sent\n  nosuch@example.org: 550 5.1.2 no such",
        ),
        (vec![created(), status("unknown")], "S1 Accepted([])"),
        (vec![created(), ("EmailSubmission/get", Canned::Http(500))], "S1 Unknown"),
        (
            vec![not_created()],
            r#"error: removed: stalwart did not create it: {"type":"forbiddenFrom"}"#,
        ),
        (
            vec![("EmailSubmission/set", Canned::Http(502)), found(json!(["S2"])), status("yes")],
            "S2 Accepted([])",
        ),
        (
            vec![("EmailSubmission/set", Canned::Http(502)), found(json!([]))],
            "error: unknown: stalwart: /jmap/: HTTP 502 Bad Gateway",
        ),
        (
            vec![
                ("EmailSubmission/set", Canned::Http(502)),
                ("EmailSubmission/query", Canned::Http(500)),
            ],
            "error: unknown: looking for its submission failed: stalwart: /jmap/: HTTP 500 Internal Server Error: \
             stalwart: /jmap/: HTTP 502 Bad Gateway",
        ),
        (
            vec![not_created(), ("Email/set", Canned::Http(500))],
            r#"error: unremoved: stalwart: /jmap/: HTTP 500 Internal Server Error: stalwart did not create it: {"type":"forbiddenFrom"}"#,
        ),
    ];
    let to = Recipients::check(
        &login("agent@example.org"),
        address(POOL),
        addresses("nosuch@example.org"),
        vec![],
    )
    .unwrap();
    for (canned, want) in cases {
        let mut stalwart = Stalwart::new(vec![email("d1", json!({}))]);
        stalwart.canned.extend(canned);
        let (_wire, server, mail) = mail(&dir, stalwart).await;
        let mail = mail.unwrap();
        let outbox = Outbox::open(&mail).await.unwrap();
        let (unknown, removed, unremoved) =
            ("unknown".into(), "removed".into(), "unremoved".into());
        let left = Leftover { unknown, removed, unremoved };
        let delivered = outbox.deliver(&Current::test("d1"), &to, left).await;
        assert_eq!(shown(delivered.map(|(id, delivery)| format!("{id} {delivery:?}"))), want);
        let kept = server.borrow().emails.len() == 1;
        assert_eq!(kept, !want.starts_with("error: removed"), "{want}");
    }
}

fn watching(timeout: u64) -> wait::Args {
    wait::Args {
        watching: wait::Watching { topic: vec!["pool".parse().unwrap()], thread: vec![] },
        session: None,
        timeout: Some(Duration::from_secs(timeout)),
        json: false,
    }
}

#[tokio::test(start_paused = true)]
async fn a_wait_signs_in_again_when_its_token_expires_and_then_claims() {
    let dir = Scratch::new("wait-expired");
    let mut stalwart = Stalwart::new(vec![email("a1", json!({}))]);
    stalwart.expire_on_stream = true;
    let (_wire, server, mail) = mail(&dir, stalwart).await;
    assert_eq!(wait::watch(&mail.unwrap(), watching(600)).await.unwrap(), ExitCode::SUCCESS);
    let server = server.borrow();
    let opened: Vec<u64> = server.opened.iter().map(|d| d.as_secs()).collect();
    assert_eq!(opened, [0, 5], "the second stream opens after the 5 s pause");
    assert!(server.log.iter().any(|l| l.starts_with("POST /oauth/v2/token")));
    assert!(server.log.iter().any(|l| l == "POST /jmap/ Email/set @t2"));
    assert_eq!(server.emails[0]["keywords"]["$seen"], true);
}

#[tokio::test(start_paused = true)]
async fn a_wait_has_stalwart_parse_only_the_mail_it_delivers() {
    let dir = Scratch::new("wait-parsed");
    let other = |id| email(id, json!({ "to": [{ "email": "agent+other@example.org" }] }));
    let stalwart = Stalwart::new(vec![other("o1"), email("a1", json!({})), other("o2")]);
    let (_wire, server, mail) = mail(&dir, stalwart).await;
    assert_eq!(wait::watch(&mail.unwrap(), watching(600)).await.unwrap(), ExitCode::SUCCESS);
    assert_eq!(server.borrow().parsed, BTreeSet::from(["a1".to_owned()]));
}

/// A mail to `to` as Stalwart delivers Owen's, signed now.
fn signed_to(to: &str) -> String {
    let message = format!(
        "From: Owner <{HIM}>\r\nTo: {to}\r\nSubject: go\r\nContent-Type: text/plain\r\n\r\nyes\r\n"
    );
    let signer = Signer::from_seed(&format!("{}Bwc", "BwcH".repeat(10))).unwrap();
    let stamp = Stamp { account: HIM.parse().unwrap(), sender: HIM.parse().unwrap(), t: now() };
    format!("{HEADER}: {}\r\n{message}", signer.sign(&stamp, message.as_bytes()))
}

fn store_in(dir: &Scratch) -> Store {
    Store {
        state: dir.0.join("state"),
        locks: dir.0.join("locks"),
        login: login("agent@example.org"),
    }
}

/// Which of Owen's mail to the pool, unsigned mail to it and his mail to another topic a session's own wait on both
/// topics claims, while `listener` is the pool's.
async fn claimed_beside(name: &str, listener: Heartbeat) -> Vec<bool> {
    let dir = Scratch::new(name);
    let other = json!({ "to": [{ "email": "agent+other@example.org" }] });
    let mut stalwart =
        Stalwart::new(vec![email("s1", json!({})), email("u1", json!({})), email("y1", other)]);
    let unsigned = format!("From: {HIM}\r\nTo: {POOL}\r\n\r\nyes\r\n");
    stalwart.downloads.extend([reply(200, signed_to(POOL)), reply(200, unsigned)]);
    let (_wire, server, mail) = mail(&dir, stalwart).await;
    store_in(&dir).listeners().unwrap().put(&"mod-1".parse().unwrap(), listener).unwrap();
    let topics = ["pool", "other"].map(|t| t.parse().unwrap()).into();
    let watching = wait::Watching { topic: topics, thread: vec![] };
    let own = wait::Args {
        watching,
        session: None,
        timeout: Some(Duration::from_secs(600)),
        json: false,
    };
    assert_eq!(wait::watch(&mail.unwrap(), own).await.unwrap(), ExitCode::SUCCESS);
    server.borrow().emails.iter().map(|m| m["keywords"]["$seen"] == true).collect()
}

#[tokio::test(start_paused = true)]
async fn a_sessions_own_wait_leaves_owens_word_to_a_live_listener_on_its_topic_and_nothing_else() {
    let pool =
        |pid, scanned| Heartbeat { pid, topics: [POOL.into()].into(), threads: [].into(), scanned };
    let me = std::process::id();
    assert_eq!(claimed_beside("beside-live", pool(me, now())).await, [false, true, true]);
    let stale = now() - 3 * RECHECK.as_secs() as i64 - 1;
    assert_eq!(claimed_beside("beside-stale", pool(me, stale)).await, [true, true, true]);
    let gone = std::process::Command::new("true").spawn().unwrap();
    let pid = gone.id();
    drop(gone.wait_with_output().unwrap());
    assert_eq!(claimed_beside("beside-gone", pool(pid, now())).await, [true, true, true]);
}

#[tokio::test(start_paused = true)]
async fn a_listener_beats_after_its_scans_and_leaves_when_its_wait_ends() {
    let dir = Scratch::new("listener-beats");
    let (_wire, _server, mail) = mail(&dir, Stalwart::new(vec![])).await;
    let (store, id): (Store, SessionId) = (store_in(&dir), "mod-1".parse().unwrap());
    store.records().unwrap().record(&id, vec!["pool".parse().unwrap()], vec![]).unwrap();
    let listeners = store.listeners().unwrap();
    let args = wait::Args {
        watching: wait::Watching::default(),
        session: Some(id.clone()),
        timeout: Some(Duration::from_secs(600)),
        json: true,
    };
    let mail = mail.unwrap();
    let (ended, (first, again)) = tokio::join!(wait::watch(&mail, args), async {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let first = listeners.all().unwrap();
        listeners.leave(&id).unwrap();
        tokio::time::sleep(RECHECK + Duration::from_secs(1)).await;
        (first, listeners.all().unwrap())
    });
    assert_eq!(ended.unwrap(), ExitCode::from(2));
    let mine = &first["mod-1"];
    assert_eq!(
        (mine.pid, mine.topics.iter().map(String::as_str).collect::<Vec<_>>()),
        (std::process::id(), vec![POOL])
    );
    assert!(again.contains_key("mod-1"), "a later scan writes it again");
    assert!(listeners.all().unwrap().is_empty(), "the ended wait takes it out");
}

#[tokio::test(start_paused = true)]
async fn a_wait_reopens_an_idle_silent_or_dropped_stream_and_gives_up_at_its_deadline() {
    let dir = Scratch::new("wait-idle");
    let mut stalwart = Stalwart::new(vec![]);
    stalwart.streams.extend([stream(&[PING]), open(&[], false), reply(200, "")]);
    let (_wire, server, mail) = mail(&dir, stalwart).await;
    assert_eq!(wait::watch(&mail.unwrap(), watching(400)).await.unwrap(), ExitCode::from(2));
    let server = server.borrow();
    let opened: Vec<u64> = server.opened.iter().map(|d| d.as_secs()).collect();
    assert_eq!(
        opened,
        [0, 150, 186, 187, 337],
        "idle reopens at once; one silent past 35 s or closed, after 1 s; the last lasts to the deadline"
    );
    let checks = server.log.iter().filter(|l| l.contains("Email/query")).count();
    assert_eq!(checks, opened.len(), "one check per stream, so a ping wakes nothing");
}

#[tokio::test(start_paused = true)]
async fn an_upload_is_bounded_by_its_progress_not_its_size() {
    let bytes = vec![0; 10 * 64 * 1024];
    let piece = async |body: &mut reqwest::Body| {
        std::future::poll_fn(|w| http_body::Body::poll_frame(Pin::new(&mut *body), w))
            .await
            .map(|f| f.unwrap())
    };
    let slow = Web::paced(&bytes, async |mut body| {
        let mut sent = 0;
        while let Some(frame) = piece(&mut body).await {
            sent += frame.into_data().unwrap().len();
            tokio::time::sleep(Duration::from_secs(9)).await;
        }
        Ok(sent)
    });
    let start = Instant::now();
    assert_eq!(slow.await.unwrap(), bytes.len());
    assert!(start.elapsed() >= Duration::from_secs(90), "{:?}", start.elapsed());
    let stalled = Web::paced(&bytes, async |mut body| {
        piece(&mut body).await;
        tokio::time::sleep(Duration::from_secs(60)).await;
        Ok(0)
    });
    assert_eq!(stalled.await.unwrap_err().to_string(), "the upload made no progress for 10s");
}

#[tokio::test(start_paused = true)]
async fn a_download_is_bounded_by_its_progress_not_its_size() {
    let dir = Scratch::new("download");
    let mut stalwart = Stalwart::new(vec![]);
    stalwart.downloads.extend([trickle(10, 9), trickle(10, 9), trickle(2, 11)]);
    let (_wire, _server, mail) = mail(&dir, stalwart).await;
    let mail = mail.unwrap();
    assert_eq!(mail.download("b1", 1 << 20).await.unwrap().map(|b| b.len()), Some(10 * 1024));
    assert_eq!(mail.download("b2", 9 * 1024).await.unwrap(), None, "past its bound");
    let stalled = mail.download("b3", 1 << 20).await.unwrap_err();
    assert!(format!("{stalled:#}").starts_with("the download stalled"), "{stalled:#}");
}

#[tokio::test(start_paused = true)]
async fn a_busy_server_is_checked_again_in_30_s_and_a_config_error_ends_the_wait() {
    let dir = Scratch::new("wait-busy");
    let mut stalwart = Stalwart::new(vec![email("a1", json!({}))]);
    stalwart.canned.extend((0..7).map(|_| ("Email/get", Canned::Http(429))));
    let (_wire, server, mail) = mail(&dir, stalwart).await;
    let mail = mail.unwrap();
    let start = Instant::now();
    assert_eq!(wait::watch(&mail, watching(600)).await.unwrap(), ExitCode::SUCCESS);
    assert!(start.elapsed() >= Duration::from_millis(31_500 + 30_000), "{:?}", start.elapsed());
    server.borrow_mut().emails.clear();
    (server.borrow_mut().expire_on_stream, server.borrow_mut().refresh_refused) = (true, true);
    let ended = wait::watch(&mail, watching(600)).await.unwrap_err();
    assert!(Failure::of(&ended).permanent(), "{ended:#}");
}

/// How a wait ends, and when, with every request to the mail host failing over `down`.
async fn through_outage(
    emails: Vec<Value>,
    down: std::ops::Range<u64>,
    timeout: u64,
) -> (u64, String) {
    let dir = Scratch::new("wait-outage");
    let (_wire, server, mail) = mail(&dir, Stalwart::new(emails)).await;
    server.borrow_mut().down = down;
    let start = Instant::now();
    let ended = match wait::watch(&mail.unwrap(), watching(timeout)).await {
        Ok(code) if code == ExitCode::SUCCESS => "delivered".into(),
        Ok(code) => format!("{code:?}"),
        Err(e) => format!("error: {e:#}"),
    };
    (start.elapsed().as_secs(), ended)
}

#[tokio::test(start_paused = true)]
async fn a_wait_fails_once_no_scan_has_finished_for_5_minutes_and_rides_out_a_shorter_outage() {
    let failed = "error: no scan finished in 300s: stalwart: /jmap/eventsource/: HTTP 503 Service Unavailable";
    assert_eq!(through_outage(vec![], 0..u64::MAX, 3600).await, (300, failed.into()));
    assert_eq!(
        through_outage(vec![], 200..u64::MAX, 3600).await,
        (450, failed.into()),
        "5 minutes after the scan at 150 s"
    );
    assert_eq!(
        through_outage(vec![email("a1", json!({}))], 0..250, 3600).await,
        (300, "delivered".into()),
        "the retry due at 315 s comes at the bound instead"
    );
    assert_eq!(
        through_outage(vec![], 0..u64::MAX, 60).await,
        (
            60,
            "error: the wait timed out before any scan finished: stalwart: /jmap/eventsource/: HTTP 503 Service Unavailable"
                .into()
        )
    );
}

#[tokio::test]
async fn change_lists_that_do_not_read_or_do_not_move_and_queries_whose_pages_keep_moving_fail() {
    let dir = Scratch::new("pages");
    let changes = |reply: Value| ("Email/changes", Canned::Answer(reply));
    let mut stalwart = Stalwart::new(vec![]);
    stalwart.canned.extend([
        changes(json!({ "created": [1], "updated": [], "newState": "s1" })),
        changes(json!({ "created": [], "updated": [], "newState": "s0", "hasMoreChanges": true })),
    ]);
    let moved = || ("Email/query", Canned::Error("anchorNotFound", false));
    stalwart.canned.extend((0..10).map(|_| moved()));
    let (_wire, _server, mail) = mail(&dir, stalwart).await;
    let mail = mail.unwrap();
    let changed = async || shown(mail.changes("s0").await.map(|c| format!("{c:?}")));
    assert_eq!(
        changed().await,
        "error: stalwart's Email/changes reply: invalid type: integer `1`, expected a string"
    );
    assert_eq!(changed().await, "error: stalwart's Email/changes has more past s0");
    assert_eq!(
        shown(mail.query(json!({})).await.map(|ids| ids.len())),
        "error: the mail kept changing under Email/query's pages"
    );
}

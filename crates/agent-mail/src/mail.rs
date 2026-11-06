use std::{cell::RefCell, fs::File, ops::ControlFlow};

use ::jmap::{
    Changed, Event, Events, Failed, Got, Limits, Queried, Request, Session, Set, Urls, View,
};
use private::{Secret, random::below};
use reqwest::{Method, Response, header};
use text::Address;

use crate::{
    auth::Auth,
    jmap::{Call, Cap, MAIL, jmap_id, keywords},
    policy::Login,
    prelude::*,
    store::Config,
    web::{self, Origin, Pinned, TIMEOUT, Web},
};

pub const MAX_BODY_BYTES: usize = 64 * 1024;
const PAGE: usize = 256;
/// Email properties Stalwart answers by parsing each message, about 27 ms a mail on cx33-1 (10-29), so a get of
/// [`PARSED_PER_GET`] of them stays well inside [`TIMEOUT`].
const PARSED: &[&str] = &["attachments", "bodyStructure", "textBody", "htmlBody", "bodyValues"];
const PARSED_PER_GET: usize = 64;
const RESTARTS: usize = 10;

/// One page of an Email/query's ids; `first` again when the pages started over.
pub struct Page {
    pub ids: Vec<String>,
    pub first: bool,
}

pub use ::jmap::mail::Status;

::jmap::view! {
    under MAIL {
        Email in "Email/get" {
            id: String = "id", thread: String = "threadId", received: String = "receivedAt",
            from: Vec<Addr> = "from", to: Vec<Addr> = "to", cc: Vec<Addr> = "cc",
            subject: String = "subject", keywords: HashMap<String, bool> = "keywords",
            mailboxes: HashMap<String, bool> = "mailboxIds", message_id: Vec<String> = "messageId",
            in_reply_to: Vec<String> = "inReplyTo", references: Vec<String> = "references",
            preview: String = "preview", content_type: String = "header:Content-Type",
            notify: Vec<Addr> = "header:Disposition-Notification-To:asAddresses",
            original_recipient: String = "header:Original-Recipient:asText", blob: String = "blobId",
        }
        Body in "Email/get" with ("fetchTextBodyValues", MAX_BODY_BYTES) {
            id: String = "id", text: Vec<Part> = "textBody", attachments: Vec<Part> = "attachments",
            values: HashMap<String, Text> = "bodyValues", structure: Part = "bodyStructure",
        }
        Thread in "Thread/get" { emails: Vec<String> = "emailIds" }
        Mailbox in "Mailbox/get" { id: String = "id", role: String = "role" }
        Identity in "Identity/get" { id: String = "id", email: String = "email" }
        Submission in "EmailSubmission/get" {
            delivery: Option<HashMap<String, Status>> = "deliveryStatus",
        }
    }
}

::jmap::object! {
    Addr { email: String = "email" }
    Part {
        blob: String = "blobId", size: usize = "size", name: String = "name",
        kind: String = "type", part: String = "partId", parts: Vec<Part> = "subParts",
    }
    Text { value: String = "value", truncated: bool = "isTruncated" }
}

pub const QUERY: Call<Queried> = Call::mail("Email/query", Some("anchorNotFound"));
pub const CHANGES: Call<Changed> = Call::mail("Email/changes", Some("cannotCalculateChanges"));
pub const SET: Call<Set> = Call::mail("Email/set", Some("stateMismatch"));
pub const IMPORT: Call<Set> = Call::mail("Email/import", None);
pub const SUBMIT: Call<Set> = Call::mail("EmailSubmission/set", None);
pub const SUBMISSIONS: Call<Queried> = Call::mail("EmailSubmission/query", None);

// `$agentmail` marks the Sent copy of what agent-mail sends, so answering from Sent, meant for other tools, skips it.
keywords! {
    Seen = "$seen", Answered = "$answered", Handled = "$handled", MdnSent = "$mdnsent",
    AgentMail = "$agentmail", Draft = "$draft",
}

/// What Stalwart took of a /set.
pub trait SetExt {
    /// The id of what it created under `key`.
    fn id(&self, key: &str) -> Result<&str>;
    /// Ok once it took every update and destroy.
    fn taken(&self) -> Result<()>;
}

impl SetExt for Set {
    fn id(&self, key: &str) -> Result<&str> {
        let refused = || {
            format!(
                "stalwart did not create it: {}",
                self.not_created.get(key).unwrap_or(&Value::Null)
            )
        };
        self.created.get(key).and_then(|c| c["id"].as_str()).with_context(refused)
    }

    fn taken(&self) -> Result<()> {
        ensure!(
            self.not_updated.is_empty() && self.not_destroyed.is_empty(),
            "stalwart refused {}",
            json!([self.not_updated, self.not_destroyed])
        );
        Ok(())
    }
}

impl Email {
    pub fn has(&self, keyword: Keyword) -> bool {
        self.keywords.get(keyword.name()) == Some(&true)
    }
}

/// Matched here, not by the server, which answers its To and Cc filters from a full-text index that is built after
/// ingest and finds agent+pool-split for agent+pool.
pub fn names<'a>(topics: &[Address], mut on: impl Iterator<Item = &'a Addr>) -> bool {
    on.any(|a| a.email.parse().is_ok_and(|a| topics.contains(&a)))
}

pub fn keywords(set: &[Keyword]) -> Value {
    Value::Object(set.iter().map(|k| (k.name().to_owned(), json!(true))).collect())
}

/// The id an email has now. Stalwart puts the thread in an email's id and answers Email/get with the thread of
/// the id it was asked for, so after a merge only a query returns the current one.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Current(String);

impl Current {
    fn new(id: &str) -> Result<Current> {
        Ok(Current(jmap_id(id)?.into()))
    }

    #[cfg(test)]
    pub fn test(id: &str) -> Current {
        Current(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub type Mark = (Current, &'static [Keyword]);

/// Two windows of Stalwart's default request limit, 1000 a minute for each account.
pub const BUSY_FOR: Duration = Duration::from_secs(120);
const PING: Duration = Duration::from_secs(30);
const SILENCE: Duration = Duration::from_secs(PING.as_secs() + 5);

pub fn fill(template: &str, values: &[(&str, &str)]) -> Result<Pinned> {
    Origin::Mail.check(&::jmap::fill(template, values))
}

pub fn check_account(username: &str, login: &Login) -> Result<()> {
    match username.eq_ignore_ascii_case(&login.to_string()) {
        true => Ok(()),
        false => Err(Failure::Config.with(format!(
            "the token is for {}, not the configured {login}",
            username.escape_debug()
        ))),
    }
}

pub struct Conn {
    pub auth: Auth,
    pub web: Web,
    token: RefCell<Secret>,
}

impl Conn {
    pub async fn open(auth: Auth, web: Web) -> Result<Conn> {
        let token = RefCell::new(auth.access().await?.token);
        Ok(Conn { auth, web, token })
    }

    /// Runs `op` with the session's token. After a refused token it signs in again and runs it once more; over
    /// Stalwart's limits it backs off, for no less than any Retry-After, and gives up before [`BUSY_FOR`] of pauses.
    /// When `locked`, either refusal comes back at once, so the lock can be released first.
    pub async fn retry<T>(&self, locked: bool, op: impl AsyncFn(Secret) -> Result<T>) -> Result<T> {
        let (mut signed_in, mut backoff, mut waited) =
            (locked, Duration::from_millis(500), Duration::ZERO);
        loop {
            let used = self.token.borrow().clone();
            let e = match op(used.clone()).await {
                Err(e) => e,
                done => return done,
            };
            match Failure::of(&e) {
                Failure::TokenRefused if !signed_in => {
                    self.auth.reject(&used).await?;
                    (*self.token.borrow_mut(), signed_in) = (self.auth.access().await?.token, true);
                }
                Failure::Busy(after) if !locked => {
                    let pause = after.unwrap_or_default().max(backoff)
                        + Duration::from_millis(below(250)? as u64);
                    if waited + pause > BUSY_FOR {
                        return Err(e.context(format!(
                            "stalwart stayed over its limits, and another retry would wait past {}s in all",
                            BUSY_FOR.as_secs()
                        )));
                    }
                    tokio::time::sleep(pause).await;
                    (waited, backoff) =
                        (waited + pause, (backoff * 2).min(Duration::from_secs(16)));
                }
                _ => return Err(e),
            }
        }
    }

    pub async fn session(&self) -> Result<Session> {
        let url = Origin::Mail.url("/.well-known/jmap");
        let get =
            async |t| self.web.send(Method::GET, url.clone(), web::Body::Empty, Some(&t)).await;
        self.retry(false, async |t| get(t).await?.jmap()).await
    }
}

pub struct Mail {
    pub conn: Conn,
    /// Read for the primary account of each capability: mail's at open, and the others when first used, so a bad one
    /// breaks only what uses it.
    session: Session,
    urls: Urls,
    api: Pinned,
    limits: Limits,
    mailboxes: HashMap<String, String>,
}

impl Mail {
    pub async fn open() -> Result<Mail> {
        Mail::with(Config::load()?).await
    }

    pub async fn with(config: Config) -> Result<Mail> {
        Mail::over(Conn::open(Auth::new(config)?, Web::new(false)?).await?).await
    }

    pub async fn over(conn: Conn) -> Result<Mail> {
        let session = conn.session().await?;
        let off = "the JMAP session names a URL off the mail origin";
        let urls = session.checked(|url| Origin::Mail.check(url).map(drop)).context(off)?;
        let api = Origin::Mail.check(&urls.api).context(off)?;
        check_account(&session.username, &conn.auth.login)?;
        session.primary(MAIL.account)?;
        let limits = session.limits();
        let mut mail = Mail { conn, session, urls, api, limits, mailboxes: HashMap::new() };
        mail.mailboxes = mail.all::<Mailbox>().await?.into_iter().map(|m| (m.role, m.id)).collect();
        Ok(mail)
    }

    fn account(&self, cap: Cap) -> Result<&str> {
        self.session.primary(cap.account)
    }

    pub fn login(&self) -> &Login {
        &self.conn.auth.login
    }

    pub fn mailbox(&self, role: &str) -> Result<String> {
        let named = || format!("no {}{} mailbox", role[..1].to_uppercase(), &role[1..]);
        self.mailboxes.get(role).cloned().with_context(named)
    }

    pub async fn reconnect(&self) -> Result<()> {
        self.conn.session().await.map(drop)
    }

    pub async fn call<T: DeserializeOwned>(&self, call: Call<T>, args: Value) -> Result<T> {
        let name = call.0;
        let answer = self.answer(false, call, args).await?;
        answer.with_context(|| format!("stalwart answered {name} with an error"))
    }

    pub async fn answer<T: DeserializeOwned>(
        &self,
        locked: bool,
        call: Call<T>,
        args: Value,
    ) -> Result<Option<T>> {
        self.answers(locked, &[(call, args)]).await?.remove(0)
    }

    async fn answers<T: DeserializeOwned>(
        &self,
        locked: bool,
        calls: &[(Call<T>, Value)],
    ) -> Result<Vec<Result<Option<T>>>> {
        let mut using = vec![::jmap::CORE];
        for &urn in calls.iter().flat_map(|(call, _)| call.2.using) {
            if !using.contains(&urn) {
                using.push(urn);
            }
        }
        let mut request = Request::new(&using);
        let sent = (calls.iter())
            .map(|(call, args)| {
                let mut args = args.clone();
                args["accountId"] = json!(self.account(call.2)?);
                Ok((call, request.call(call.0, args)))
            })
            .collect::<Result<Vec<_>>>()?;
        let response: ::jmap::Response = self
            .conn
            .retry(locked, async |t| {
                let post = |r: reqwest::RequestBuilder| r.timeout(TIMEOUT).json(&request);
                Ok(self.conn.web.fetch(Method::POST, &self.api, &t, post).await?.json().await?)
            })
            .await?;
        let read = |(call, id): (&Call<T>, ::jmap::Call)| match response.answer(id) {
            Ok(answer) => Ok(Some(answer)),
            Err(Failed::Refused(e)) if Some(e.kind.as_str()) == call.1 => Ok(None),
            // A method answered with an error made no change (RFC 8620 §3.6.2).
            Err(Failed::Refused(e)) => {
                Err(Failure::Unprocessed.with(format!("stalwart refused {}: {}", call.0, e.kind)))
            }
            Err(Failed::Unanswered) => bail!("stalwart sent no reply to {}", call.0),
            Err(Failed::Unreadable(e)) => Err(e).context(format!("stalwart's {} reply", call.0)),
        };
        Ok(sent.into_iter().map(read).collect())
    }

    pub async fn set(&self, locked: bool, args: Value) -> Result<Option<Set>> {
        let Some(answer) = self.answer(locked, SET, args).await? else { return Ok(None) };
        answer.taken()?;
        Ok(Some(answer))
    }

    async fn got<V: View>(&self, locked: bool, ids: Value) -> Result<Got<V>> {
        self.answer(locked, Call::new(V::GET, None, V::CAP), V::args(ids)).await?.context(V::GET)
    }

    pub async fn get<V: View>(&self, ids: &[impl Serialize]) -> Result<Vec<V>> {
        let parses = V::PROPERTIES.iter().any(|p| PARSED.contains(p));
        let mut found = vec![];
        for chunk in
            ids.chunks(if parses { PARSED_PER_GET.min(self.limits.get) } else { self.limits.get })
        {
            found.extend(self.got(false, json!(chunk)).await?.list);
        }
        Ok(found)
    }

    /// Every object of `V`, from a get with no ids. Past the server's get limit Stalwart cuts such a reply short, where
    /// RFC 8620 §5.1 asks for requestTooLarge, so a reply that fills the limit is refused.
    pub async fn all<V: View>(&self) -> Result<Vec<V>> {
        let all = self.got(false, Value::Null).await?.list;
        ensure!(
            all.len() < self.limits.get,
            "stalwart answered {} with {}, its whole get limit, so it may have left some out",
            V::GET,
            all.len()
        );
        Ok(all)
    }

    pub async fn state(&self) -> Result<String> {
        Ok(self.got::<Email>(false, json!([])).await?.state)
    }

    /// Every id the filter matches, unsorted, since a sort is answered from the full-text index, which lags ingest.
    pub async fn query(&self, filter: Value) -> Result<Vec<String>> {
        let mut ids = vec![];
        self.pages(json!({ "filter": filter, "limit": PAGE }), async |page: Page| {
            if page.first {
                ids.clear();
            }
            ids.extend(page.ids);
            Ok(ControlFlow::Continue(()))
        })
        .await?;
        Ok(ids)
    }

    /// Email/query over `query`, page after page until `each` breaks or a page comes back shorter than the limit the
    /// server used, which it may set below the one asked (RFC 8620 §5.5). A lost anchor starts the pages over.
    pub async fn pages(
        &self,
        query: Value,
        mut each: impl AsyncFnMut(Page) -> Result<ControlFlow<()>>,
    ) -> Result<()> {
        let asked = query["limit"].as_u64();
        let (mut anchor, mut restarts) = (None::<String>, 0);
        loop {
            let mut at = query.clone();
            match &anchor {
                Some(last) => (at["anchor"], at["anchorOffset"]) = (json!(last), json!(1)),
                None => at["position"] = json!(0),
            }
            let Some(page) = self.answer(false, QUERY, at).await? else {
                restarts += 1;
                ensure!(restarts < RESTARTS, "the mail kept changing under Email/query's pages");
                anchor = None;
                continue;
            };
            let short = page.limit.or(asked).is_none_or(|l| (page.ids.len() as u64) < l);
            let first = anchor.is_none();
            anchor = page.ids.last().cloned();
            if each(Page { ids: page.ids, first }).await?.is_break() || short {
                return Ok(());
            }
        }
    }

    pub async fn current(&self, id: &str) -> Result<Option<Current>> {
        Ok(self.currents(&[id.to_owned()]).await?.remove(0))
    }

    /// The id each of `ids` has now, one query each, as many to a request as Stalwart takes, since its limit counts
    /// requests. Stalwart refuses an id outside its alphabet in Email/query; Email/get just finds no such email.
    pub async fn currents(&self, ids: &[String]) -> Result<Vec<Option<Current>>> {
        const FIND: Call<Queried> = Call::mail("Email/query", Some("invalidArguments"));
        let valid: Vec<&String> = ids.iter().filter(|id| jmap_id(id).is_ok()).collect();
        let mut now = HashMap::new();
        for chunk in valid.chunks(self.limits.calls) {
            let calls: Vec<_> =
                chunk.iter().map(|id| (FIND, json!({ "filter": { "id": [id] } }))).collect();
            for (id, answer) in chunk.iter().zip(self.answers(false, &calls).await?) {
                if let Some(found) = answer?.and_then(|f| f.ids.into_iter().next()) {
                    now.insert(*id, Current::new(&found)?);
                }
            }
        }
        Ok(ids.iter().map(|id| now.get(id).cloned()).collect())
    }

    pub async fn email<V: View>(&self, id: &str) -> Result<(Current, V)> {
        let missing = || format!("no email {}", id.escape_debug());
        let current = self.current(id).await?.with_context(missing)?;
        let found = self.get::<V>(&[&current]).await?.into_iter().next();
        Ok((current, found.with_context(missing)?))
    }

    pub async fn members(&self, threads: &[&String]) -> Result<Vec<String>> {
        Ok(self.get::<Thread>(threads).await?.into_iter().flat_map(|t| t.emails).collect())
    }

    pub async fn changes(&self, since: &str) -> Result<Option<(Vec<String>, String)>> {
        let (mut ids, mut state) = (BTreeSet::new(), since.to_owned());
        loop {
            let args = json!({ "sinceState": state, "maxChanges": PAGE });
            let Some(changed) = self.answer(false, CHANGES, args).await? else { return Ok(None) };
            ids.extend(changed.created.into_iter().chain(changed.updated));
            if !changed.more {
                return Ok(Some((ids.into_iter().collect(), changed.new_state)));
            }
            ensure!(changed.new_state != state, "stalwart's Email/changes has more past {state}");
            state = changed.new_state;
        }
    }

    pub async fn mark(&self, marks: &[Mark]) -> Result<()> {
        for chunk in marks.chunks(self.limits.set) {
            self.set(false, json!({ "update": patches(chunk) })).await?;
        }
        Ok(())
    }

    pub fn created(answer: &Set, key: &str) -> Result<Current> {
        Current::new(answer.id(key)?)
    }

    pub async fn upload(&self, bytes: &[u8], kind: &str) -> Result<String> {
        let url = fill(&self.urls.upload, &[("accountId", self.account(MAIL)?)])?;
        let post = async |t| {
            Web::paced(bytes, async |body| {
                let post =
                    |r: reqwest::RequestBuilder| r.header(header::CONTENT_TYPE, kind).body(body);
                Ok(self.conn.web.fetch(Method::POST, &url, &t, post).await?.json::<Value>().await?)
            })
            .await
        };
        let uploaded = self.conn.retry(false, post).await?;
        Ok(uploaded["blobId"].as_str().context("stalwart's upload reply names no blob")?.into())
    }

    /// `m` as Stalwart holds it, or why it can't be read whole.
    pub async fn message(&self, m: &Email) -> Result<Result<Vec<u8>, String>> {
        const MOST: usize = 32 << 20;
        Ok(self.download(&m.blob, MOST).await?.ok_or(format!(
            "the message is over {} MiB, so its signature goes unchecked",
            MOST >> 20
        )))
    }

    pub async fn download(&self, blob: &str, most: usize) -> Result<Option<Vec<u8>>> {
        let ids = [("accountId", self.account(MAIL)?), ("blobId", jmap_id(blob)?)];
        let fixed = [("name", "attachment"), ("type", "application/octet-stream")];
        let url = fill(&self.urls.download, &[ids, fixed].concat())?;
        self.conn
            .retry(false, async |t| {
                let get = self.conn.web.fetch(Method::GET, &url, &t, |r| r);
                let get = tokio::time::timeout(TIMEOUT, get).await;
                let (mut response, mut bytes) =
                    (get.context("the download sent no headers in time")??, vec![]);
                while let Some(chunk) = tokio::time::timeout(TIMEOUT, response.chunk())
                    .await
                    .context("the download stalled")??
                {
                    if bytes.len() + chunk.len() > most {
                        return Ok(None);
                    }
                    bytes.extend_from_slice(&chunk);
                }
                Ok(Some(bytes))
            })
            .await
    }

    pub async fn pushes(&self) -> Result<Pushes> {
        let ping = PING.as_secs().to_string();
        let values = [("types", "Email"), ("closeafter", "no"), ("ping", ping.as_str())];
        let (url, token) = (fill(&self.urls.events, &values)?, self.conn.token.borrow().clone());
        let accept = |r: reqwest::RequestBuilder| r.header(header::ACCEPT, "text/event-stream");
        let opened =
            tokio::time::timeout(TIMEOUT, self.conn.web.fetch(Method::GET, &url, &token, accept));
        let opened = opened.await.context("the push stream sent no response headers in time")??;
        Ok(Pushes(opened, Frames::default()))
    }
}

impl Mail {
    pub async fn canonical(&self, ids: &[String]) -> Result<Vec<Current>> {
        let mut found = BTreeSet::new();
        for chunk in ids.chunks(PAGE) {
            found.extend(self.query(json!({ "id": chunk })).await?);
        }
        found.iter().map(|id| Current::new(id)).collect()
    }

    /// Takes up the token other sessions have since signed in for first, so nothing signs in under the lock.
    pub async fn exclusive(&self) -> Result<File> {
        *self.conn.token.borrow_mut() = self.conn.auth.access().await?.token;
        self.conn.auth.store.lock("claims.lock").await
    }

    pub async fn snapshot(&self, ids: &[Current]) -> Result<Vec<Email>> {
        Ok(self
            .got(true, json!(&ids[..ids.len().min(self.limits.get).min(self.limits.set)]))
            .await?
            .list)
    }

    pub fn calls_per_request(&self) -> usize {
        self.limits.calls
    }

    pub async fn each(&self, locked: bool, marks: &[Mark]) -> Result<Vec<Result<bool>>> {
        let set = |mark: &Mark| (SET, json!({ "update": patches(std::slice::from_ref(mark)) }));
        let answers = self.answers(locked, &marks.iter().map(set).collect::<Vec<_>>()).await?;
        let wrote = |(answer, (id, _)): (Result<Option<Set>>, &Mark)| {
            let wrote = |a: Set| a.updated.contains_key(id.as_str()) && a.new_state != a.old_state;
            Ok(answer?.is_some_and(wrote))
        };
        Ok(answers.into_iter().zip(marks).map(wrote).collect())
    }
}

fn patches(marks: &[Mark]) -> Map<String, Value> {
    let patch = |set: &[Keyword]| {
        set.iter().map(|k| (format!("keywords/{}", k.name()), json!(true))).collect()
    };
    marks.iter().map(|(id, set)| (id.0.clone(), Value::Object(patch(set)))).collect()
}

/// Server-sent events as they arrive; only a state change wakes, since Stalwart's pings carry data too.
#[derive(Default)]
pub struct Frames(Events);

impl Frames {
    pub fn wake(&mut self, chunk: &[u8]) -> bool {
        let wakes = |e: &String| !matches!(Event::read(e), None | Some(Event::Ping));
        self.0.push(chunk).iter().any(wakes)
    }
}

pub struct Pushes(Response, Frames);

impl Pushes {
    pub async fn next(&mut self) -> Result<()> {
        loop {
            let failed = |e| Failure::Dropped.with(format!("the push stream failed: {e}"));
            let silent = format!("the push stream was silent for {}s", SILENCE.as_secs());
            let chunk = tokio::time::timeout(SILENCE, self.0.chunk()).await;
            match chunk.map_err(|_| Failure::Dropped.with(silent))?.map_err(failed)? {
                Some(chunk) if self.1.wake(&chunk) => return Ok(()),
                Some(_) => {}
                None => return Err(Failure::Dropped.with("the push stream closed")),
            }
        }
    }
}

use std::{cell::RefCell, fs::File};

use private::{Secret, random::below};
use reqwest::{Method, Response, header};

use crate::{
    auth::Auth,
    jmap::{Call, View, jmap_id, keywords, wire},
    policy::Login,
    prelude::*,
    store::Config,
    web::{self, Origin, Pinned, TIMEOUT, Web},
};

pub const MAX_BODY_BYTES: usize = 64 * 1024;
const PAGE: usize = 256;
const RESTARTS: usize = 10;

wire! {
    views {
        Email in "Email/get" {
            id: String = "id", thread: String = "threadId", received: String = "receivedAt",
            from: Vec<Addr> = "from", to: Vec<Addr> = "to", cc: Vec<Addr> = "cc",
            subject: String = "subject", keywords: HashMap<String, bool> = "keywords",
            mailboxes: HashMap<String, bool> = "mailboxIds", message_id: Vec<String> = "messageId",
            in_reply_to: Vec<String> = "inReplyTo", references: Vec<String> = "references",
            preview: String = "preview", content_type: String = "header:Content-Type",
            notify: Vec<Addr> = "header:Disposition-Notification-To:asAddresses",
            original_recipient: String = "header:Original-Recipient:asText",
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
    objects {
        Addr { email: String = "email" }
        Part {
            blob: String = "blobId", size: usize = "size", name: String = "name",
            kind: String = "type", part: String = "partId", parts: Vec<Part> = "subParts",
        }
        Text { value: String = "value", truncated: bool = "isTruncated" }
        Status { reply: String = "smtpReply", delivered: String = "delivered" }
        Session {
            api: String = "apiUrl", download: String = "downloadUrl", upload: String = "uploadUrl",
            events: String = "eventSourceUrl", username: String = "username",
            accounts: HashMap<String, String> = "primaryAccounts",
            capabilities: HashMap<String, Value> = "capabilities",
        }
        Answer {
            ids: Vec<String> = "ids", list: Vec<Value> = "list", more: bool = "hasMoreChanges",
            state: String = "state", old_state: String = "oldState", new_state: String = "newState",
            created: Value = "created", not_created: Map<String, Value> = "notCreated",
            updated: Value = "updated", not_updated: Map<String, Value> = "notUpdated",
            not_destroyed: Map<String, Value> = "notDestroyed",
        }
    }
}

pub const QUERY: Call = Call("Email/query", Some("anchorNotFound"));
pub const CHANGES: Call = Call("Email/changes", Some("cannotCalculateChanges"));
pub const SET: Call = Call("Email/set", Some("stateMismatch"));
pub const IMPORT: Call = Call("Email/import", None);
pub const SUBMIT: Call = Call("EmailSubmission/set", None);
pub const SUBMISSIONS: Call = Call("EmailSubmission/query", None);

// `$agentmail` marks the Sent copy of what agent-mail sends, so answering from Sent, meant for other tools, skips it.
keywords! {
    Seen = "$seen", Answered = "$answered", Handled = "$handled", MdnSent = "$mdnsent",
    AgentMail = "$agentmail", Draft = "$draft",
}

impl Email {
    pub fn has(&self, keyword: Keyword) -> bool {
        self.keywords.get(keyword.name()) == Some(&true)
    }
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

const PING: Duration = Duration::from_secs(30);
const SILENCE: Duration = Duration::from_secs(PING.as_secs() + 5);

pub fn fill(template: &str, values: &[(&str, &str)]) -> Result<Pinned> {
    let fill =
        |url: String, (name, value): &(&str, &str)| url.replace(&format!("{{{name}}}"), value);
    Origin::Mail.check(&values.iter().fold(template.to_owned(), fill))
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

    /// Runs `op` with the session's token. After a refused token it signs in again and runs it once more, unless
    /// `locked`, when the refusal comes back so the lock can be released first; over Stalwart's limit it backs off.
    pub async fn retry<T>(&self, locked: bool, op: impl AsyncFn(Secret) -> Result<T>) -> Result<T> {
        let (mut signed_in, mut backoff) = (locked, Duration::from_millis(500));
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
                Failure::Busy if backoff <= Duration::from_secs(16) => {
                    tokio::time::sleep(backoff + Duration::from_millis(below(250)? as u64)).await;
                    backoff *= 2;
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
    session: Session,
    api: Pinned,
    account: String,
    limits: [usize; 3],
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
        for url in [&session.upload, &session.download, &session.events] {
            Origin::Mail.check(url).context(off)?;
        }
        let api = Origin::Mail.check(&session.api).context(off)?;
        check_account(&session.username, &conn.auth.login)?;
        let account =
            session.accounts.get("urn:ietf:params:jmap:mail").context("no mail account")?;
        let account = jmap_id(account)?.into();
        let core =
            session.capabilities.get("urn:ietf:params:jmap:core").cloned().unwrap_or_default();
        let limits = ["maxObjectsInGet", "maxObjectsInSet", "maxCallsInRequest"]
            .map(|n| core[n].as_u64().map_or(100, |n| n as usize).max(1));
        let mut mail = Mail { conn, session, api, account, limits, mailboxes: HashMap::new() };
        mail.mailboxes = mail.all::<Mailbox>().await?.into_iter().map(|m| (m.role, m.id)).collect();
        Ok(mail)
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

    pub async fn call(&self, call: Call, args: Value) -> Result<Answer> {
        let name = call.0;
        let answer = self.answer(false, call, args).await?;
        answer.with_context(|| format!("stalwart answered {name} with an error"))
    }

    pub async fn answer(&self, locked: bool, call: Call, args: Value) -> Result<Option<Answer>> {
        self.answers(locked, &[(call, args)]).await?.remove(0)
    }

    async fn answers(
        &self,
        locked: bool,
        calls: &[(Call, Value)],
    ) -> Result<Vec<Result<Option<Answer>>>> {
        let using = ["core", "mail", "submission"].map(|c| format!("urn:ietf:params:jmap:{c}"));
        let method = |(n, (call, args)): (usize, &(Call, Value))| {
            let mut args = args.clone();
            args["accountId"] = json!(self.account);
            json!([call.0, args, n.to_string()])
        };
        let methods: Vec<Value> = calls.iter().enumerate().map(method).collect();
        let body = json!({ "using": using, "methodCalls": methods });
        let response: Value = self
            .conn
            .retry(locked, async |t| {
                let post = |r: reqwest::RequestBuilder| r.timeout(TIMEOUT).json(&body);
                Ok(self.conn.web.fetch(Method::POST, &self.api, &t, post).await?.json().await?)
            })
            .await?;
        let replies = response["methodResponses"].as_array().cloned().unwrap_or_default();
        let read = |(n, (Call(name, tolerated), _)): (usize, &(Call, Value))| {
            let reply =
                replies.get(n).filter(|r| r[2].as_str().and_then(|i| i.parse().ok()) == Some(n));
            match reply.map(|r| (r[0].as_str(), r[1]["type"].as_str(), &r[1])) {
                Some((Some("error"), kind, _)) if kind.is_some() && kind == *tolerated => Ok(None),
                Some((Some("error"), kind, _)) => {
                    bail!("stalwart refused {name}: {}", kind.unwrap_or_default())
                }
                Some((Some(_), _, answer)) => Ok(Some(
                    serde_json::from_value(answer.clone())
                        .with_context(|| format!("stalwart's {name} reply"))?,
                )),
                _ => bail!("stalwart sent no reply to {name}"),
            }
        };
        Ok(calls.iter().enumerate().map(read).collect())
    }

    pub async fn set(&self, locked: bool, args: Value) -> Result<Option<Answer>> {
        let Some(answer) = self.answer(locked, SET, args).await? else { return Ok(None) };
        ensure!(
            answer.not_updated.is_empty() && answer.not_destroyed.is_empty(),
            "stalwart refused {}",
            json!([answer.not_updated, answer.not_destroyed])
        );
        Ok(Some(answer))
    }

    async fn got<V: View>(&self, locked: bool, ids: Value) -> Result<(String, Vec<V>)> {
        let mut args = json!({ "ids": ids, "properties": V::PROPERTIES });
        if let Some((bodies, bytes)) = V::VALUES {
            (args[bodies], args["maxBodyValueBytes"]) = (json!(true), json!(bytes));
        }
        let answer = self.answer(locked, Call(V::GET, None), args).await?.context(V::GET)?;
        let list = serde_json::from_value(Value::Array(answer.list));
        Ok((answer.state, list.with_context(|| format!("stalwart's {}", V::GET))?))
    }

    pub async fn get<V: View>(&self, ids: &[impl Serialize]) -> Result<Vec<V>> {
        let mut found = vec![];
        for chunk in ids.chunks(self.limits[0]) {
            found.extend(self.got(false, json!(chunk)).await?.1);
        }
        Ok(found)
    }

    pub async fn all<V: View>(&self) -> Result<Vec<V>> {
        Ok(self.got(false, Value::Null).await?.1)
    }

    pub async fn state(&self) -> Result<String> {
        Ok(self.got::<Email>(false, json!([])).await?.0)
    }

    /// Every id the filter matches, unsorted, since a sort is answered from the full-text index, which lags ingest.
    pub async fn query(&self, filter: Value) -> Result<Vec<String>> {
        let (mut ids, mut restarts): (Vec<String>, usize) = (vec![], 0);
        loop {
            let at = match ids.last() {
                Some(last) => {
                    json!({ "filter": filter, "limit": PAGE, "anchor": last, "anchorOffset": 1 })
                }
                None => json!({ "filter": filter, "limit": PAGE, "position": 0 }),
            };
            let Some(page) = self.answer(false, QUERY, at).await? else {
                restarts += 1;
                ensure!(restarts < RESTARTS, "the mail kept changing under Email/query's pages");
                ids.clear();
                continue;
            };
            let last = page.ids.len() < PAGE;
            ids.extend(page.ids);
            if last {
                return Ok(ids);
            }
        }
    }

    /// Stalwart refuses an id outside its alphabet in Email/query; Email/get just finds no such email.
    pub async fn current(&self, id: &str) -> Result<Option<Current>> {
        const FIND: Call = Call("Email/query", Some("invalidArguments"));
        if jmap_id(id).is_err() {
            return Ok(None);
        }
        let found = self.answer(false, FIND, json!({ "filter": { "id": [id] } })).await?;
        found.and_then(|f| f.ids.into_iter().next()).map(|id| Current::new(&id)).transpose()
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
            for list in [changed.created, changed.updated] {
                let list: Option<Vec<String>> =
                    serde_json::from_value(list).context("stalwart's Email/changes reply")?;
                ids.extend(list.unwrap_or_default());
            }
            if !changed.more {
                return Ok(Some((ids.into_iter().collect(), changed.new_state)));
            }
            ensure!(changed.new_state != state, "stalwart's Email/changes has more past {state}");
            state = changed.new_state;
        }
    }

    pub async fn mark(&self, marks: &[Mark]) -> Result<()> {
        for chunk in marks.chunks(self.limits[1]) {
            self.set(false, json!({ "update": patches(chunk) })).await?;
        }
        Ok(())
    }

    pub fn created(answer: &Answer, key: &str) -> Result<Current> {
        match answer.created[key]["id"].as_str() {
            Some(id) => Current::new(id),
            None => bail!(
                "stalwart did not create it: {}",
                answer.not_created.get(key).unwrap_or(&Value::Null)
            ),
        }
    }

    pub async fn upload(&self, bytes: &[u8], kind: &str) -> Result<String> {
        let url = fill(&self.session.upload, &[("accountId", &self.account)])?;
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

    pub async fn download(&self, blob: &str, most: usize) -> Result<Option<Vec<u8>>> {
        let ids = [("accountId", jmap_id(&self.account)?), ("blobId", jmap_id(blob)?)];
        let fixed = [("name", "attachment"), ("type", "application%2Foctet-stream")];
        let url = fill(&self.session.download, &[ids, fixed].concat())?;
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
        let (url, token) = (fill(&self.session.events, &values)?, self.conn.token.borrow().clone());
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
            .got(true, json!(&ids[..ids.len().min(self.limits[0]).min(self.limits[1])]))
            .await?
            .1)
    }

    pub fn calls_per_request(&self) -> usize {
        self.limits[2]
    }

    pub async fn each(&self, locked: bool, marks: &[Mark]) -> Result<Vec<Result<bool>>> {
        let set = |mark: &Mark| (SET, json!({ "update": patches(std::slice::from_ref(mark)) }));
        let answers = self.answers(locked, &marks.iter().map(set).collect::<Vec<_>>()).await?;
        let wrote = |(answer, (id, _)): (Result<Option<Answer>>, &Mark)| {
            let answer = answer?;
            Ok(answer.is_some_and(|a| {
                a.updated.get(id.as_str()).is_some() && a.new_state != a.old_state
            }))
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
pub struct Frames(Vec<u8>);

impl Frames {
    pub fn wake(&mut self, chunk: &[u8]) -> bool {
        self.0.extend(chunk.iter().filter(|&&b| b != b'\r'));
        let mut woke = false;
        while let Some(end) = self.0.windows(2).position(|w| w == b"\n\n") {
            let event: Vec<u8> = self.0.drain(..end + 2).collect();
            let lines = || event.split(|&b| b == b'\n');
            let ping =
                |l: &[u8]| l.strip_prefix(b"event:").is_some_and(|v| v.trim_ascii() == b"ping");
            woke |= !lines().any(ping) && lines().any(|l| l.starts_with(b"data:"));
        }
        woke
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

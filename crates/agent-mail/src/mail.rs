//! A JMAP session with Stalwart that signs in again once when Stalwart refuses the token.

use std::{cell::RefCell, collections::BTreeSet, rc::Rc, time::Duration};

use anyhow::{Context, Result, anyhow, ensure};
use futures_util::Stream;
use jmap_client::{
    DataType, URI,
    client::{Client, Credentials},
    core::{
        error::{JMAPError, MethodErrorType, ProblemType},
        query::Filter,
        request::Request,
        response::{MailboxGetResponse, TaggedMethodResponse, ThreadGetResponse},
    },
    email::{self, Email, query},
    event_source::PushNotification,
    mailbox::{self, Role},
    thread,
};

use crate::{
    auth::Auth,
    config::ConfigError,
    identity::Login,
    origin::{self, Origin, Pinned},
    policy::JMAP_HOST,
    random::Draws,
    secret::Secret,
};

const THROTTLE_BACKOFF: Duration = Duration::from_millis(500);
const THROTTLE_GIVE_UP: Duration = Duration::from_secs(16);
const QUERY_PAGE: usize = 256;
const CHANGES_PAGE: usize = 256;
const PUSH_HEADERS_TIMEOUT: Duration = Duration::from_secs(30);

pub const SEEN: &str = "$seen";
pub const ANSWERED: &str = "$answered";
pub const HANDLED: &str = "$handled";
/// RFC 3503's `$MDNSent`; servers return keywords in lowercase.
pub const MDN_SENT: &str = "$mdnsent";
/// On the Sent copy of everything agent-mail sends, so the Sent fallback, meant for other tools' replies, skips it.
pub const AGENT_MAIL: &str = "$agentmail";

pub fn has_keyword(m: &Email, keyword: &str) -> bool {
    m.keywords().contains(&keyword)
}

pub struct Session {
    pub client: Client,
    token: Secret,
    downloads: reqwest::Client,
    mailboxes: Vec<(Role, String)>,
}

impl Session {
    pub fn request(&self) -> Request<'_> {
        let mut request = self.client.build();
        request.using = vec![URI::Core, URI::Mail, URI::Submission];
        request
    }

    /// RFC 8620 lets a server refuse an Email/get for more ids than this.
    pub fn max_objects_in_get(&self) -> usize {
        self.client
            .session()
            .core_capabilities()
            .map_or(100, |core| core.max_objects_in_get())
            .max(1)
    }

    pub fn max_objects_in_set(&self) -> usize {
        self.client
            .session()
            .core_capabilities()
            .map_or(100, |core| core.max_objects_in_set())
            .max(1)
    }

    pub fn mailbox(&self, role: Role) -> Result<&str> {
        self.mailboxes
            .iter()
            .find(|(r, _)| *r == role)
            .map(|(_, id)| id.as_str())
            .ok_or_else(|| anyhow!("no {role:?} mailbox"))
    }

    /// jmap-client bounds only the connect, so the wait for the response headers is bounded here.
    pub async fn pushes(
        &self,
    ) -> Result<impl Stream<Item = jmap_client::Result<PushNotification>> + Unpin + '_> {
        let opening = self
            .client
            .event_source(Some([DataType::Email]), false, Some(30), None);
        tokio::time::timeout(PUSH_HEADERS_TIMEOUT, opening)
            .await
            .context("the push stream sent no response headers in time")?
            .context("stalwart")
    }

    async fn download(&self, blob: &str, bounds: Bounds) -> jmap_client::Result<Option<Vec<u8>>> {
        let url = download_url(
            self.client.session().download_url(),
            self.client.default_account_id(),
            blob,
        )
        .map_err(|e| jmap_client::Error::Internal(format!("{e:#}")))?;
        let response = self
            .downloads
            .get(url.as_str())
            .bearer_auth(self.token.expose())
            .timeout(bounds.time)
            .send()
            .await?;
        let mut response = Client::handle_error(response).await?;
        let mut bytes = vec![];
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > bounds.bytes {
                return Ok(None);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(Some(bytes))
    }

    /// The session names the URLs every later request goes to, so they are checked before any of them is used, and
    /// from then on no request follows a redirect. The account must be the configured login's, since the
    /// recipient policy is keyed on that login.
    async fn open(token: Secret, login: &Login) -> Result<Session> {
        let mut client = Client::new()
            .credentials(Credentials::bearer(token.expose()))
            .follow_redirects([JMAP_HOST])
            .connect(&Origin::Mail.base())
            .await?;
        let named = client.session();
        for url in [
            named.api_url(),
            named.upload_url(),
            named.download_url(),
            named.event_source_url(),
        ] {
            Origin::Mail
                .check(url)
                .context("the JMAP session names a URL off the mail origin")?;
        }
        check_account(named.username(), login)?;
        let mail_account = named
            .primary_accounts()
            .find(|(uri, _)| uri.as_str() == "urn:ietf:params:jmap:mail")
            .map(|(_, id)| id.clone());
        if let Some(account) = mail_account {
            client.set_default_account_id(account);
        }
        client.set_follow_redirects(Vec::<String>::new());
        let mut session = Session {
            client,
            token,
            downloads: origin::download_client()?,
            mailboxes: vec![],
        };
        let mut request = session.request();
        request
            .get_mailbox()
            .properties([mailbox::Property::Id, mailbox::Property::Role]);
        let mailboxes = request
            .send_single::<MailboxGetResponse>()
            .await?
            .take_list();
        session.mailboxes = mailboxes
            .into_iter()
            .filter_map(|m| Some((m.role(), m.id()?.to_owned())))
            .collect();
        Ok(session)
    }
}

/// An RFC 8620 id: 1 to 255 of `[A-Za-z0-9_-]`, so safe unescaped in a URL path and as a file name.
#[derive(Clone, Copy)]
pub struct JmapId<'a>(&'a str);

impl<'a> JmapId<'a> {
    pub fn new(id: &'a str) -> Result<JmapId<'a>> {
        let valid = (1..=255).contains(&id.len())
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        ensure!(valid, "{} is not a JMAP id", id.escape_debug());
        Ok(JmapId(id))
    }

    pub fn as_str(self) -> &'a str {
        self.0
    }
}

/// How much of a download is taken, and how long it may take in all; each read is bounded besides.
#[derive(Clone, Copy)]
pub struct Bounds {
    pub bytes: usize,
    pub time: Duration,
}

fn download_url(template: &str, account: &str, blob: &str) -> Result<Pinned> {
    let url = template
        .replace("{accountId}", JmapId::new(account)?.as_str())
        .replace("{blobId}", JmapId::new(blob)?.as_str())
        .replace("{name}", "attachment")
        .replace("{type}", "application%2Foctet-stream");
    Origin::Mail.check(&url)
}

/// The recipient policy is keyed on the configured login, so the server must agree that is whose account this is.
fn check_account(username: &str, login: &Login) -> Result<()> {
    if !username.eq_ignore_ascii_case(&login.to_string()) {
        return Err(ConfigError::new(format!(
            "the token is for {}, not the configured {login}",
            username.escape_debug()
        ))
        .into());
    }
    Ok(())
}

pub struct Mail {
    pub auth: Auth,
    session: RefCell<Rc<Session>>,
}

/// A request-level `limit` error means no call in the request ran (RFC 8620 §3.6.1), so a retry is safe; only the
/// concurrency limit can clear by waiting.
fn throttled(e: &jmap_client::Error) -> bool {
    let jmap_client::Error::Problem(problem) = e else {
        return false;
    };
    matches!(problem.error(), ProblemType::JMAP(JMAPError::Limit))
        && problem.limit() == Some("maxConcurrentRequests")
}

/// Stalwart refused the token in a call made under a lock, which must be released before signing in again.
#[derive(Debug)]
pub struct NeedsSignIn;

impl std::fmt::Display for NeedsSignIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("stalwart refused the token while a lock was held")
    }
}

/// Whether `with` gave up waiting out the request limit.
pub fn busy(e: &anyhow::Error) -> bool {
    e.downcast_ref::<jmap_client::Error>()
        .is_some_and(throttled)
}

/// Whether Stalwart refused the access token, which is what an expired one looks like.
pub fn refused_token(e: &anyhow::Error) -> bool {
    e.downcast_ref::<jmap_client::Error>()
        .is_some_and(unauthorized)
}

fn unauthorized(e: &jmap_client::Error) -> bool {
    match e {
        jmap_client::Error::Problem(problem) => problem.status() == Some(401),
        jmap_client::Error::Server(status) => status.starts_with("401"),
        _ => false,
    }
}

impl Mail {
    pub async fn connect(auth: Auth) -> Result<Mail> {
        let session = Self::sign_in(&auth, None).await?;
        Ok(Mail {
            auth,
            session: RefCell::new(Rc::new(session)),
        })
    }

    async fn sign_in(auth: &Auth, mut refused: Option<Secret>) -> Result<Session> {
        loop {
            if let Some(token) = &refused {
                auth.reject(token).await?;
            }
            let token = auth.access().await?.token;
            match Session::open(token.clone(), &auth.login).await {
                Err(e) if e.downcast_ref().is_some_and(unauthorized) && refused.is_none() => {
                    refused = Some(token)
                }
                opened => return opened.context("stalwart"),
            }
        }
    }

    pub fn session(&self) -> Rc<Session> {
        self.session.borrow().clone()
    }

    /// Signs in again now if the cached token has moved on from the one this session was opened with.
    pub async fn ensure_fresh(&self) -> Result<()> {
        let token = self.auth.access().await?.token;
        if token != self.session().token {
            self.reconnect().await?;
        }
        Ok(())
    }

    pub async fn reconnect(&self) -> Result<()> {
        let session = Self::sign_in(&self.auth, None).await?;
        *self.session.borrow_mut() = Rc::new(session);
        Ok(())
    }

    /// Runs `op` on the session. After a 401 it signs in again and runs `op` once more; while Stalwart is over its
    /// request limit it backs off and retries.
    pub async fn with<T>(&self, op: impl AsyncFn(&Session) -> jmap_client::Result<T>) -> Result<T> {
        self.run(op, true).await
    }

    async fn run<T>(
        &self,
        op: impl AsyncFn(&Session) -> jmap_client::Result<T>,
        may_sign_in: bool,
    ) -> Result<T> {
        let mut signed_in_again = false;
        let mut backoff = THROTTLE_BACKOFF;
        loop {
            let session = self.session();
            match op(&session).await {
                Err(e) if unauthorized(&e) && !may_sign_in => {
                    return Err(anyhow::Error::new(e).context(NeedsSignIn));
                }
                Err(e) if unauthorized(&e) && !signed_in_again => {
                    let fresh = Self::sign_in(&self.auth, Some(session.token.clone())).await?;
                    *self.session.borrow_mut() = Rc::new(fresh);
                    signed_in_again = true;
                }
                Err(e) if throttled(&e) && backoff <= THROTTLE_GIVE_UP => {
                    let jitter = Duration::from_millis(Draws::default().below(250)? as u64);
                    tokio::time::sleep(backoff + jitter).await;
                    backoff *= 2;
                }
                done => return done.context("stalwart"),
            }
        }
    }

    pub async fn call(&self, build: impl Fn(&mut Request<'_>)) -> Result<Responses> {
        self.request(build, true).await
    }

    /// Like `call`, but for use under a lock: it never signs in, and a 401 comes back as `NeedsSignIn`.
    pub async fn call_locked(&self, build: impl Fn(&mut Request<'_>)) -> Result<Responses> {
        self.request(build, false).await
    }

    async fn request(
        &self,
        build: impl Fn(&mut Request<'_>),
        may_sign_in: bool,
    ) -> Result<Responses> {
        let send = async |session: &Session| {
            let mut request = session.request();
            build(&mut request);
            request.send().await
        };
        let response = self.run(send, may_sign_in).await?;
        Ok(Responses(response.unwrap_method_responses().into_iter()))
    }

    /// Every id an Email/query matches, page by page. There is no sort: a sort is answered from the full-text
    /// index, which lags ingest, while keyword and mailbox filters are answered from the message cache. Each page
    /// starts after the last id of the one before, so mail that leaves the result meanwhile shifts nothing.
    pub async fn query(&self, filter: impl Fn() -> Filter<query::Filter>) -> Result<Vec<String>> {
        let mut ids: Vec<String> = vec![];
        loop {
            let mut responses = self
                .call(|r| {
                    let q = r.query_email();
                    q.filter(filter()).limit(QUERY_PAGE);
                    match ids.last() {
                        Some(last) => q.anchor(last.clone()).anchor_offset(1),
                        None => q.position(0),
                    };
                })
                .await?;
            let page = match responses.next()?.unwrap_query_email() {
                Ok(mut page) => page.take_ids(),
                Err(jmap_client::Error::Method(e))
                    if e.p_type == MethodErrorType::AnchorNotFound =>
                {
                    ids.clear();
                    continue;
                }
                Err(e) => return Err(e.into()),
            };
            let last = page.len() < QUERY_PAGE;
            ids.extend(page);
            if last {
                return Ok(ids);
            }
        }
    }

    pub async fn emails(
        &self,
        ids: &[String],
        properties: &[email::Property],
        configure: impl Fn(&mut email::GetArguments),
    ) -> Result<Vec<Email>> {
        let mut found = vec![];
        for chunk in ids.chunks(self.session().max_objects_in_get()) {
            let mut responses = self
                .call(|r| {
                    let get = r.get_email();
                    get.ids(chunk.iter().cloned());
                    get.properties(properties.iter().cloned());
                    configure(get.arguments());
                })
                .await?;
            found.extend(responses.emails()?);
        }
        Ok(found)
    }

    pub async fn threads(&self, thread_ids: &[String]) -> Result<Vec<thread::Thread>> {
        let mut found = vec![];
        for chunk in thread_ids.chunks(self.session().max_objects_in_get()) {
            let mut responses = self
                .call(|r| {
                    r.get_thread().ids(chunk.iter().cloned());
                })
                .await?;
            let mut threads: ThreadGetResponse = responses.next()?.unwrap_get_thread()?;
            found.extend(threads.take_list());
        }
        Ok(found)
    }

    pub async fn mark(&self, marks: &[(&str, Vec<&str>)]) -> Result<()> {
        for chunk in marks.chunks(self.session().max_objects_in_set()) {
            let mut responses = self.call(|r| set_keywords(r, None, chunk)).await?;
            responses
                .next()?
                .unwrap_set_email()?
                .unwrap_update_errors()?;
        }
        Ok(())
    }

    /// Adds keywords only if the account's Email state is still `state`; `false` means another change came first.
    /// For use under a lock, so it never signs in: a 401 comes back as `NeedsSignIn`.
    pub async fn mark_if(&self, state: &str, marks: &[(&str, Vec<&str>)]) -> Result<bool> {
        if marks.is_empty() {
            return Ok(true);
        }
        ensure!(
            marks.len() <= self.session().max_objects_in_set(),
            "too many marks for one Email/set"
        );
        let mut responses = self
            .call_locked(|r| set_keywords(r, Some(state), marks))
            .await?;
        match responses.next()?.unwrap_set_email() {
            Err(jmap_client::Error::Method(e)) if e.p_type == MethodErrorType::StateMismatch => {
                Ok(false)
            }
            set => {
                set?.unwrap_update_errors()?;
                Ok(true)
            }
        }
    }

    /// The blob's bytes, or `None` once they pass the byte bound. A download sets no keyword.
    pub async fn download(&self, blob: &str, bounds: Bounds) -> Result<Option<Vec<u8>>> {
        self.with(async |s| s.download(blob, bounds).await).await
    }

    /// The id `id` has now. Stalwart puts the thread in an email's id and answers Email/get with the thread of the
    /// id it was asked for, so after a merge only a query returns the current one. `None` when the email is gone.
    pub async fn current(&self, id: &str) -> Result<Option<String>> {
        let mut responses = self
            .call(|r| {
                r.query_email().filter(query::Filter::id([id]));
            })
            .await?;
        Ok(responses
            .next()?
            .unwrap_query_email()?
            .take_ids()
            .into_iter()
            .next())
    }

    /// The current ids of `ids`, each email once; see `current`.
    pub async fn canonical(&self, ids: &[String]) -> Result<Vec<String>> {
        let mut found = BTreeSet::new();
        for chunk in ids.chunks(QUERY_PAGE) {
            found.extend(
                self.query(|| Filter::from(query::Filter::id(chunk.iter().cloned())))
                    .await?,
            );
        }
        Ok(found.into_iter().collect())
    }

    pub async fn email_state(&self) -> Result<String> {
        let mut responses = self
            .call(|r| {
                r.get_email().ids(Vec::<String>::new());
            })
            .await?;
        Ok(responses.next()?.unwrap_get_email()?.take_state())
    }

    /// Ids created or updated since `state`, and the state they bring it to; `None` when the server can no longer
    /// say, and a full check is needed.
    pub async fn changes(&self, state: &str) -> Result<Option<(Vec<String>, String)>> {
        let (mut ids, mut state) = (BTreeSet::new(), state.to_owned());
        loop {
            let mut responses = self
                .call(|r| {
                    r.changes_email(state.clone()).max_changes(CHANGES_PAGE);
                })
                .await?;
            let mut changes = match responses.next()?.unwrap_changes_email() {
                Ok(changes) => changes,
                Err(jmap_client::Error::Method(e))
                    if e.p_type == MethodErrorType::CannotCalculateChanges =>
                {
                    return Ok(None);
                }
                Err(e) => return Err(e.into()),
            };
            ids.extend(changes.take_created());
            ids.extend(changes.take_updated());
            state = changes.new_state().to_owned();
            if !changes.has_more_changes() {
                return Ok(Some((ids.into_iter().collect(), state)));
            }
        }
    }

    pub async fn import(
        &self,
        message: Vec<u8>,
        mailbox: &str,
        keywords: &[&str],
    ) -> Result<String> {
        let blob = self
            .with(async |s| {
                s.client
                    .upload(None, message.clone(), Some("message/rfc822"))
                    .await
            })
            .await?;
        let mut responses = self
            .call(|r| {
                r.import_email()
                    .email(blob.blob_id())
                    .mailbox_ids([mailbox])
                    .keywords(keywords.iter().copied());
            })
            .await?;
        let imported = responses.next()?.unwrap_import_email()?.created("i0")?;
        Ok(imported.id().unwrap_or_default().to_owned())
    }

    /// Destroys an email agent-mail created itself and failed to send, so it is neither left as a draft nor
    /// mistaken for a sent answer.
    pub async fn discard_unsent(&self, id: &str) -> Result<()> {
        let mut responses = self
            .call(|r| {
                r.set_email().destroy([id]);
            })
            .await?;
        responses.next()?.unwrap_set_email()?.destroyed(id)?;
        Ok(())
    }
}

fn set_keywords(r: &mut Request<'_>, state: Option<&str>, marks: &[(&str, Vec<&str>)]) {
    let set = r.set_email();
    if let Some(state) = state {
        set.if_in_state(state);
    }
    for (id, keywords) in marks {
        let email = set.update(*id);
        for keyword in keywords {
            email.keyword(keyword, true);
        }
    }
}

/// Method responses in call order; implicit responses (`onSuccessUpdateEmail`) follow their call.
pub struct Responses(std::vec::IntoIter<TaggedMethodResponse>);

impl Responses {
    pub fn next(&mut self) -> Result<TaggedMethodResponse> {
        self.0
            .next()
            .context("stalwart sent fewer method responses than calls")
    }

    pub fn emails(&mut self) -> Result<Vec<Email>> {
        Ok(self.next()?.unwrap_get_email()?.take_list())
    }
}

#[cfg(test)]
impl Responses {
    pub fn from_vec(responses: Vec<TaggedMethodResponse>) -> Self {
        Responses(responses.into_iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::is_permanent;

    #[test]
    fn a_refused_token_is_recognised_through_context() {
        let refused = anyhow::Error::from(jmap_client::Error::Server("401 Unauthorized".into()));
        assert!(refused_token(&refused.context("stalwart")));
        let other = anyhow::Error::from(jmap_client::Error::Server("500 Internal".into()));
        assert!(!refused_token(&other.context("stalwart")));
    }

    #[test]
    fn the_session_must_belong_to_the_configured_login() {
        let login: Login = "agent@example.org".parse().unwrap();
        assert!(check_account("Agent@Example.org", &login).is_ok());
        let other = check_account("ada@agent.example.org", &login)
            .err()
            .unwrap();
        assert!(is_permanent(&other), "{other:#}");
    }

    #[test]
    fn downloads_stay_on_the_mail_origin() {
        let template =
            "https://mail.example.org/jmap/download/{accountId}/{blobId}/{name}?accept={type}";
        assert_eq!(
            download_url(template, "b", "Ab_9-z").unwrap().as_str(),
            "https://mail.example.org/jmap/download/b/Ab_9-z/attachment?accept=application%2Foctet-stream"
        );
        for blob in ["", "../x", "a/b", "a?b", "a%2Fb", "@evil.com"] {
            assert!(download_url(template, "b", blob).is_err(), "{blob}");
        }
        assert!(download_url("https://{blobId}/x", "b", "evilcom").is_err());
    }
}

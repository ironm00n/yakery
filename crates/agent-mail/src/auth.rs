use lol_html::{RewriteStrSettings, element, rewrite_str, text};
use oidc::{Access, Device, Failed, Grant, OAuthError, Polling};
use private::{Secret, now};
use reqwest::Method;
use tokio::time::{Instant, sleep};

use crate::{
    CLIENT_ID,
    policy::Login,
    prelude::*,
    store::{Config, Held, PasswordFile, Store, Tokens},
    web::{Body, Origin, Page, Pinned, TIMEOUT, Web},
};

pub const SCOPE: &str = "openid email profile offline_access";
pub const ADMIN_SCOPE: &str =
    "openid email profile offline_access urn:zitadel:iam:org:project:id:zitadel:aud";

pub trait Zitadel {
    /// Leases `held` for each of its steps.
    async fn device_flow(
        &self,
        login: &Login,
        password: &Secret,
        scope: &str,
        held: &Held,
    ) -> Result<Tokens>;
    /// `None` when ZITADEL refuses the refresh token, so that only a sign-in gets tokens; an error
    /// when it gives no OAuth answer.
    async fn refresh(&self, refresh: &Secret) -> Result<Option<Tokens>>;
    async fn change_password(&self, token: &Secret, old: &Secret, new: &Secret) -> Result<()>;
}

pub struct Auth<Z = Http> {
    pub zitadel: Z,
    pub store: Store,
    pub login: Login,
    pub password: PasswordFile,
}

impl Auth {
    pub fn new(config: Config) -> Result<Auth> {
        let (zitadel, store) = (Http(Web::new(false)?), Store::new(&config.login)?);
        Ok(Auth { zitadel, store, login: config.login, password: config.password })
    }
}

fn refused<T>(r: &Result<T>) -> bool {
    r.as_ref().is_err_and(|e| Failure::of(e) == Failure::PasswordRefused)
}

fn refusal() -> anyhow::Error {
    Failure::PasswordRefused.with("zitadel refused the password")
}

/// ZITADEL's error IDs for a password check that ran and failed, so counted toward a lockout.
const FAILED_CHECK: [&str; 3] = ["COMMAND-3M0fs", "COMMAND-CahN2", "EVENT-SDe2f"];

/// Where the forms ZITADEL answers a failed password check with post: its password page, or its
/// login name page when it hides which login names exist.
const CHECKING: [&str; 2] = ["/ui/login/password", "/ui/login/loginname"];

/// The device action ZITADEL's consent page approves with, which its done page's path ends in.
const ALLOWED: &str = "allowed";

/// Where the walk through the login pages is with the password.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Password {
    Unposted,
    /// The page in hand answers the password's post, so it is what ZITADEL says of the password.
    Answered,
    /// The walk moved past the page that answered it, so ZITADEL took it.
    Taken,
}

impl Password {
    fn after(self, post: &Post) -> Password {
        match (self, post.asks_password) {
            (_, true) => Password::Answered,
            (Password::Answered, false) => Password::Taken,
            (walked, false) => walked,
        }
    }

    /// A failure on the page answering the password, which says nothing else of itself, taken as
    /// its refusal: only a refusal stops the password being posted again.
    fn refusing(self, e: anyhow::Error) -> anyhow::Error {
        match self == Password::Answered && Failure::tag(&e).is_none() {
            true => Failure::PasswordRefused.with(format!("{e:#}; taken as a refusal")),
            false => e,
        }
    }
}

/// What ZITADEL says of the password with a form it answers a failed check with, by its error's ID.
fn checked(error: Option<&Shown>, at: &str) -> anyhow::Error {
    let Some(id) = error.map(|e| e.id.as_str()).filter(|id| !id.is_empty()) else {
        return anyhow!("zitadel answered the password at {at} with no error ID");
    };
    let shown = format!("error {id:?} at {at}");
    match FAILED_CHECK.contains(&id) {
        true => Failure::PasswordRefused.with(format!("zitadel failed the password: {shown}")),
        false => Failure::Transient.with(format!(
            "zitadel answered the password with {shown}, which no failed check gives"
        )),
    }
}

/// A login step ZITADEL wants past the password that this walk cannot take.
fn unfinished(step: String) -> anyhow::Error {
    let person = "a person has to finish that step in a browser";
    Failure::Unfinished.with(format!("{step}, past the password; {person}"))
}

/// The post answering `page`, or none when it has no form. Once the password is posted, `page` is
/// judged by which of ZITADEL's handlers renders it.
fn answer(page: &Page, answers: &[(&str, &str)], password: Password) -> Result<Option<Post>> {
    let at = page.url.path();
    let read = || {
        let form = Form::read(&page.text)?;
        let step = match form.post(answers, &page.url)? {
            Step::Post(Post { ref url, .. }) | Step::Asks(_, ref url)
                if password == Password::Answered && CHECKING.contains(&url.path()) =>
            {
                return Err(checked(form.error.as_ref(), at));
            }
            Step::Post(post) if password == Password::Unposted || !post.asks_password => {
                return Ok(Some(post));
            }
            Step::Post(_) => bail!("zitadel asked for the password again without saying why"),
            Step::Asks(asked, to) => {
                format!("zitadel asks for {} at {}", asked.join(", "), to.path())
            }
            Step::Ends => match form.error {
                Some(shown) => format!("zitadel shows {:?} at {at}", shown.text),
                None => return Ok(None),
            },
        };
        Err(match password {
            Password::Unposted => anyhow!(step),
            Password::Answered | Password::Taken => unfinished(step),
        })
    };
    read().map_err(|e| password.refusing(e))
}

/// `page`, unless it failed: ZITADEL's login renders what it says of a password as 200 pages, so
/// a 4xx or 5xx says nothing of it.
fn served(page: Page) -> Result<Page> {
    match page.status.is_client_error() || page.status.is_server_error() {
        true => Err(Failure::Transient.with(format!("{}: HTTP {}", page.url.path(), page.status))),
        false => Ok(page),
    }
}

fn access(tokens: &Tokens) -> Result<Access> {
    tokens.access.clone().context("zitadel granted no access token")
}

impl<Z: Zitadel> Auth<Z> {
    pub async fn access(&self) -> Result<Access> {
        let held = self.store.held().await?;
        let Tokens { access: cached, refresh } = self.store.tokens(&held)?;
        if let Some(cached) = cached.filter(|a| a.fresh(now(), TIMEOUT)) {
            return Ok(cached);
        }
        let refreshed = match refresh {
            Some(old) => held
                .within(TIMEOUT, self.zitadel.refresh(&old))
                .await?
                .map(|t| Tokens { refresh: t.refresh.or(Some(old)), ..t }),
            None => None,
        };
        let tokens = match refreshed {
            Some(tokens) => tokens,
            None => self.sign_in(SCOPE, &held).await?.0,
        };
        let granted = access(&tokens)?;
        self.store.save(tokens, &held)?;
        Ok(granted)
    }

    pub async fn reject(&self, token: &Secret) -> Result<()> {
        let held = self.store.held().await?;
        let mut stored = self.store.tokens(&held)?;
        if stored.access.as_ref().is_some_and(|a| a.token == *token) {
            stored.access = None;
            self.store.save(stored, &held)?;
        }
        Ok(())
    }

    async fn post(&self, staged: bool, scope: &str, held: &Held) -> Result<(Tokens, Secret)> {
        let (file, password) = (self.password.fingerprint(staged)?, self.password.read(staged)?);
        if self.store.refused(&file, held)? {
            let (path, login) = (file.path.display(), &self.login);
            return Err(refusal().context(format!(
                "the password in {path} was refused for {login} before; not posting it again until the file changes"
            )));
        }
        let posted = self.zitadel.device_flow(&self.login, &password, scope, held).await;
        if refused(&posted) {
            self.store.refuse(&file, held)?;
        }
        Ok((posted?, password))
    }

    async fn sign_in(&self, scope: &str, held: &Held) -> Result<(Tokens, Secret)> {
        let posted = self.post(false, scope, held).await;
        if !refused(&posted) || self.password.staged()?.is_none() {
            return posted;
        }
        let (tokens, staged) = self.post(true, scope, held).await?;
        self.password.promote(&staged, held)?;
        Ok((tokens, staged))
    }

    pub async fn rotate_password(&self) -> Result<()> {
        let held = self.store.held().await?;
        let (admin, old) = self.sign_in(ADMIN_SCOPE, &held).await?;
        let new = Secret::password(40)?;
        self.password.stage(&new, &held)?;
        let token = access(&admin)?.token;
        let put = self.zitadel.change_password(&token, &old, &new);
        if let Err(put) = held.within(TIMEOUT, put).await {
            return self.settle(put, &held).await;
        }
        self.password.promote(&new, &held)?;
        let signed_in = self.post(false, SCOPE, &held).await;
        let (tokens, _) =
            signed_in.context("the password changed, but signing in with it failed")?;
        self.store.save(tokens, &held)
    }

    /// The change may have applied with only its reply lost: whichever password signs in decides which file stays.
    async fn settle(&self, put: anyhow::Error, held: &Held) -> Result<()> {
        let kept = self.password.describe();
        let with_new = self.post(true, SCOPE, held).await;
        let with_old =
            if refused(&with_new) { Some(self.post(false, SCOPE, held).await) } else { None };
        match (with_new, with_old) {
            (Ok((tokens, new)), _) => {
                self.password.promote(&new, held)?;
                warn!("changing the password reported {put:#}, but the new password works");
                self.store.save(tokens, held)
            }
            (_, Some(Ok((tokens, _)))) => {
                self.password.discard(held)?;
                self.store.save(tokens, held)?;
                Err(put.context("the password is unchanged"))
            }
            (_, Some(old)) if refused(&old) => bail!(
                "changing the password failed ({put:#}) and neither password signs in; kept both {kept}"
            ),
            _ => bail!(
                "changing the password failed ({put:#}) and checking which password works failed too; kept both {kept}"
            ),
        }
    }
}

pub struct Http(pub Web);

const TOKEN: &str = "/oauth/v2/token";

fn client() -> oidc::Client {
    oidc::Client { id: CLIENT_ID.into(), auth: oidc::Auth::Public }
}

fn token() -> http::Uri {
    Origin::Auth.url(TOKEN).as_str().parse().expect("a pinned URL is a URI")
}

/// A reply that does not parse is quoted only on failure statuses, since success bodies carry codes and tokens.
fn unread(reply: &Page, failed: Failed) -> anyhow::Error {
    let (path, status) = (reply.url.path(), reply.status);
    match status.is_success() {
        true => anyhow!("{path}: HTTP {status}: {failed}"),
        false => anyhow!("{path}: HTTP {status}: {}", reply.text),
    }
}

impl Http {
    async fn post(&self, path: &str, form: Vec<(&'static str, String)>) -> Result<Page> {
        let form: Vec<(String, String)> = form.into_iter().map(|(k, v)| (k.into(), v)).collect();
        self.0.send(Method::POST, Origin::Auth.url(path), Body::Form(&form), None).await
    }

    async fn token(&self, grant: Grant<'_>) -> Result<Result<Tokens, OAuthError>> {
        let sent = now();
        let reply = self.post(TOKEN, grant.form(&client(), None, &token(), sent)?).await?;
        match oidc::Tokens::read(reply.status, reply.text.as_bytes(), sent) {
            Ok(granted) => {
                Ok(Ok(Tokens { access: Some(granted.access), refresh: granted.refresh }))
            }
            Err(Failed::Refused(error)) => Ok(Err(error)),
            Err(failed) => Err(unread(&reply, failed)),
        }
    }
}

impl Http {
    /// Walks ZITADEL's login pages for `device`, then polls for its grant.
    async fn granted(
        &self,
        device: &Device,
        mut polling: Polling,
        login: &Login,
        password: &Secret,
    ) -> Result<Tokens> {
        let login = login.to_string();
        let answers = [
            ("user_code", device.user_code.as_str()),
            ("loginName", &login),
            ("password", password.expose()),
        ];
        let browser = Web::new(true)?;
        let start = Origin::Auth.check(&device.verification_uri)?;
        let mut page = served(browser.send(Method::GET, start, Body::Empty, None).await?)?;
        let mut password = Password::Unposted;
        for _ in 0..8 {
            let Some(post) = answer(&page, &answers, password)? else { break };
            password = password.after(&post);
            let sent = browser.send(Method::POST, post.url, Body::Form(&post.fields), None);
            page = served(sent.await?)?;
            if post.consents {
                break;
            }
        }
        let path = page.url.path();
        if !path.ends_with(ALLOWED) {
            let ended = anyhow!("the login pages ended at {path} without approving the device");
            return Err(password.refusing(ended));
        }
        loop {
            let error = match self.token(Grant::Device(&device.device_code)).await? {
                Ok(tokens) => return Ok(tokens),
                Err(error) => error,
            };
            let ended = |failed| anyhow!("device flow: {failed}");
            sleep(polling.next(error, Instant::now().into_std()).map_err(ended)?).await;
        }
    }
}

impl Zitadel for Http {
    async fn device_flow(
        &self,
        login: &Login,
        password: &Secret,
        scope: &str,
        held: &Held,
    ) -> Result<Tokens> {
        let (asking, asked) = (Device::form(&client(), scope, &token(), now())?, Instant::now());
        let reply =
            held.within(TIMEOUT, self.post("/oauth/v2/device_authorization", asking)).await?;
        let device =
            Device::read(reply.status, reply.text.as_bytes()).map_err(|f| unread(&reply, f))?;
        let polling = device.polling(asked.into_std());
        // The last poll goes out before the code expires, and its answer takes up to a request's time.
        let deadline = polling.expiry().and_then(|e| Instant::from_std(e).checked_add(TIMEOUT));
        let deadline = deadline.context("the device code outlasts the clock")?;
        held.until(deadline, self.granted(&device, polling, login, password)).await
    }

    async fn refresh(&self, refresh: &Secret) -> Result<Option<Tokens>> {
        let refused =
            |e: &OAuthError| note!("zitadel refused the refresh token ({}); signing in", e.code);
        Ok(self.token(Grant::Refresh(refresh)).await?.inspect_err(refused).ok())
    }

    async fn change_password(&self, token: &Secret, old: &Secret, new: &Secret) -> Result<()> {
        let body = json!({ "oldPassword": old.expose(), "newPassword": new.expose() });
        let url = Origin::Auth.url("/auth/v1/users/me/password");
        let reply = self.0.send(Method::PUT, url, Body::Json(&body), Some(token)).await?;
        match reply.status.is_success() {
            true => Ok(()),
            false => bail!("changing the password: HTTP {}: {}", reply.status, reply.text),
        }
    }
}

pub struct Form {
    elements: Vec<[String; 5]>,
    pub error: Option<Shown>,
}

/// The error a page shows.
pub struct Shown {
    /// Empty on an error page, which gives it only in its text.
    pub id: String,
    pub text: String,
}

/// What the walk does with a page's first form.
pub enum Step {
    /// The page has no form.
    Ends,
    Post(Post),
    /// The form asks for these, which the walk has no answers to, at its action.
    Asks(Vec<String>, Pinned),
}

pub struct Post {
    pub url: Pinned,
    pub fields: Vec<(String, String)>,
    pub asks_password: bool,
    pub consents: bool,
}

pub fn set(fields: &mut Vec<(String, String)>, name: &str, value: &str) {
    let first = fields.iter().position(|(n, _)| n == name);
    fields.retain(|(n, _)| n != name);
    fields.insert(first.unwrap_or(fields.len()), (name.into(), value.into()));
}

impl Form {
    pub fn read(html: &str) -> Result<Form> {
        let (mut error, mut text) = (None, String::new());
        let (mut elements, mut forms) = (vec![], 0);
        let collect = element!("form, form input, form button", |el| {
            let attr = |name: &str| el.get_attribute(name).unwrap_or_default();
            let tag = el.tag_name();
            forms += usize::from(tag == "form");
            if forms == 1 {
                let action = attr(if tag == "form" { "action" } else { "formaction" });
                elements.push([tag, attr("name"), attr("value"), attr("type"), action]);
            }
            Ok(())
        });
        let shown = element!("div.lgn-error", |el| {
            error = Some(el.get_attribute("title").unwrap_or_default());
            Ok(())
        });
        let said = text!("div.lgn-error .lgn-error-message", |t| {
            text.push_str(t.as_str());
            Ok(())
        });
        let settings = RewriteStrSettings::new().append_element_content_handler(collect);
        let settings = settings.append_element_content_handler(shown);
        rewrite_str(html, settings.append_element_content_handler(said))?;
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        Ok(Form { elements, error: error.map(|id| Shown { id, text }) })
    }

    pub fn post(&self, answers: &[(&str, &str)], page: &Pinned) -> Result<Step> {
        let Some(([_, _, _, _, action], rest)) = self.elements.split_first() else {
            return Ok(Step::Ends);
        };
        let action = page.join(if action.is_empty() { page.as_str() } else { action })?;
        let inputs = || rest.iter().filter(|e| e[0] == "input" && !e[1].is_empty());
        let mut fields = vec![];
        for [_, name, value, ..] in inputs().filter(|e| e[3] == "hidden") {
            set(&mut fields, name, value);
        }
        let mut unanswered = vec![];
        for name in inputs().filter(|e| e[3] != "hidden").map(|e| e[1].as_str()) {
            match answers.iter().find(|(n, _)| *n == name) {
                Some((_, answer)) => set(&mut fields, name, answer),
                None => unanswered.push(name),
            }
        }
        unanswered.dedup();
        let asks_password = inputs().any(|e| e[3] != "hidden" && e[1] == "password");
        let buttons = || rest.iter().filter(|e| e[0] == "button");
        if let Some([.., allow]) = buttons().find(|b| b[4].ends_with(ALLOWED)) {
            let url = page.join(allow)?;
            return Ok(Step::Post(Post { url, fields, asks_password, consents: true }));
        }
        if !unanswered.is_empty() {
            let Some([_, name, value, ..]) = buttons().find(|b| b[1] == "skip") else {
                return Ok(Step::Asks(unanswered.into_iter().map(String::from).collect(), action));
            };
            set(&mut fields, name, value);
        }
        Ok(Step::Post(Post { url: action, fields, asks_password, consents: false }))
    }
}

use lol_html::{RewriteStrSettings, element, rewrite_str};
use reqwest::Method;

use crate::{
    CLIENT_ID,
    policy::Login,
    prelude::*,
    private::{Secret, now},
    store::{Access, Config, Held, PasswordFile, Store, Tokens},
    web::{Body, Origin, Page, Pinned, Web},
};

pub const SCOPE: &str = "openid email profile offline_access";
pub const ADMIN_SCOPE: &str =
    "openid email profile offline_access urn:zitadel:iam:org:project:id:zitadel:aud";

pub trait Zitadel {
    async fn device_flow(&self, login: &Login, password: &Secret, scope: &str) -> Result<Tokens>;
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

fn access(tokens: &Tokens) -> Result<Access> {
    tokens.access.clone().context("zitadel granted no access token")
}

impl<Z: Zitadel> Auth<Z> {
    pub async fn access(&self) -> Result<Access> {
        let held = self.store.held().await?;
        let Tokens { access: cached, refresh } = self.store.tokens(&held)?;
        if let Some(cached) = cached.filter(|a| a.fresh(now())) {
            return Ok(cached);
        }
        let refreshed = match refresh {
            Some(old) => self
                .zitadel
                .refresh(&old)
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
        if self.store.refused(&file, false, held)? {
            let (path, login) = (file.path.display(), &self.login);
            return Err(refusal().context(format!(
                "the password in {path} was refused for {login} before; not posting it again until the file changes"
            )));
        }
        let posted = self.zitadel.device_flow(&self.login, &password, scope).await;
        if refused(&posted) {
            self.store.refused(&file, true, held)?;
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
        if let Err(put) = self.zitadel.change_password(&access(&admin)?.token, &old, &new).await {
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
                note!("changing the password reported {put:#}, but the new password works");
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

/// A body that does not parse is quoted only on failure statuses, since success bodies carry codes and tokens.
fn json<T: DeserializeOwned>(reply: Page) -> Result<T> {
    serde_json::from_str(&reply.text).map_err(|e| match reply.status.is_success() {
        true => anyhow!("{}: HTTP {}: unreadable body: {e}", reply.url.path(), reply.status),
        false => anyhow!("{}: HTTP {}: {}", reply.url.path(), reply.status, reply.text),
    })
}

impl Http {
    async fn post<T: DeserializeOwned>(&self, path: &str, form: &[(&str, &str)]) -> Result<T> {
        let form: Vec<(String, String)> = form.iter().map(|&(k, v)| (k.into(), v.into())).collect();
        json(self.0.send(Method::POST, Origin::Auth.url(path), Body::Form(&form), None).await?)
    }

    async fn token(&self, form: &[(&str, &str)]) -> Result<Result<Tokens, String>> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Reply {
            Granted { access_token: Secret, refresh_token: Option<Secret>, expires_in: i64 },
            Refused { error: String },
        }
        Ok(match self.post("/oauth/v2/token", form).await? {
            Reply::Granted { access_token: token, refresh_token: refresh, expires_in } => {
                let expires_at = now().checked_add(expires_in).context("token expiry overflows")?;
                Ok(Tokens { access: Some(Access { token, expires_at }), refresh })
            }
            Reply::Refused { error } => Err(error),
        })
    }
}

impl Zitadel for Http {
    async fn device_flow(&self, login: &Login, password: &Secret, scope: &str) -> Result<Tokens> {
        #[derive(Deserialize)]
        struct Device {
            device_code: String,
            user_code: String,
            verification_uri: String,
        }
        let device: Device = self
            .post("/oauth/v2/device_authorization", &[("client_id", CLIENT_ID), ("scope", scope)])
            .await?;
        let login = login.to_string();
        let answers = [
            ("user_code", device.user_code.as_str()),
            ("loginName", &login),
            ("password", password.expose()),
        ];
        let browser = Web::new(true)?;
        let start = Origin::Auth.check(&device.verification_uri)?;
        let mut page = browser.send(Method::GET, start, Body::Empty, None).await?;
        let mut password_posted = false;
        for _ in 0..8 {
            let Some(post) = Form::read(&page.text)?.post(&answers, &page.url)? else { break };
            if post.asks_password && std::mem::replace(&mut password_posted, true) {
                return Err(refusal());
            }
            page = browser.send(Method::POST, post.url, Body::Form(&post.fields), None).await?;
            if post.consents {
                break;
            }
        }
        let grant = [
            ("client_id", CLIENT_ID),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", &device.device_code),
        ];
        for _ in 0..10 {
            match self.token(&grant).await? {
                Ok(tokens) => return Ok(tokens),
                Err(error) if error == "authorization_pending" || error == "slow_down" => {}
                Err(error) => bail!("token: {error}"),
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
        bail!("device flow never authorized; last page {}", page.url.path())
    }

    async fn refresh(&self, refresh: &Secret) -> Result<Option<Tokens>> {
        let form = [
            ("client_id", CLIENT_ID),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.expose()),
        ];
        let refused = |e| note!("zitadel refused the refresh token ({e}); signing in");
        Ok(self.token(&form).await?.map_err(refused).ok())
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

pub struct Form(Vec<[String; 5]>);

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
        let (mut elements, mut forms) = (vec![], 0);
        let collect = element!("form, input, button", |el| {
            let attr = |name: &str| el.get_attribute(name).unwrap_or_default();
            let tag = el.tag_name();
            forms += usize::from(tag == "form");
            if forms == 1 {
                let action = attr(if tag == "form" { "action" } else { "formaction" });
                elements.push([tag, attr("name"), attr("value"), attr("type"), action]);
            }
            Ok(())
        });
        rewrite_str(html, RewriteStrSettings::new().append_element_content_handler(collect))?;
        Ok(Form(elements))
    }

    pub fn post(&self, answers: &[(&str, &str)], page: &Pinned) -> Result<Option<Post>> {
        let Some(([_, _, _, _, action], rest)) = self.0.split_first() else { return Ok(None) };
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
        let asks_password = inputs().any(|e| e[3] != "hidden" && e[1] == "password");
        let buttons = || rest.iter().filter(|e| e[0] == "button");
        if let Some([.., allow]) = buttons().find(|b| b[4].ends_with("allowed")) {
            let url = page.join(allow)?;
            return Ok(Some(Post { url, fields, asks_password, consents: true }));
        }
        if !unanswered.is_empty() {
            let Some([_, name, value, ..]) = buttons().find(|b| b[1] == "skip") else {
                bail!("zitadel asks for {} at {}", unanswered.join(", "), page.path());
            };
            set(&mut fields, name, value);
        }
        Ok(Some(Post { url: action, fields, asks_password, consents: false }))
    }
}

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use jiff::{Timestamp, ToSpan};
use oidc::{Access, Assertion, Auth, Code, Endpoints, Failed, Grant, OAuthError, Tokens};
use private::{Dir, Secret};
use reqwest::{Client, Method, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{Mutex, MutexGuard};

use crate::event::Change;
use crate::local::Owned;
use crate::policy::{Feed, Policy, Route};
use crate::state::{self, SignedIn, Subscription};

const GRAPH: &str = "https://graph.microsoft.com/v1.0";
pub fn scope(policy: &Policy) -> &'static str {
    match policy.write {
        true => "openid offline_access https://graph.microsoft.com/Calendars.ReadWrite",
        false => "openid offline_access https://graph.microsoft.com/Calendars.Read",
    }
}
/// Graph's longest lifetime for an Outlook event subscription is 10,080 minutes; this stays under it.
const LIFETIME: i64 = 10_000;
/// How long a request to Graph or Entra may take, so how long before its expiry a token is let go.
const WITHIN: Duration = Duration::from_secs(30);

pub struct Graph {
    client: Client,
    entra: oidc::Client,
    endpoints: Endpoints,
    policy: &'static Policy,
    state: std::rc::Rc<Dir>,
    access: Mutex<Option<Access>>,
}

pub struct Page {
    pub changes: Vec<Change>,
    pub next: Option<String>,
    pub delta: Option<String>,
}

pub enum Delta {
    Page(Page),
    Expired,
}

pub struct Refused(pub String);

/// A subscription as Graph answers a subscribe or a renewal with it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Made {
    id: String,
    expiration_date_time: Timestamp,
}

/// Refused when Graph won't take the event as it is; a failure a retry may fix is the outer error.
pub type Sent = std::result::Result<(), Refused>;

impl Graph {
    pub fn new(
        policy: &'static Policy,
        assertion: Assertion,
        state: std::rc::Rc<Dir>,
    ) -> Result<Graph> {
        let client = Client::builder().https_only(true).timeout(WITHIN).build()?;
        let entra = oidc::Client {
            id: policy.client_id.clone(),
            auth: Auth::Assertion(Box::new(assertion)),
        };
        let (endpoints, access) = (policy.endpoints()?, Mutex::default());
        Ok(Graph { client, entra, endpoints, policy, state, access })
    }

    /// Takes the cache's guard so that grants run one at a time, and the refresh token saved last
    /// comes from the grant made last. `signed_in` is the scope its sign-in asked for.
    async fn grant(
        &self,
        cache: &mut MutexGuard<'_, Option<Access>>,
        grant: Grant<'_>,
        signed_in: &str,
    ) -> Result<Tokens> {
        let (at, sent) = (&self.endpoints.token, Timestamp::now().as_second());
        let form = grant.form(&self.entra, Some(scope(self.policy)), at, sent)?;
        let reply = self.client.post(at.to_string()).form(&form).send().await.context("entra")?;
        let status = reply.status();
        let tokens = Tokens::read(status, &reply.bytes().await?, sent)
            .map_err(|failed| anyhow!("entra token: HTTP {status}: {}", said(&failed)))?;
        **cache = Some(tokens.access.clone());
        if let Some(refresh) = tokens.refresh.clone() {
            let signed_in = Some(SignedIn { refresh, scope: signed_in.to_owned() });
            self.state.save(state::TOKENS, &state::Tokens { signed_in })?;
        }
        Ok(tokens)
    }

    pub async fn redeem(&self, code: &Code<'_>) -> Result<()> {
        let redirect = self.policy.public.url(Route::Callback);
        let mut cache = self.access.lock().await;
        let code = Grant::Code { code, redirect: &redirect };
        let tokens = self.grant(&mut cache, code, scope(self.policy)).await?;
        anyhow::ensure!(tokens.refresh.is_some(), "entra granted no refresh token");
        Ok(())
    }

    async fn token(&self) -> Result<Secret> {
        let (mut cache, now) = (self.access.lock().await, Timestamp::now().as_second());
        if let Some(access) = cache.as_ref().filter(|a| a.fresh(now, WITHIN)) {
            return Ok(access.token.clone());
        }
        let SignedIn { refresh, scope } = self.signed_in()?.map_err(|why| anyhow!(why))?;
        Ok(self.grant(&mut cache, Grant::Refresh(&refresh), &scope).await?.access.token)
    }

    pub fn signed_in(&self) -> Result<Result<SignedIn, String>> {
        let tokens: state::Tokens = self.state.load(state::TOKENS)?;
        Ok(tokens.signed_in_for(scope(self.policy)))
    }

    async fn call(
        &self,
        method: Method,
        url: &str,
        body: Option<Value>,
    ) -> Result<(StatusCode, String)> {
        for attempt in 0..2 {
            let token = self.token().await?;
            let mut request = self.client.request(method.clone(), url).bearer_auth(token.expose());
            if let Some(body) = &body {
                request = request.json(body);
            }
            let reply = request.send().await.with_context(|| format!("graph {method}"))?;
            let status = reply.status();
            if status == StatusCode::UNAUTHORIZED && attempt == 0 {
                let mut cache = self.access.lock().await;
                if cache.as_ref().is_some_and(|a| a.token == token) {
                    *cache = None;
                }
                continue;
            }
            return Ok((status, reply.text().await?));
        }
        unreachable!("the second attempt returns")
    }

    pub async fn subscribe(&self, client_state: &Secret) -> Result<Subscription> {
        let expires = Timestamp::now() + LIFETIME.minutes();
        let body = json!({
            "changeType": "created,updated,deleted",
            "resource": "/me/events",
            "notificationUrl": self.policy.public.url(Route::Notices(Feed::Changes)),
            "lifecycleNotificationUrl": self.policy.public.url(Route::Notices(Feed::Lifecycle)),
            "expirationDateTime": expires,
            "clientState": client_state.expose(),
        });
        let (status, text) =
            self.call(Method::POST, &format!("{GRAPH}/subscriptions"), Some(body)).await?;
        anyhow::ensure!(
            status == StatusCode::CREATED,
            "graph subscribe: HTTP {status}: {}",
            refusal(&text)
        );
        let made: Made =
            serde_json::from_str(&text).context("graph subscribe: unreadable reply")?;
        Ok(Subscription {
            id: made.id,
            client_state: client_state.clone(),
            expires: made.expiration_date_time,
        })
    }

    /// False when Graph no longer has the subscription.
    pub async fn renew(&self, subscription: &mut Subscription) -> Result<bool> {
        let expires = Timestamp::now() + LIFETIME.minutes();
        let url = format!("{GRAPH}/subscriptions/{}", subscription.id);
        let (status, text) =
            self.call(Method::PATCH, &url, Some(json!({"expirationDateTime": expires}))).await?;
        match status {
            StatusCode::NOT_FOUND => Ok(false),
            s if s.is_success() => {
                let made: Made =
                    serde_json::from_str(&text).context("graph renew: unreadable reply")?;
                subscription.expires = made.expiration_date_time;
                Ok(true)
            }
            s => bail!("graph renew: HTTP {s}: {}", refusal(&text)),
        }
    }

    pub async fn reauthorize(&self, subscription: &Subscription) -> Result<bool> {
        let url = format!("{GRAPH}/subscriptions/{}/reauthorize", subscription.id);
        let (status, text) = self.call(Method::POST, &url, None).await?;
        match status {
            StatusCode::NOT_FOUND => Ok(false),
            s if s.is_success() => Ok(true),
            s => bail!("graph reauthorize: HTTP {s}: {}", refusal(&text)),
        }
    }

    async fn write(&self, method: Method, url: &str, body: Option<Value>) -> Result<Sent> {
        let (status, text) = self.call(method.clone(), url, body).await?;
        match status.as_u16() {
            200..300 => Ok(Ok(())),
            400 | 404 | 409 | 422 => Ok(Err(Refused(format!("HTTP {status}: {}", refusal(&text))))),
            _ => bail!("graph {method}: HTTP {status}: {}", refusal(&text)),
        }
    }

    pub async fn create(&self, event: Owned) -> Result<Sent> {
        self.write(Method::POST, &format!("{GRAPH}/me/events"), Some(event.body())).await
    }

    pub async fn update(&self, id: &str, mut event: Value) -> Result<Sent> {
        // an occurrence's series is Outlook's to change
        event.as_object_mut().map(|e| e.remove("recurrence"));
        self.write(Method::PATCH, &format!("{GRAPH}/me/events/{}", urlencode(id)), Some(event))
            .await
    }

    pub async fn remove(&self, id: &str) -> Result<Sent> {
        self.write(Method::DELETE, &format!("{GRAPH}/me/events/{}", urlencode(id)), None).await
    }

    pub fn listing(from: Timestamp, to: Timestamp) -> String {
        let mut query = form_urlencoded::Serializer::new(String::new());
        query.append_pair("startDateTime", &from.to_string());
        query.append_pair("endDateTime", &to.to_string());
        format!("{GRAPH}/me/calendarView/delta?{}", query.finish())
    }

    pub async fn page(&self, url: &str) -> Result<Delta> {
        anyhow::ensure!(url.starts_with(GRAPH), "refusing a delta link off Graph: {url}");
        let (status, text) = self.call(Method::GET, url, None).await?;
        if status == StatusCode::GONE {
            return Ok(Delta::Expired);
        }
        anyhow::ensure!(status.is_success(), "graph delta: HTTP {status}: {}", refusal(&text));
        #[derive(Deserialize)]
        struct Reply {
            value: Vec<Change>,
            #[serde(rename = "@odata.nextLink")]
            next: Option<String>,
            #[serde(rename = "@odata.deltaLink")]
            delta: Option<String>,
        }
        let reply: Reply = serde_json::from_str(&text).context("graph delta: unreadable page")?;
        Ok(Delta::Page(Page { changes: reply.value, next: reply.next, delta: reply.delta }))
    }
}

/// A Graph id as one path segment: ids hold `/`, `+` and `=`.
fn urlencode(id: &str) -> String {
    form_urlencoded::byte_serialize(id.as_bytes()).collect::<String>().replace('+', "%20")
}

/// A Graph error reply's code and message, never a token-bearing body.
fn refusal(text: &str) -> String {
    let reply: Value = serde_json::from_str(text).unwrap_or_default();
    match reply["error"]["code"].as_str() {
        Some(code) => format!("{code}: {}", reply["error"]["message"].as_str().unwrap_or("")),
        None => "unreadable error".into(),
    }
}

/// A failed token request, with the description Entra's errors carry their AADSTS code in.
fn said(failed: &Failed) -> String {
    match failed {
        Failed::Refused(OAuthError { code, description }) => {
            format!("{code}: {}", description.as_deref().unwrap_or(""))
        }
        other => other.to_string(),
    }
}

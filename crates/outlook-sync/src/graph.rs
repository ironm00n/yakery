//! Entra and Microsoft Graph over HTTPS: tokens from a code or a refresh token, the one
//! subscription on the mailbox's events, and delta pages of the default calendar's occurrences.

use std::cell::RefCell;

use anyhow::{Context, Result, bail};
use jiff::{Timestamp, ToSpan};
use private::{Dir, Secret};
use reqwest::{Client, Method, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::assertion::Credential;
use crate::event::Change;
use crate::policy::Policy;
use crate::state::{self, Subscription, Tokens};

const GRAPH: &str = "https://graph.microsoft.com/v1.0";
/// What the sign-in asks for: writing back needs Calendars.ReadWrite, which an admin of the
/// tenant may have to grant.
pub fn scope(policy: &Policy) -> &'static str {
    match policy.write {
        true => "openid offline_access https://graph.microsoft.com/Calendars.ReadWrite",
        false => "openid offline_access https://graph.microsoft.com/Calendars.Read",
    }
}
/// Graph's longest lifetime for an Outlook event subscription is 10,080 minutes; this stays under it.
const LIFETIME: i64 = 10_000;

pub struct Graph {
    client: Client,
    credential: Credential,
    policy: &'static Policy,
    state: Dir,
    access: RefCell<Option<(Secret, Timestamp)>>,
}

#[derive(Deserialize)]
struct Granted {
    access_token: Secret,
    refresh_token: Option<Secret>,
    expires_in: i64,
}

/// One page of a delta query: its changes, and where the next page or the next run starts.
pub struct Page {
    pub changes: Vec<Change>,
    pub next: Option<String>,
    pub delta: Option<String>,
}

pub enum Delta {
    Page(Page),
    /// The delta link is no longer good, and only a fresh listing is.
    Expired,
}

/// Why Graph won't take an edit of the owner's as it is.
pub struct Refused(pub String);

pub type Sent = std::result::Result<(), Refused>;

impl Graph {
    pub fn new(policy: &'static Policy, credential: Credential, state: Dir) -> Result<Graph> {
        let client = Client::builder()
            .https_only(true)
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        Ok(Graph { client, credential, policy, state, access: RefCell::default() })
    }

    async fn grant(&self, form: &[(&str, &str)]) -> Result<Granted> {
        let audience = self.policy.authority("token");
        let assertion =
            self.credential.assertion(&self.policy.client_id, &audience, Timestamp::now())?;
        let mut fields = vec![
            ("client_id", self.policy.client_id.as_str()),
            ("client_assertion_type", "urn:ietf:params:oauth:client-assertion-type:jwt-bearer"),
            ("client_assertion", assertion.as_str()),
            ("scope", scope(self.policy)),
        ];
        fields.extend_from_slice(form);
        let reply = self.client.post(&audience).form(&fields).send().await.context("entra")?;
        let status = reply.status();
        let text = reply.text().await?;
        anyhow::ensure!(status.is_success(), "entra token: HTTP {status}: {}", refusal(&text));
        let granted: Granted =
            serde_json::from_str(&text).context("entra token: unreadable reply")?;
        let expires = Timestamp::now() + (granted.expires_in - 60).seconds();
        *self.access.borrow_mut() = Some((granted.access_token.clone(), expires));
        if let Some(refresh) = &granted.refresh_token {
            self.state.save(state::TOKENS, &Tokens { refresh: Some(refresh.clone()) })?;
        }
        Ok(granted)
    }

    /// Redeems the callback's code, which is the only way to a first refresh token.
    pub async fn redeem(&self, code: &str, verifier: &Secret) -> Result<()> {
        let redirect = self.policy.redirect();
        let form = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect.as_str()),
            ("code_verifier", verifier.expose()),
        ];
        let granted = self.grant(&form).await?;
        anyhow::ensure!(granted.refresh_token.is_some(), "entra granted no refresh token");
        Ok(())
    }

    async fn token(&self) -> Result<Secret> {
        if let Some((token, until)) = self.access.borrow().clone()
            && Timestamp::now() < until
        {
            return Ok(token);
        }
        let Tokens { refresh } = self.state.load(state::TOKENS)?;
        let refresh = refresh.context("not signed in: run `outlook-sync login`")?;
        let form = [("grant_type", "refresh_token"), ("refresh_token", refresh.expose())];
        Ok(self.grant(&form).await?.access_token)
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
                *self.access.borrow_mut() = None;
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
            "notificationUrl": format!("{}/notify", self.policy.public),
            "lifecycleNotificationUrl": format!("{}/lifecycle", self.policy.public),
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
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Made {
            id: String,
            expiration_date_time: Timestamp,
        }
        let made: Made =
            serde_json::from_str(&text).context("graph subscribe: unreadable reply")?;
        Ok(Subscription {
            id: made.id,
            client_state: client_state.clone(),
            expires: made.expiration_date_time,
        })
    }

    /// Pushes the subscription's end out by its lifetime; false when Graph no longer has it.
    pub async fn renew(&self, subscription: &mut Subscription) -> Result<bool> {
        let expires = Timestamp::now() + LIFETIME.minutes();
        let url = format!("{GRAPH}/subscriptions/{}", subscription.id);
        let (status, text) =
            self.call(Method::PATCH, &url, Some(json!({"expirationDateTime": expires}))).await?;
        match status {
            StatusCode::NOT_FOUND => Ok(false),
            s if s.is_success() => {
                subscription.expires = expires;
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

    /// A write of one event: an error when trying again may work, Refused when Graph won't take
    /// this event as it is.
    async fn write(&self, method: Method, url: &str, body: Option<Value>) -> Result<Sent> {
        let (status, text) = self.call(method.clone(), url, body).await?;
        match status.as_u16() {
            200..300 => Ok(Ok(())),
            400 | 404 | 409 | 422 => Ok(Err(Refused(format!("HTTP {status}: {}", refusal(&text))))),
            _ => bail!("graph {method}: HTTP {status}: {}", refusal(&text)),
        }
    }

    /// Creates the owner's new event; `transaction` makes a retried create the same event.
    pub async fn create(&self, mut event: Value, transaction: &str) -> Result<Sent> {
        event["transactionId"] = json!(transaction);
        self.write(Method::POST, &format!("{GRAPH}/me/events"), Some(event)).await
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

    /// The first page of a fresh listing over `[from, to)`.
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

/// An error reply's code and description, never a token-bearing body.
fn refusal(text: &str) -> String {
    let reply: Value = serde_json::from_str(text).unwrap_or_default();
    let error = &reply["error"];
    match (error.as_str(), error["code"].as_str()) {
        (Some(code), _) => format!("{code}: {}", reply["error_description"].as_str().unwrap_or("")),
        (_, Some(code)) => format!("{code}: {}", error["message"].as_str().unwrap_or("")),
        _ => "unreadable error".into(),
    }
}

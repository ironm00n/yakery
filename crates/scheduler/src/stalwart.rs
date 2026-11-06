use std::collections::BTreeSet;
use std::iter;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::header::{AUTHORIZATION, CONTENT_TYPE};
use hyper::{Method, Request, StatusCode};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use invitation::{At, Invitation, Person};
use jiff::Timestamp;
use jmap::{Got, Queried, Session};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use text::Address;

use crate::link::{Booking, Created, Digest, Limits};
use crate::policy::Policy;
use crate::slots::{Period, weeks};

pub trait Calendar {
    fn owner(&self) -> &Address;
    async fn busy(&self, period: Period) -> Result<Vec<Period>>;
    async fn create(&self, event: Value) -> Result<Created>;
}

/// The booking's JSCalendar event, without the calendar, which only [`Stalwart`] knows.
pub fn event(
    policy: &Policy,
    owner: &Address,
    digest: Digest,
    limits: &Limits,
    booking: &Booking,
) -> Value {
    let booker = &booking.booker;
    let note = if booker.note.is_empty() { String::new() } else { format!("{}\n\n", booker.note) };
    let address: Address = booker.address.parse().expect("a checked form's address parses");
    Invitation {
        uid: &format!("{digest}@{}", policy.host),
        title: &format!("{} with {}", limits.title.as_deref().unwrap_or("Meeting"), booker.name),
        description: Some(&format!("{note}Booked with link {}", digest.id())),
        start: At { when: booking.start, zone: &policy.zone },
        duration: limits.duration,
        location: limits.location.as_deref(),
        organizer: owner,
        organizer_attends: true,
        invited: &[Person { address: &address, name: Some(&booker.name) }],
    }
    .event()
}

/// An account signed in to, whose permissions and session the core has checked.
struct Login {
    address: String,
    account: String,
    auth: String,
}

/// Stalwart's loopback origin, which every request goes to and no answer can change.
struct Http {
    client: Client<HttpConnector, Full<Bytes>>,
    origin: String,
}

pub struct Stalwart {
    http: Http,
    login: Login,
    owner: Address,
    owner_id: String,
    bookings: String,
    groups: Vec<String>,
}

/// sysexits.h's: systemd starts the core again, with the new app password.
const EX_TEMPFAIL: i32 = 75;

pub const TIMEOUT: Duration = Duration::from_secs(30);

impl Stalwart {
    /// Signs in, refusing to go on unless schedule@ holds no permission beyond the policy's and
    /// sees one account besides its own, the owner's, with Bookings its only calendar there.
    /// What it can't see goes unchecked: a free/busy share, and a share granting writing
    /// without reading.
    pub async fn connect(policy: &Policy, secret: &Path) -> Result<Stalwart> {
        let client = Client::builder(TokioExecutor::new()).build_http();
        let http = Http { client, origin: format!("http://{}", policy.stalwart) };
        let address = policy.account.address.to_string();
        let secret = std::fs::read_to_string(secret).context("the app password")?;
        let auth = format!("Basic {}", STANDARD.encode(format!("{address}:{}", secret.trim())));
        let mut login = Login { address, account: String::new(), auth };

        // no more than the policy's, not exactly: Stalwart 0.16's `/api/account` hides the
        // `jmapPrincipal*` permissions while directory queries are off, and v1's doesn't
        let held: BTreeSet<String> = serde_json::from_value(
            http.get::<Value>(&login, "/api/account").await?["permissions"].take(),
        )?;
        let beyond: Vec<_> = held.difference(&policy.account.permissions).collect();
        ensure!(beyond.is_empty(), "{} holds {beyond:?}, beyond the policy's", login.address);

        let session: Session = http.get(&login, "/jmap/session").await?;
        let (own, others): (Vec<_>, Vec<_>) =
            session.accounts.iter().partition(|(_, a)| a.name == login.address);
        let ([(own, _)], [(owner_id, owner)]) = (own.as_slice(), others.as_slice()) else {
            let seen: Vec<&String> = session.accounts.values().map(|a| &a.name).collect();
            bail!(
                "{} must see one account besides its own, the one that shared Bookings, not {seen:?}",
                login.address
            );
        };
        login.account = own.to_string();
        let (owner_id, owner) = (owner_id.to_string(), owner.name.clone());
        let owner: Address = owner
            .parse()
            .with_context(|| format!("the account {owner:?} is named by no address"))?;

        let calendars: Got<Value> =
            http.call(&login, "Calendar/get", json!({"accountId": owner_id})).await?;
        let [bookings] = calendars.list.as_slice() else {
            bail!("{} must see exactly one calendar of {owner}'s, Bookings", login.address);
        };
        let rights = &bookings["myRights"];
        let right = |r: &str| rights[r].as_bool() == Some(true);
        ensure!(
            right("mayReadItems")
                && right("mayWriteAll")
                && !right("mayShare")
                && !right("mayDelete"),
            "Bookings must be shared with {} to read and write items, and no more: {rights}",
            login.address
        );
        let bookings = bookings["id"].as_str().context("Bookings' id")?.into();

        let mut groups = vec![];
        for group in &policy.groups {
            let found: Queried = http
                .call(
                    &login,
                    "Principal/query",
                    json!({"accountId": login.account, "filter": {"email": group}}),
                )
                .await?;
            let [id] = found.ids.as_slice() else { bail!("no account {group}") };
            groups.push(id.clone());
        }
        Ok(Stalwart { http, login, owner, owner_id, bookings, groups })
    }
}

impl Http {
    async fn send<T: DeserializeOwned>(
        &self,
        login: &Login,
        method: Method,
        path: &str,
        body: Vec<u8>,
    ) -> Result<T> {
        let request = Request::builder()
            .method(method)
            .uri(format!("{}{path}", self.origin))
            .header(AUTHORIZATION, &login.auth)
            .header(CONTENT_TYPE, "application/json")
            .body(Full::new(Bytes::from(body)))?;
        let (status, bytes) = tokio::time::timeout(TIMEOUT, async {
            let response = self.client.request(request).await?;
            let status = response.status();
            anyhow::Ok((status, response.into_body().collect().await?.to_bytes()))
        })
        .await??;
        if status == StatusCode::UNAUTHORIZED {
            yakery_log::warn!(
                "{} was refused; exiting to read the app password again",
                login.address
            );
            std::process::exit(EX_TEMPFAIL);
        }
        ensure!(status.is_success(), "{path} answered {status} for {}", login.address);
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn get<T: DeserializeOwned>(&self, login: &Login, path: &str) -> Result<T> {
        self.send(login, Method::GET, path, vec![]).await
    }

    async fn call<T: DeserializeOwned>(
        &self,
        login: &Login,
        method: &'static str,
        args: Value,
    ) -> Result<T> {
        let mut request = jmap::Request::new(&[jmap::CORE, jmap::CALENDARS, jmap::PRINCIPALS]);
        let call = request.call(method, args);
        let body = serde_json::to_vec(&request)?;
        let response: jmap::Response = self.send(login, Method::POST, "/jmap/", body).await?;
        response.answer(call).map_err(|e| anyhow!("{method} for {} failed: {e}", login.address))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Busy {
    utc_start: Timestamp,
    utc_end: Timestamp,
}

impl Calendar for Stalwart {
    fn owner(&self) -> &Address {
        &self.owner
    }

    async fn busy(&self, period: Period) -> Result<Vec<Period>> {
        let mut found = vec![];
        for principal in iter::once(&self.owner_id).chain(&self.groups) {
            for week in weeks(period) {
                let args = json!({"accountId": self.login.account, "id": principal, "utcStart": week.start, "utcEnd": week.end});
                let mut answer: Value =
                    self.http.call(&self.login, "Principal/getAvailability", args).await?;
                let list: Vec<Busy> = serde_json::from_value(answer["list"].take())?;
                found.extend(
                    list.into_iter().map(|b| Period { start: b.utc_start, end: b.utc_end }),
                );
            }
        }
        Ok(found)
    }

    async fn create(&self, mut event: Value) -> Result<Created> {
        event["calendarIds"] = json!({&self.bookings: true});
        let args = json!({"accountId": self.owner_id, "sendSchedulingMessages": true, "create": {"b": event}});
        Ok(created(&self.http.call(&self.login, "CalendarEvent/set", args).await?))
    }
}

fn created(result: &Value) -> Created {
    let refusal = &result["notCreated"]["b"];
    match result["created"]["b"]["id"].as_str() {
        Some(id) => Created::Event(id.into()),
        None if refusal["type"] == "invalidProperties"
            && refusal["properties"] == json!(["uid"]) =>
        {
            Created::UidTaken
        }
        None => Created::Refused(refusal.to_string()),
    }
}

#[cfg(test)]
#[path = "../tests/stalwart.rs"]
mod tests;

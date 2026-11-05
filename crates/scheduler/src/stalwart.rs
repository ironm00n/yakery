//! Stalwart over JMAP on loopback, as `schedule@`: busy time from the calendars of whoever shared
//! Bookings with it and of the groups the policy names, and bookings into Bookings.

use std::collections::BTreeSet;
use std::iter;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::header::{AUTHORIZATION, CONTENT_TYPE};
use hyper::{Method, Request, StatusCode};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use jiff::Timestamp;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::link::{Booking, Created, Digest, Limits};
use crate::policy::Policy;
use crate::slots::{Period, weeks};

pub trait Calendar {
    /// The address of the account the bookings go to.
    fn owner(&self) -> &str;
    async fn busy(&self, period: Period) -> Result<Vec<Period>>;
    async fn create(&self, event: Value) -> Result<Created>;
}

/// The booking's JSCalendar event, without the calendar, which only [`Stalwart`] knows.
pub fn event(
    policy: &Policy,
    owner: &str,
    digest: Digest,
    limits: &Limits,
    booking: &Booking,
) -> Value {
    let owner = format!("mailto:{owner}");
    let booker = &booking.booker;
    let note = if booker.note.is_empty() { String::new() } else { format!("{}\n\n", booker.note) };
    let mut event = json!({
        "@type": "Event",
        "uid": format!("{digest}@{}", policy.host),
        "title": format!("{} with {}", limits.title.as_deref().unwrap_or("Meeting"), booker.name),
        "description": format!("{note}Booked with link {}", digest.id()),
        "start": booking.start.to_zoned(policy.zone.clone()).datetime().strftime("%Y-%m-%dT%H:%M:%S").to_string(),
        "timeZone": policy.zone_name(),
        "duration": limits.duration.to_string(),
        "organizerCalendarAddress": owner,
        "participants": {
            "owner": {"calendarAddress": owner, "roles": {"owner": true, "attendee": true}, "participationStatus": "accepted"},
            "booker": {"name": booker.name, "calendarAddress": format!("mailto:{}", booker.address), "roles": {"attendee": true}, "expectReply": true},
        },
    });
    if let Some(place) = &limits.location {
        event["locations"] = json!({"place": {"name": place}});
    }
    event
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
    owner: String,
    owner_id: String,
    bookings: String,
    groups: Vec<String>,
}

/// The longest one call to Stalwart may take, answer and all.
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
        let held: BTreeSet<String> =
            serde_json::from_value(http.get(&login, "/api/account").await?["permissions"].take())?;
        let beyond: Vec<_> = held.difference(&policy.account.permissions).collect();
        ensure!(beyond.is_empty(), "{} holds {beyond:?}, beyond the policy's", login.address);

        let session = http.get(&login, "/jmap/session").await?;
        let accounts = session["accounts"].as_object().context("a JMAP session")?;
        let name = |a: &Value| a["name"].as_str().unwrap_or_default().to_string();
        let (own, others): (Vec<_>, Vec<_>) =
            accounts.iter().partition(|(_, a)| name(a) == login.address);
        let ([(own, _)], [(owner_id, owner)]) = (own.as_slice(), others.as_slice()) else {
            let seen: Vec<String> = accounts.values().map(name).collect();
            bail!(
                "{} must see one account besides its own, the one that shared Bookings, not {seen:?}",
                login.address
            );
        };
        login.account = own.to_string();
        let (owner_id, owner) = (owner_id.to_string(), name(owner));

        let calendars = http.call(&login, "Calendar/get", json!({"accountId": owner_id})).await?;
        let [bookings] = calendars["list"].as_array().map(Vec::as_slice).unwrap_or_default() else {
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
            let found = http
                .call(
                    &login,
                    "Principal/query",
                    json!({"accountId": login.account, "filter": {"email": group}}),
                )
                .await?;
            let [id] = found["ids"].as_array().map(Vec::as_slice).unwrap_or_default() else {
                bail!("no account {group}");
            };
            groups.push(id.as_str().context("a principal id")?.into());
        }
        Ok(Stalwart { http, login, owner, owner_id, bookings, groups })
    }
}

impl Http {
    async fn send(
        &self,
        login: &Login,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        let request = Request::builder()
            .method(method)
            .uri(format!("{}{path}", self.origin))
            .header(AUTHORIZATION, &login.auth)
            .header(CONTENT_TYPE, "application/json")
            .body(Full::new(body.map(|b| Bytes::from(b.to_string())).unwrap_or_default()))?;
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
            std::process::exit(75);
        }
        ensure!(status.is_success(), "{path} answered {status} for {}", login.address);
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn get(&self, login: &Login, path: &str) -> Result<Value> {
        self.send(login, Method::GET, path, None).await
    }

    async fn call(&self, login: &Login, method: &str, args: Value) -> Result<Value> {
        let using = [
            "urn:ietf:params:jmap:core",
            "urn:ietf:params:jmap:calendars",
            "urn:ietf:params:jmap:principals",
        ];
        let body = json!({"using": using, "methodCalls": [[method, args, "0"]]});
        let mut response =
            self.send(login, Method::POST, "/jmap/", Some(body)).await?["methodResponses"][0]
                .take();
        ensure!(response[0] == method, "{method} for {} failed: {}", login.address, response[1]);
        Ok(response[1].take())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Busy {
    utc_start: Timestamp,
    utc_end: Timestamp,
}

impl Calendar for Stalwart {
    fn owner(&self) -> &str {
        &self.owner
    }

    async fn busy(&self, period: Period) -> Result<Vec<Period>> {
        let mut found = vec![];
        for principal in iter::once(&self.owner_id).chain(&self.groups) {
            for week in weeks(period) {
                let args = json!({"accountId": self.login.account, "id": principal, "utcStart": week.start, "utcEnd": week.end});
                let list: Vec<Busy> = serde_json::from_value(
                    self.http.call(&self.login, "Principal/getAvailability", args).await?["list"]
                        .take(),
                )?;
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

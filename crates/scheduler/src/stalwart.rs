//! Stalwart over JMAP on loopback, as one program account per counted calendar and `schedule@`
//! for Bookings; and the checks at start that each account can do what it should and no more.

use std::collections::{BTreeMap, BTreeSet};
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
    /// Busy periods overlapping `period`, each with the index of its counted calendar.
    async fn busy(&self, period: Period) -> Result<Vec<(Period, usize)>>;
    async fn create(&self, event: Value) -> Result<Created>;
}

/// The booking's JSCalendar event, without the calendar, which only [`Stalwart`] knows.
pub fn event(policy: &Policy, digest: Digest, limits: &Limits, booking: &Booking) -> Value {
    let owner = format!("mailto:{}", policy.owner);
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
    schedule: Login,
    counted: Vec<Login>,
    owner: String,
    bookings: String,
}

/// The longest one call to Stalwart may take, answer and all.
pub const TIMEOUT: Duration = Duration::from_secs(30);

/// Every permission each account holds, as `/api/account` lists them: it hides the
/// `jmapPrincipal*` ones while directory queries are off.
const SCHEDULE: [&str; 4] =
    ["authenticate", "calendarSchedulingSend", "jmapCalendarEventCreate", "jmapCalendarGet"];
const COUNTED: [&str; 1] = ["authenticate"];

impl Stalwart {
    /// Signs in as each account, refusing to go on unless each sees what it should, and finds
    /// the owner's account through schedule@'s session, the only other account there. What no
    /// account can see goes unchecked: a free/busy share, and a share to schedule@ that grants
    /// writing without reading.
    pub async fn connect(policy: &Policy, credentials: &Path) -> Result<Stalwart> {
        let client = Client::builder(TokioExecutor::new()).build_http();
        let http = Http { client, origin: format!("http://{}", policy.stalwart) };
        let owner = policy.owner.to_string();
        let sign_in =
            |local: String, held, others| http.sign_in(policy, credentials, local, held, others);
        let (schedule, mut seen) = sign_in("schedule".into(), &SCHEDULE, vec![&owner]).await?;
        let owner = seen.remove(&owner).context("the owner's account")?;
        let mut counted = vec![];
        for c in &policy.calendars {
            counted.push(sign_in(format!("schedule-{}", c.key), &COUNTED, vec![]).await?.0);
        }
        let calendars = http.call(&schedule, "Calendar/get", json!({"accountId": owner})).await?;
        let [bookings] = calendars["list"].as_array().map(Vec::as_slice).unwrap_or_default() else {
            bail!("schedule@ must see exactly one calendar of the owner's, Bookings");
        };
        let rights = &bookings["myRights"];
        let right = |r: &str| rights[r].as_bool() == Some(true);
        let only = right("mayReadItems")
            && right("mayWriteAll")
            && !right("mayShare")
            && !right("mayDelete");
        ensure!(
            only,
            "Bookings must be shared with schedule@ to read and write items, and no more: {rights}"
        );
        let bookings = bookings["id"].as_str().context("Bookings' id")?.into();
        Ok(Stalwart { http, schedule, counted, owner, bookings })
    }
}

impl Http {
    /// Signs in with the app password LoadCredential gave, and checks the account holds exactly
    /// `held` and its session lists only its own account and `others`; returns it with its
    /// account id, and the others' ids by name.
    async fn sign_in(
        &self,
        policy: &Policy,
        credentials: &Path,
        local: String,
        held: &[&str],
        others: Vec<&str>,
    ) -> Result<(Login, BTreeMap<String, String>)> {
        let address = format!("{local}@{}", policy.owner.domain());
        let secret = std::fs::read_to_string(credentials.join(&local))
            .with_context(|| format!("{local}'s app password"))?;
        let auth = format!("Basic {}", STANDARD.encode(format!("{address}:{}", secret.trim())));
        let mut login = Login { address, account: String::new(), auth };
        let permissions: BTreeSet<String> =
            serde_json::from_value(self.get(&login, "/api/account").await?["permissions"].take())?;
        let address = &login.address;
        ensure!(
            permissions.iter().eq(held.iter().copied().collect::<BTreeSet<_>>().iter()),
            "{address} holds {permissions:?}, not {held:?}"
        );
        let session = self.get(&login, "/jmap/session").await?;
        let accounts = session["accounts"].as_object().context("a JMAP session")?;
        let mut seen: BTreeMap<String, String> = accounts
            .iter()
            .map(|(id, a)| (a["name"].as_str().unwrap_or_default().into(), id.clone()))
            .collect();
        let wanted: BTreeSet<&str> = others.into_iter().chain([address.as_str()]).collect();
        ensure!(
            seen.keys().map(String::as_str).eq(wanted.iter().copied()),
            "{address} sees {:?}, not {wanted:?}",
            seen.keys()
        );
        login.account = seen.remove(address).expect("its own account");
        Ok((login, seen))
    }

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
                "{} was refused; exiting to read the app passwords again",
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
    async fn busy(&self, period: Period) -> Result<Vec<(Period, usize)>> {
        let mut found = vec![];
        for (i, login) in self.counted.iter().enumerate() {
            for week in weeks(period) {
                let args = json!({"accountId": login.account, "id": self.owner, "utcStart": week.start, "utcEnd": week.end});
                let list: Vec<Busy> = serde_json::from_value(
                    self.http.call(login, "Principal/getAvailability", args).await?["list"].take(),
                )?;
                found.extend(
                    list.into_iter().map(|b| (Period { start: b.utc_start, end: b.utc_end }, i)),
                );
            }
        }
        Ok(found)
    }

    async fn create(&self, mut event: Value) -> Result<Created> {
        event["calendarIds"] = json!({&self.bookings: true});
        let args = json!({"accountId": self.owner, "sendSchedulingMessages": true, "create": {"b": event}});
        Ok(created(&self.http.call(&self.schedule, "CalendarEvent/set", args).await?))
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

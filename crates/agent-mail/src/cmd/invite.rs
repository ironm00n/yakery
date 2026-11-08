//! Invitations to events, made as events in the login's own calendar: Stalwart writes and sends the invitation, any
//! update and the cancel from that event, and records each answer on it, so agent-mail writes no iTIP itself.

use std::process::ExitCode;

use invitation::{At, Details, Invitation, Length, Person, Zone, mailto};
use jiff::Timestamp;
use ring::digest::{SHA256, digest};
use text::Address;

use super::{send::sender, succeed};
use crate::{
    TIME_ZONE,
    address::Topic,
    jmap::{Call, wire},
    mail::{Answer, Mail},
    policy::{Login, Recipients},
    prelude::*,
};

wire! {
    views under Calendars (default) {
        Calendar in "Calendar/get" { id: String = "id", default: bool = "isDefault" }
        Event in "CalendarEvent/get" {
            id: String = "id", title: String = "title", start: String = "start", zone: String = "timeZone",
            organizer: String = "organizerCalendarAddress",
            participants: HashMap<String, Participant> = "participants",
        }
    }
    objects {
        Participant {
            address: String = "calendarAddress", answer: String = "participationStatus",
            roles: HashMap<String, bool> = "roles",
        }
    }
}

const CALENDARS: Call = Call::calendars("Calendar/set", None);
const EVENTS: Call = Call::calendars("CalendarEvent/set", None);
const FIND: Call = Call::calendars("CalendarEvent/query", None);

#[derive(clap::Subcommand)]
pub enum Args {
    /// Invite people to a new event, from <login>+<topic>, where their answers come back; prints its id.
    New(New),
    /// Change an invitation; Stalwart sends everyone invited the update.
    Update(Update),
    /// Cancel an invitation; Stalwart sends everyone invited the cancel.
    Cancel(Cancel),
}

#[derive(clap::Args)]
pub struct New {
    #[arg(long)]
    topic: Option<Topic>,
    #[arg(long, required = true)]
    to: Vec<Address>,
    #[arg(long)]
    title: String,
    /// RFC 3339, with its offset.
    #[arg(long)]
    start: Timestamp,
    /// ISO 8601 (`PT30M`) or `30m`.
    #[arg(long)]
    duration: Length,
    /// The event's time zone, which calendar apps show it in and keep its local time by; the deployment's by default.
    #[arg(long)]
    zone: Option<Zone>,
    #[command(flatten)]
    text: Text,
}

#[derive(clap::Args)]
#[command(group(clap::ArgGroup::new("change").required(true).multiple(true)
    .args(["title", "start", "duration", "location", "note"])))]
pub struct Update {
    id: String,
    /// The topic the invitation went from; only its organizer changes it.
    #[arg(long)]
    topic: Option<Topic>,
    #[arg(long)]
    title: Option<String>,
    #[arg(long)]
    start: Option<Timestamp>,
    #[arg(long)]
    duration: Option<Length>,
    #[arg(long, requires = "start")]
    zone: Option<Zone>,
    #[command(flatten)]
    text: Text,
}

#[derive(clap::Args)]
pub struct Cancel {
    id: String,
    #[arg(long)]
    topic: Option<Topic>,
}

#[derive(clap::Args)]
pub struct Text {
    #[arg(long)]
    location: Option<String>,
    #[arg(long)]
    note: Option<String>,
}

#[derive(clap::Args)]
pub struct List {
    #[arg(long)]
    topic: Option<Topic>,
}

fn deployment_zone() -> Result<Zone> {
    Zone::named(TIME_ZONE).context("the deployment's time zone")
}

/// An invitation's uid, from what makes it the same invitation, the addresses as they compare, without case, so
/// `invite new` run again finds the event an earlier run may have made instead of inviting twice.
fn uid(
    organizer: &Address,
    invited: &[String],
    title: &str,
    start: Timestamp,
    duration: Length,
) -> String {
    let mut invited: Vec<String> = invited.iter().map(|a| a.to_lowercase()).collect();
    invited.sort();
    let from = organizer.to_string().to_lowercase();
    let key = format!("{from}\n{}\n{title}\n{start}\n{duration}", invited.join("\n"));
    let hash = private::hex::encode(digest(&SHA256, key.as_bytes()).as_ref());
    format!("{}@{}", &hash[..32], organizer.domain())
}

async fn made(mail: &Mail, uid: &str) -> Result<Option<String>> {
    Ok(mail.call(FIND, json!({ "filter": { "uid": uid } })).await?.ids.into_iter().next())
}

/// From the shared login, another topic's invitation isn't this session's to change.
async fn mine(mail: &Mail, login: &Login, id: &str, topic: Option<&Topic>) -> Result<Event> {
    let organizer = mailto(&sender(login, topic, None)?.to_string());
    let event = mail
        .get::<Event>(&[id])
        .await?
        .into_iter()
        .next()
        .with_context(|| format!("no event {id}"))?;
    ensure!(event.organizer == organizer, "event {id} isn't an invitation from {organizer}");
    Ok(event)
}

pub async fn run(args: Args) -> Result<ExitCode> {
    succeed!("{}", invite(&Mail::open().await?, args).await?)
}

/// The invitation's id.
pub async fn invite(mail: &Mail, args: Args) -> Result<String> {
    match args {
        Args::New(new) => {
            let login = mail.login();
            let envelope =
                Recipients::check(login, sender(login, new.topic.as_ref(), None)?, new.to, vec![])?;
            let zone = new.zone.map_or_else(deployment_zone, Ok)?;
            let start = At { when: new.start, zone: &zone };
            let addresses: Vec<String> = envelope.to.iter().map(Address::to_string).collect();
            let invited: Vec<Person> =
                addresses.iter().map(|a| Person { address: a, name: None }).collect();
            let uid = uid(&envelope.from, &addresses, &new.title, new.start, new.duration);
            let mut event = Invitation {
                uid: &uid,
                title: &new.title,
                start,
                duration: new.duration,
                description: new.text.note.as_deref(),
                location: new.text.location.as_deref(),
                organizer: &envelope.from.to_string(),
                organizer_attends: false,
                invited: &invited,
            }
            .event();
            let calendar = calendar(mail).await?;
            event["calendarIds"] = json!({ calendar: true });
            let created = set(mail, json!({ "create": { "i": event } })).await;
            match created.and_then(|answer| Ok(answer.id("i")?.to_string())) {
                Ok(id) => Ok(id),
                // refused as a uid already taken, or with an answer that never came: the uid finds what was made
                Err(e) => match made(mail, &uid).await {
                    Ok(Some(id)) => {
                        note!("{uid} was already made, as {id}");
                        Ok(id)
                    }
                    Ok(None) => Err(e),
                    Err(lookup) => {
                        Err(e.context(format!("and looking {uid} up failed too: {lookup:#}")))
                    }
                },
            }
        }
        Args::Update(update) => {
            let event = mine(mail, mail.login(), &update.id, update.topic.as_ref()).await?;
            let zone = match (update.start, update.zone) {
                (None, _) => None,
                (Some(_), Some(zone)) => Some(zone),
                (Some(_), None) => Some(Zone::named(&event.zone).or_else(|_| deployment_zone())?),
            };
            let start = update.start.zip(zone.as_ref()).map(|(when, zone)| At { when, zone });
            let change = Details {
                title: update.title.as_deref(),
                description: update.text.note.as_deref(),
                start,
                duration: update.duration,
                location: update.text.location.as_deref(),
            }
            .properties();
            set(mail, json!({ "update": { &update.id: change } })).await?;
            Ok(update.id)
        }
        Args::Cancel(cancel) => {
            mine(mail, mail.login(), &cancel.id, cancel.topic.as_ref()).await?;
            set(mail, json!({ "destroy": [&cancel.id] })).await?;
            // the uid comes from the invitation's fields, and calendars keep a cancelled uid
            note!(
                "inviting the same people to the same title, start and length again would reuse this uid, which their calendars hold as cancelled; change one of those"
            );
            Ok(cancel.id)
        }
    }
}

async fn set(mail: &Mail, mut args: Value) -> Result<Answer> {
    args["sendSchedulingMessages"] = json!(true);
    let answer = mail.call(EVENTS, args).await?;
    answer.taken()?;
    Ok(answer)
}

async fn calendar(mail: &Mail) -> Result<String> {
    if let Some(found) = mail.all::<Calendar>().await?.into_iter().find(|c| c.default) {
        return Ok(found.id);
    }
    let made =
        json!({ "create": { "c": { "name": "Invitations" } }, "onSuccessSetIsDefault": "#c" });
    Ok(mail.call(CALENDARS, made).await?.id("c")?.to_string())
}

pub async fn list(args: List) -> Result<ExitCode> {
    let lines = invitations(&Mail::open().await?, &args).await?;
    succeed!("{}", if lines.is_empty() { "no invitations".into() } else { lines.join("\n") })
}

pub async fn invitations(mail: &Mail, args: &List) -> Result<Vec<String>> {
    let login = mail.login();
    let ours = |organizer: &str| {
        organizer.strip_prefix("mailto:").is_some_and(|a| match &args.topic {
            Some(topic) => login.topic(a).as_ref() == Some(topic),
            None => login.owns(a),
        })
    };
    let lines = mail
        .all::<Event>()
        .await?
        .into_iter()
        .filter(|e| ours(&e.organizer))
        .map(|e| {
            let answers: Vec<String> = e
                .participants
                .values()
                .filter(|p| !p.roles.contains_key("owner"))
                .map(|p| {
                    let answer = if p.answer.is_empty() { "needs-action" } else { &p.answer };
                    format!("{} {answer}", p.address.trim_start_matches("mailto:"))
                })
                .collect();
            format!("{}  {} {}  {}  {}", e.id, e.start, e.zone, e.title, answers.join(", "))
        })
        .collect();
    Ok(lines)
}

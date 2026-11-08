//! Invitations to events, made as events in the login's own calendar: Stalwart writes and sends the invitation, any
//! update and the cancel from that event, and records each answer on it, so agent-mail writes no iTIP itself.

use std::process::ExitCode;

use invitation::{At, Details, Invitation, Length, Person, Zone, address, mailto};
use jiff::{
    Span, SpanRelativeTo, Timestamp,
    civil::{DateTime, time},
};
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
            duration: String = "duration",
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
    invited: &[&Address],
    title: &str,
    start: Timestamp,
    duration: Length,
) -> String {
    let lower = |a: &Address| a.to_string().to_lowercase();
    let mut invited: Vec<String> = invited.iter().map(|a| lower(a)).collect();
    invited.sort();
    let key = json!([lower(organizer), invited, title, start.to_string(), duration.to_string()]);
    let key = key.to_string();
    let hash = private::hex::encode(digest(&SHA256, key.as_bytes()).as_ref());
    format!("{}@{}", &hash[..32], organizer.domain())
}

async fn made(mail: &Mail, uid: &str) -> Result<Option<String>> {
    Ok(mail.call(FIND, json!({ "filter": { "uid": uid } })).await?.ids.into_iter().next())
}

/// From the shared login, another topic's invitation isn't this session's to change.
async fn mine(mail: &Mail, login: &Login, id: &str, topic: Option<&Topic>) -> Result<Event> {
    let organizer = sender(login, topic, None)?;
    let event = mail
        .get::<Event>(&[id])
        .await?
        .into_iter()
        .next()
        .with_context(|| format!("no event {id}"))?;
    ensure!(
        address(&event.organizer).as_ref() == Some(&organizer),
        "event {id} isn't an invitation from {}",
        mailto(&organizer)
    );
    Ok(event)
}

pub async fn run(args: Args) -> Result<ExitCode> {
    succeed!("{}", invite(&Mail::open().await?, args, Timestamp::now()).await?)
}

/// Stalwart keeps an event already over and sends nothing for it.
fn to_come(ends: Timestamp, now: Timestamp) -> Result<()> {
    ensure!(
        ends > now,
        "an event ending at {ends} is over, and Stalwart sends no invitation for it"
    );
    Ok(())
}

/// Where Stalwart ends an event from `start` with the stored `duration`, as calcard reckons it: each day 24 hours, and
/// no duration at all the end of the start's day in `zone`.
fn stalwarts_end(start: Timestamp, duration: &str, zone: &Zone) -> Result<Timestamp> {
    if duration.is_empty() {
        let day = start.to_zoned(zone.time_zone());
        return Ok(day.with().time(time(23, 59, 59, 0)).build()?.timestamp());
    }
    let span: Span = duration.parse().with_context(|| format!("the duration {duration:?}"))?;
    Ok(start.checked_add(span.to_duration(SpanRelativeTo::days_are_24_hours())?)?)
}

/// The invitation's id.
pub async fn invite(mail: &Mail, args: Args, now: Timestamp) -> Result<String> {
    match args {
        Args::New(new) => {
            let login = mail.login();
            let envelope =
                Recipients::check(login, sender(login, new.topic.as_ref(), None)?, new.to, vec![])?;
            let zone = new.zone.map_or_else(deployment_zone, Ok)?;
            let start = At { when: new.start, zone: &zone };
            let to: Vec<&Address> = (envelope.to.iter().enumerate())
                .filter(|(n, a)| !envelope.to[..*n].contains(a))
                .map(|(_, a)| a)
                .collect();
            to_come(new.start.checked_add(new.duration.get())?, now)?;
            if let Some(own) = to.iter().find(|a| login.reaches(&a.to_string())) {
                bail!("{own} is the login's own address, which Stalwart sends no invitation to");
            }
            let invited: Vec<Person> =
                to.iter().map(|a| Person { address: a, name: None }).collect();
            let uid = uid(&envelope.from, &to, &new.title, new.start, new.duration);
            let mut event = Invitation {
                uid: &uid,
                title: &new.title,
                start,
                duration: new.duration,
                description: new.text.note.as_deref(),
                location: new.text.location.as_deref(),
                organizer: &envelope.from,
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
            let held = Zone::named(&event.zone).or_else(|_| deployment_zone())?;
            let stored_start = || -> Result<Timestamp> {
                let local: DateTime = (event.start.parse())
                    .with_context(|| format!("event {}'s start {:?}", update.id, event.start))?;
                Ok(local.to_zoned(held.time_zone())?.timestamp())
            };
            let starts = update.start.map_or_else(stored_start, Ok)?;
            let zone = update.zone.unwrap_or(held);
            let ends = match update.duration {
                Some(length) => starts.checked_add(length.get())?,
                None => stalwarts_end(starts, &event.duration, &zone)
                    .with_context(|| format!("event {}'s end", update.id))?,
            };
            to_come(ends, now)?;
            let start = update.start.map(|when| At { when, zone: &zone });
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
    let from = args.topic.as_ref().map(|t| sender(login, Some(t), None)).transpose()?;
    let ours = |organizer: &str| {
        address(organizer).is_some_and(|a| match &from {
            Some(from) => a == *from,
            None => login.owns(&a.to_string()),
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
                    let shown = address(&p.address).map_or(p.address.clone(), |a| a.to_string());
                    format!("{shown} {answer}")
                })
                .collect();
            format!("{}  {} {}  {}  {}", e.id, e.start, e.zone, e.title, answers.join(", "))
        })
        .collect();
    Ok(lines)
}

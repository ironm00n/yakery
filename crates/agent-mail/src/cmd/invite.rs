//! Invitations to events, made as events in the login's own calendar: Stalwart writes and sends the invitation, any
//! update and the cancel from that event, and records each answer on it, so agent-mail writes no iTIP itself.

use std::process::ExitCode;

use invitation::{Details, Invitation, Person};
use jiff::{SignedDuration, Timestamp, tz::TimeZone};
use text::Address;

use super::{send::sender, succeed};
use crate::{
    TIME_ZONE,
    address::Topic,
    jmap::{Call, wire},
    mail::{Answer, Mail},
    policy::Recipients,
    prelude::*,
};

wire! {
    views {
        Calendar in "Calendar/get" under Calendars { id: String = "id", default: bool = "isDefault" }
        Event in "CalendarEvent/get" under Calendars {
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

#[derive(clap::Subcommand)]
pub enum Args {
    /// Invite people to a new event, from <login>+<topic>, where their answers come back; prints its id.
    New(New),
    /// Change an invitation; Stalwart sends everyone invited the update.
    Update(Update),
    /// Cancel an invitation; Stalwart sends everyone invited the cancel.
    Cancel { id: String },
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
    duration: SignedDuration,
    /// The event's time zone, which calendar apps show it in and keep its local time by; the deployment's by default.
    #[arg(long, value_parser = zone)]
    zone: Option<TimeZone>,
    #[command(flatten)]
    text: Text,
}

#[derive(clap::Args)]
pub struct Update {
    id: String,
    #[arg(long)]
    title: Option<String>,
    #[arg(long)]
    start: Option<Timestamp>,
    #[arg(long)]
    duration: Option<SignedDuration>,
    #[arg(long, value_parser = zone, requires = "start")]
    zone: Option<TimeZone>,
    #[command(flatten)]
    text: Text,
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

fn zone(name: &str) -> Result<TimeZone, jiff::Error> {
    jiff::tz::db().get(name)
}

fn zoned(at: Timestamp, zone: Option<TimeZone>) -> Result<jiff::Zoned> {
    let zone =
        zone.map_or_else(|| self::zone(TIME_ZONE).context("the deployment's time zone"), Ok)?;
    Ok(at.to_zoned(zone))
}

pub async fn run(args: Args) -> Result<ExitCode> {
    succeed!("{}", invite(&Mail::open().await?, args).await?)
}

/// What `args` asks of `mail`'s calendar; the invitation's id.
pub async fn invite(mail: &Mail, args: Args) -> Result<String> {
    match args {
        Args::New(new) => {
            let login = mail.login();
            let envelope =
                Recipients::check(login, sender(login, new.topic.as_ref(), None)?, new.to, vec![])?;
            let start = zoned(new.start, new.zone)?;
            let addresses: Vec<String> = envelope.to.iter().map(Address::to_string).collect();
            let invited: Vec<Person> =
                addresses.iter().map(|a| Person { address: a, name: None }).collect();
            let mut event = Invitation {
                title: &new.title,
                start: &start,
                duration: new.duration,
                description: new.text.note.as_deref(),
                location: new.text.location.as_deref(),
                organizer: &envelope.from.to_string(),
                organizer_attends: false,
                invited: &invited,
            }
            .event()?;
            let calendar = calendar(mail).await?;
            event["calendarIds"] = json!({ calendar: true });
            let answer = set(mail, json!({ "create": { "i": event } })).await?;
            let id = answer.created["i"]["id"].as_str().map(String::from);
            id.with_context(|| format!("stalwart made no event: {}", json!(answer.not_created)))
        }
        Args::Update(update) => {
            let start = update.start.map(|at| zoned(at, update.zone)).transpose()?;
            let change = Details {
                title: update.title.as_deref(),
                description: update.text.note.as_deref(),
                start: start.as_ref(),
                duration: update.duration,
                location: update.text.location.as_deref(),
            }
            .properties()?;
            ensure!(!change.is_empty(), "nothing to change");
            let answer = set(mail, json!({ "update": { &update.id: change } })).await?;
            ensure!(
                answer.not_updated.is_empty(),
                "stalwart refused the update: {}",
                json!(answer.not_updated)
            );
            Ok(update.id)
        }
        Args::Cancel { id } => {
            let answer = set(mail, json!({ "destroy": [&id] })).await?;
            ensure!(
                answer.not_destroyed.is_empty(),
                "stalwart refused the cancel: {}",
                json!(answer.not_destroyed)
            );
            Ok(id)
        }
    }
}

async fn set(mail: &Mail, mut args: Value) -> Result<Answer> {
    args["sendSchedulingMessages"] = json!(true);
    mail.call(EVENTS, args).await
}

/// The login's default calendar, made first when it has none.
async fn calendar(mail: &Mail) -> Result<String> {
    if let Some(found) = mail.all::<Calendar>().await?.into_iter().find(|c| c.default) {
        return Ok(found.id);
    }
    let made =
        json!({ "create": { "c": { "name": "Invitations" } }, "onSuccessSetIsDefault": "#c" });
    let answer = mail.call(CALENDARS, made).await?;
    let id = answer.created["c"]["id"].as_str().map(String::from);
    id.with_context(|| format!("stalwart made no calendar: {}", json!(answer.not_created)))
}

pub async fn list(args: List) -> Result<ExitCode> {
    let lines = invitations(&Mail::open().await?, &args).await?;
    succeed!("{}", if lines.is_empty() { "no invitations".into() } else { lines.join("\n") })
}

/// One line per invitation this login (or its `topic`) sent: its id, local start and zone, title, and each answer.
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

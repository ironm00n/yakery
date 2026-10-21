use std::{
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use chrono::DateTime;
use clap::ArgGroup;
use jmap_client::email::{self, Email, EmailBodyPart, Header, HeaderValue};
use serde_json::json;

use crate::{
    address::{Address, MessageId, Topic},
    auth::{Auth, store::now},
    body::Body,
    config::Config,
    entity::Entity,
    identity::Login,
    mail::{AGENT_MAIL, ANSWERED, Mail, SEEN},
    mime,
    outbox::{Delivery, Outbox, Outcome, describe, require_all},
    policy::{Recipients, bare_sender},
    random::{ALPHANUMERIC, Draws},
    show::{addresses, emails, id, out, plain, preview, received, utc, warn},
    signature::{ClaudeSession, Signature, host},
};

#[derive(clap::Args)]
#[command(group(ArgGroup::new("about").required(true).multiple(true).args(["subject", "reply"])))]
pub struct Args {
    #[arg(long, required_unless_present = "all")]
    to: Vec<Address>,
    #[arg(long)]
    cc: Vec<Address>,
    /// Send from the subaddress <login>+<topic>. A login every session shares sends only from one: this, or a --reply
    /// to mail sent to one.
    #[arg(long)]
    topic: Option<Topic>,
    /// Thread under this email and take its subject.
    #[arg(long, value_name = "EMAIL_ID")]
    reply: Option<String>,
    /// Reply even though the thread holds newer mail than --reply.
    #[arg(long, requires = "reply")]
    past: bool,
    /// Also send to everyone on the --reply mail but us: its From to To, its To and Cc to Cc.
    #[arg(long, requires = "reply")]
    all: bool,
    #[arg(long)]
    subject: Option<String>,
    #[arg(long, value_name = "FILE")]
    attach: Vec<PathBuf>,
    /// Send this MIME entity, built elsewhere, instead of rendering Markdown from stdin. It may carry only
    /// Content-* fields and MIME-Version; agent-mail writes every other field and adds no signature.
    #[arg(long, value_name = "FILE", conflicts_with = "attach")]
    mime: Option<PathBuf>,
}

struct Attachment {
    name: String,
    content_type: &'static str,
    bytes: Vec<u8>,
}

impl Attachment {
    fn read(path: &Path) -> Result<Attachment> {
        let name = path
            .file_name()
            .with_context(|| format!("{} names no file", path.display()))?;
        Ok(Attachment {
            name: name.to_string_lossy().into_owned(),
            content_type: content_type(path),
            bytes: fs::read(path).with_context(|| path.display().to_string())?,
        })
    }
}

fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
    {
        "patch" | "diff" => "text/x-diff",
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "html" | "htm" => "text/html",
        "csv" => "text/csv",
        "json" => "application/json",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

struct Reply {
    /// The email replied to; the reply answers it, and only it, once sent.
    parent: String,
    message_id: Vec<MessageId>,
    references: Vec<MessageId>,
    subject: String,
    topic: Option<String>,
    /// The parent's From, and its To and Cc, for `--all`.
    from: Vec<String>,
    rest: Vec<String>,
}

/// Thread mail from anyone but us that is not older than `parent`, which a reply should be read against. Mail
/// from the same second counts, since receivedAt cannot order it. This only prompts; it marks nothing answered.
fn newer_from_others<'a>(login: &Login, parent: &Email, siblings: &'a [Email]) -> Vec<&'a Email> {
    siblings
        .iter()
        .filter(|m| id(m) != id(parent) && received(m) >= received(parent))
        .filter(|m| !addresses(m.from()).any(|a| login.owns(a)))
        .collect()
}

impl Reply {
    async fn to(mail: &Mail, parent_id: &str, past: bool) -> Result<Reply> {
        use email::Property as P;
        let parent_props = [
            P::MessageId,
            P::References,
            P::Subject,
            P::From,
            P::To,
            P::Cc,
            P::ReceivedAt,
            P::ThreadId,
        ];
        let missing = || format!("no email {parent_id}");
        let current = mail.current(parent_id).await?.with_context(missing)?;
        let parent = mail
            .emails(std::slice::from_ref(&current), &parent_props, |_| {})
            .await?
            .into_iter()
            .next()
            .with_context(missing)?;
        let thread = parent.thread_id().unwrap_or_default().to_owned();
        let ids: Vec<String> = mail
            .threads(&[thread])
            .await?
            .iter()
            .flat_map(|t| t.email_ids().to_vec())
            .collect();
        let members = mail
            .emails(&ids, &[P::From, P::ReceivedAt, P::Preview], |_| {})
            .await?;
        let login = &mail.auth.login;
        let newer = newer_from_others(login, &parent, &members);
        if !newer.is_empty() && !past {
            let lines: Vec<String> = newer
                .iter()
                .map(|m| {
                    format!(
                        "  {}  {}  {}: {}",
                        id(m),
                        utc(received(m)),
                        emails(m.from()),
                        preview(m, 100)
                    )
                })
                .collect();
            bail!(
                "the thread has newer mail than {parent_id}; read `agent-mail thread {parent_id}` and reply to the newest, or pass --past\n{}",
                lines.join("\n")
            );
        }
        let subject = plain(parent.subject().unwrap_or_default());
        let message_id = MessageId::all_valid(parent.message_id().unwrap_or_default());
        Ok(Reply {
            parent: current,
            from: addresses(parent.from()).map(str::to_owned).collect(),
            rest: addresses(parent.to())
                .chain(addresses(parent.cc()))
                .map(str::to_owned)
                .collect(),
            references: [
                MessageId::all_valid(parent.references().unwrap_or_default()),
                message_id.clone(),
            ]
            .concat(),
            message_id,
            subject: match subject.get(..3) {
                Some(re) if re.eq_ignore_ascii_case("re:") => subject.clone(),
                _ => format!("Re: {subject}"),
            },
            topic: login
                .first_topic_address(
                    addresses(parent.from())
                        .chain(addresses(parent.to()))
                        .chain(addresses(parent.cc())),
                )
                .map(str::to_owned),
        })
    }
}

/// What a mail says: Markdown rendered here, with its attachments, or an entity rendered elsewhere.
enum Content {
    Markdown {
        body: Body,
        attachments: Vec<Attachment>,
    },
    Mime(Entity),
}

const DRAFT: [&str; 3] = ["$draft", SEEN, AGENT_MAIL];

/// The mail's own header fields, decided once for either content.
struct Heading {
    from: Address,
    to: Vec<Address>,
    cc: Vec<Address>,
    subject: String,
    in_reply_to: Vec<MessageId>,
    references: Vec<MessageId>,
}

impl Heading {
    fn decide(login: &Login, args: &Args, reply: Option<&Reply>) -> Result<Heading> {
        let recipients = match (reply, args.all) {
            (Some(reply), true) => {
                let (to, cc) = reply_all(login, &args.to, &args.cc, &reply.from, &reply.rest)?;
                Recipients::check(login, to, cc)?
            }
            _ => Recipients::check(login, args.to.clone(), args.cc.clone())?,
        };
        let subject = match (&args.subject, reply) {
            (Some(subject), _) => plain(subject),
            (None, Some(reply)) => reply.subject.clone(),
            (None, None) => bail!("send needs --subject or --reply"),
        };
        let from = match (&args.topic, reply.and_then(|r| r.topic.as_deref())) {
            (Some(topic), _) => login.topic_address(topic).parse()?,
            (None, Some(address)) => address.parse()?,
            (None, None) => bare_sender(login).context(
                "send needs --topic, or --reply to mail sent to one of the login's topic addresses",
            )?,
        };
        Ok(Heading {
            from,
            to: recipients.to,
            cc: recipients.cc,
            subject,
            in_reply_to: reply.map(|r| r.message_id.clone()).unwrap_or_default(),
            references: reply.map(|r| r.references.clone()).unwrap_or_default(),
        })
    }

    fn everyone(&self) -> Vec<Address> {
        self.to.iter().chain(&self.cc).cloned().collect()
    }

    /// The fields of a raw message, each ending in CRLF: those the Markdown path sets through JMAP, and the Date
    /// and Message-ID Stalwart adds to those.
    fn fields(&self, date_secs: i64, message_id: &MessageId) -> String {
        let join = |items: Vec<String>, by: &str| items.join(by);
        let addresses =
            |list: &[Address]| join(list.iter().map(Address::to_string).collect(), ", ");
        let ids = |list: &[MessageId]| join(list.iter().map(MessageId::to_string).collect(), " ");
        let date = DateTime::from_timestamp(date_secs, 0)
            .unwrap_or_default()
            .to_rfc2822();
        let mut fields = vec![
            format!("Date: {date}"),
            format!("From: {}", self.from),
            format!("To: {}", addresses(&self.to)),
        ];
        if !self.cc.is_empty() {
            fields.push(format!("Cc: {}", addresses(&self.cc)));
        }
        fields.push(format!("Subject: {}", mime::text(&self.subject)));
        fields.push(format!("Message-ID: {message_id}"));
        if !self.in_reply_to.is_empty() {
            fields.push(format!("In-Reply-To: {}", ids(&self.in_reply_to)));
        }
        if !self.references.is_empty() {
            fields.push(format!("References: {}", ids(&self.references)));
        }
        fields.push(format!("Disposition-Notification-To: {}", self.from));
        fields.iter().map(|f| format!("{f}\r\n")).collect()
    }
}

pub async fn run(args: Args) -> Result<()> {
    let config = Config::load()?;
    Recipients::check(&config.login, args.to.clone(), args.cc.clone())?;
    let (host, me) = (host(), ClaudeSession::own());
    let content = match &args.mime {
        Some(path) => {
            let bytes = fs::read(path).with_context(|| path.display().to_string())?;
            let entity =
                Entity::parse(&bytes).with_context(|| format!("--mime {}", path.display()))?;
            Content::Mime(entity)
        }
        None => {
            let markdown = io::read_to_string(io::stdin())?;
            ensure!(
                !markdown.trim().is_empty(),
                "send reads the body from stdin, and it was empty"
            );
            let attachments = args
                .attach
                .iter()
                .map(|path| Attachment::read(path))
                .collect::<Result<Vec<_>>>()?;
            let body = Body::new(&markdown, &Signature::new(&host, me.as_ref()))?;
            Content::Markdown { body, attachments }
        }
    };

    let mail = Mail::connect(Auth::new(config)?).await?;
    let outbox = Outbox::open(&mail).await?;
    let reply = match &args.reply {
        Some(parent) => Some(Reply::to(&mail, parent, args.past).await?),
        None => None,
    };
    let heading = Heading::decide(&mail.auth.login, &args, reply.as_ref())?;
    let draft = match &content {
        Content::Markdown { body, attachments } => {
            draft_markdown(&mail, &outbox, &heading, body, attachments).await?
        }
        Content::Mime(entity) => draft_mime(&mail, &outbox, &heading, entity).await?,
    };
    deliver(&mail, &outbox, &draft, &heading, reply.as_ref()).await
}

/// The draft of a mail rendered here, through JMAP's body parts, its attachments uploaded first.
async fn draft_markdown(
    mail: &Mail,
    outbox: &Outbox,
    heading: &Heading,
    body: &Body,
    attachments: &[Attachment],
) -> Result<String> {
    let mut parts = Vec::with_capacity(attachments.len());
    for a in attachments {
        let blob = mail
            .with(async |s| {
                s.client
                    .upload(None, a.bytes.clone(), Some(a.content_type))
                    .await
            })
            .await?;
        // EmailBodyPart has no disposition setter.
        let part: EmailBodyPart = serde_json::from_value(json!({
            "blobId": blob.blob_id(), "type": a.content_type, "name": a.name, "disposition": "attachment",
        }))?;
        parts.push(part);
    }
    let from = heading.from.to_string();
    let mut created = mail
        .call(|r| {
            let draft = r.set_email().create_with_id("draft");
            draft
                .mailbox_ids([outbox.drafts.as_str()])
                .keywords(DRAFT)
                .from([from.as_str()])
                .to(heading.to.iter().map(Address::to_string))
                .subject(&heading.subject)
                .body_value("text".into(), body.text.as_str())
                .body_value("html".into(), body.html.as_str())
                .text_body(
                    EmailBodyPart::new()
                        .part_id("text")
                        .content_type("text/plain"),
                )
                .html_body(
                    EmailBodyPart::new()
                        .part_id("html")
                        .content_type("text/html"),
                );
            if !heading.cc.is_empty() {
                draft.cc(heading.cc.iter().map(Address::to_string));
            }
            if !heading.in_reply_to.is_empty() {
                draft.in_reply_to(heading.in_reply_to.iter().map(MessageId::bare));
            }
            if !heading.references.is_empty() {
                draft.references(heading.references.iter().map(MessageId::bare));
            }
            draft.header(
                Header::as_text("Disposition-Notification-To", false),
                HeaderValue::AsText(from.clone()),
            );
            for part in &parts {
                draft.attachment(part.clone());
            }
        })
        .await
        .context("nothing was sent; the draft may or may not exist")?;
    let draft = created.next()?.unwrap_set_email()?.created("draft")?;
    Ok(id(&draft).to_owned())
}

/// The draft of a mail rendered elsewhere: agent-mail's fields, then the entity, stored as it is.
async fn draft_mime(
    mail: &Mail,
    outbox: &Outbox,
    heading: &Heading,
    entity: &Entity,
) -> Result<String> {
    let token = Draws::default().string(ALPHANUMERIC, 24)?;
    let domain = mail.auth.login.address().domain();
    let message_id: MessageId = format!("{token}@{domain}").parse()?;
    let message = entity.message(&heading.fields(now(), &message_id));
    mail.import(message, &outbox.drafts, &DRAFT)
        .await
        .context("nothing was sent; the draft may or may not exist")
}

/// Submits the draft to everyone on `heading` and files it; a draft that reached nobody is removed, and the mail
/// replied to is marked answered only when every recipient took the reply.
async fn deliver(
    mail: &Mail,
    outbox: &Outbox,
    draft: &str,
    heading: &Heading,
    reply: Option<&Reply>,
) -> Result<()> {
    let everyone = heading.everyone();
    let (submission, delivery) = match outbox.send(mail, draft, &everyone).await {
        Outcome::Submitted { id, delivery } => (id, delivery),
        Outcome::Unsent(e) => return Err(discard(mail, draft, e).await),
        Outcome::Unknown(e) => {
            return Err(e.context(format!(
                "not known to be sent; the copy is still in Drafts as {draft}"
            )));
        }
    };
    if let Err(e) = require_all(&submission, &delivery) {
        return Err(discard(mail, draft, e).await);
    }
    if let Err(e) = outbox.file(mail, draft).await {
        warn(&format!(
            "sent (submission {submission}), but filing {draft} into Sent failed: {e:#}"
        ));
    }
    match &delivery {
        Delivery::Accepted { refused } if refused.is_empty() => {
            if let Some(reply) = reply
                && let Err(e) = mail.mark(&[(reply.parent.as_str(), vec![ANSWERED])]).await
            {
                warn(&format!(
                    "sent, but marking {} answered failed, so wait will remind of it: {e:#}",
                    reply.parent
                ));
            }
        }
        Delivery::Accepted { refused } => {
            warn(&format!(
                "submission {submission}: some recipients were refused\n{}",
                describe(refused)
            ));
        }
        _ => warn(&format!(
            "submission {submission} was created, but its delivery status did not come back"
        )),
    }
    let everyone: Vec<String> = everyone.iter().map(Address::to_string).collect();
    out(&format!(
        "sent {draft} \"{}\" from {} to {} (submission {submission})",
        heading.subject,
        heading.from,
        everyone.join(", ")
    ))
}

/// `--all`: everyone on the parent but us, without duplicates in any case: its From joins To, its To and Cc join
/// Cc. When the parent is ours, the first of the rest becomes the To.
fn reply_all(
    login: &Login,
    to: &[Address],
    cc: &[Address],
    from: &[String],
    rest: &[String],
) -> Result<(Vec<Address>, Vec<Address>)> {
    let others = |list: &[String]| -> Vec<Address> {
        list.iter()
            .filter(|a| !login.owns(a))
            .filter_map(|a| match a.parse() {
                Ok(address) => Some(address),
                Err(e) => {
                    warn(&format!("leaving out {}: {e:#}", plain(a)));
                    None
                }
            })
            .collect()
    };
    let unique = |list: Vec<Address>, without: &[Address]| {
        let mut kept: Vec<Address> = vec![];
        for a in list {
            if !kept.contains(&a) && !without.contains(&a) {
                kept.push(a);
            }
        }
        kept
    };
    let mut to = unique([to, &others(from)].concat(), &[]);
    let mut cc = unique([cc, &others(rest)].concat(), &to);
    if to.is_empty() && !cc.is_empty() {
        to.push(cc.remove(0));
    }
    ensure!(
        !to.is_empty(),
        "--all found nobody but us on the --reply mail"
    );
    Ok((to, cc))
}

/// Removes a draft whose submission was not created or reached nobody, so nothing unsent is left behind.
async fn discard(mail: &Mail, draft: &str, why: anyhow::Error) -> anyhow::Error {
    match mail.discard_unsent(draft).await {
        Ok(()) => why.context("not sent; the draft was removed"),
        Err(d) => why.context(format!(
            "not sent, and the draft {draft} could not be removed: {d:#}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail(id: &str, at: &str, from: &str) -> Email {
        serde_json::from_value(json!({ "id": id, "receivedAt": at, "from": [{ "email": from }] }))
            .unwrap()
    }

    fn addresses_of(list: &[&str]) -> Vec<Address> {
        list.iter().map(|a| a.parse().unwrap()).collect()
    }

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| a.to_string()).collect()
    }

    fn args(topic: Option<&str>) -> Args {
        Args {
            to: addresses_of(&["ironmoon@ironmoon.dev"]),
            cc: vec![],
            topic: topic.map(|t| t.parse().unwrap()),
            reply: None,
            past: false,
            all: false,
            subject: Some("pick one".into()),
            attach: vec![],
            mime: None,
        }
    }

    fn from(login: &str, args: &Args, reply: Option<&Reply>) -> Result<String> {
        let login: Login = login.parse().unwrap();
        Heading::decide(&login, args, reply).map(|h| h.from.to_string())
    }

    #[test]
    fn the_shared_login_sends_only_from_a_topic() {
        let refused = from("agent@ironmoon.dev", &args(None), None).unwrap_err();
        assert!(
            format!("{refused:#}").contains("sends only from a topic subaddress"),
            "{refused:#}"
        );
        assert_eq!(
            from("agent@ironmoon.dev", &args(Some("tooling")), None).unwrap(),
            "agent+tooling@ironmoon.dev"
        );
        let reply = Reply {
            parent: "p".into(),
            message_id: vec![],
            references: vec![],
            subject: "Re: [desktop · pool] x".into(),
            topic: Some("agent+pool@ironmoon.dev".into()),
            from: vec![],
            rest: vec![],
        };
        let answer = Args {
            subject: None,
            ..args(None)
        };
        assert_eq!(
            from("agent@ironmoon.dev", &answer, Some(&reply)).unwrap(),
            "agent+pool@ironmoon.dev"
        );
        let untopical = Reply {
            topic: None,
            ..reply
        };
        assert!(from("agent@ironmoon.dev", &answer, Some(&untopical)).is_err());
        assert_eq!(
            from("ada@agent.ironmoon.dev", &args(None), None).unwrap(),
            "ada@agent.ironmoon.dev"
        );
    }

    fn heading(reply: bool) -> Heading {
        let ids = |list: &[&str]| list.iter().map(|i| i.parse().unwrap()).collect();
        Heading {
            from: "agent+tooling@ironmoon.dev".parse().unwrap(),
            to: addresses_of(&["ironmoon@ironmoon.dev"]),
            cc: match reply {
                true => addresses_of(&["owenduckham11@gmail.com", "bob@agent.ironmoon.dev"]),
                false => vec![],
            },
            subject: match reply {
                true => "Re: [desktop · mail] the pick · now".into(),
                false => "[desktop · mail] pick one".into(),
            },
            in_reply_to: match reply {
                true => ids(&["p@ironmoon.dev"]),
                false => vec![],
            },
            references: match reply {
                true => ids(&["root@ironmoon.dev", "p@ironmoon.dev"]),
                false => vec![],
            },
        }
    }

    #[test]
    fn a_new_mail_carries_agent_mails_fields_only() {
        let id: MessageId = "T@ironmoon.dev".parse().unwrap();
        assert_eq!(
            heading(false).fields(1_760_000_000, &id),
            "Date: Thu, 9 Oct 2025 08:53:20 +0000\r\n\
             From: agent+tooling@ironmoon.dev\r\n\
             To: ironmoon@ironmoon.dev\r\n\
             Subject: =?UTF-8?B?W2Rlc2t0b3AgwrcgbWFpbF0gcGljayBvbmU=?=\r\n\
             Message-ID: <T@ironmoon.dev>\r\n\
             Disposition-Notification-To: agent+tooling@ironmoon.dev\r\n"
        );
    }

    #[test]
    fn a_reply_threads_under_its_parent_and_keeps_its_cc() {
        let id: MessageId = "T@ironmoon.dev".parse().unwrap();
        let fields = heading(true).fields(1_760_000_000, &id);
        let lines: Vec<&str> = fields.split("\r\n").collect();
        assert_eq!(
            lines[3..],
            [
                "Cc: owenduckham11@gmail.com, bob@agent.ironmoon.dev",
                "Subject: =?UTF-8?B?UmU6IFtkZXNrdG9wIMK3IG1haWxdIHRoZSBwaWNrIMK3IG5vdw==?=",
                "Message-ID: <T@ironmoon.dev>",
                "In-Reply-To: <p@ironmoon.dev>",
                "References: <root@ironmoon.dev> <p@ironmoon.dev>",
                "Disposition-Notification-To: agent+tooling@ironmoon.dev",
                "",
            ]
        );
    }

    #[test]
    fn mime_takes_no_attachments_of_its_own() {
        #[derive(clap::Parser)]
        struct Cli {
            #[command(flatten)]
            args: Args,
        }
        let parse = |extra: &[&str]| {
            let base = ["send", "--to", "ironmoon@ironmoon.dev", "--subject", "s"];
            <Cli as clap::Parser>::try_parse_from(base.iter().chain(extra))
        };
        let conflict = parse(&["--mime", "m.eml", "--attach", "a.png"])
            .err()
            .unwrap();
        assert_eq!(conflict.kind(), clap::error::ErrorKind::ArgumentConflict);
        assert!(parse(&["--mime", "m.eml"]).is_ok());
        assert!(parse(&["--attach", "a.png"]).is_ok());
    }

    #[test]
    fn reply_all_adds_everyone_but_us_once() {
        let login: Login = "agent@ironmoon.dev".parse().unwrap();
        let (to, cc) = reply_all(
            &login,
            &addresses_of(&["Ada@agent.ironmoon.dev"]),
            &[],
            &strings(&["ironmoon@ironmoon.dev"]),
            &strings(&[
                "agent+pool@ironmoon.dev",
                "ada@agent.ironmoon.dev",
                "IronMoon@ironmoon.dev",
                "bob@agent.ironmoon.dev",
            ]),
        )
        .unwrap();
        assert_eq!(
            to,
            addresses_of(&["ada@agent.ironmoon.dev", "ironmoon@ironmoon.dev"])
        );
        assert_eq!(cc, addresses_of(&["bob@agent.ironmoon.dev"]));
    }

    #[test]
    fn reply_all_to_our_own_mail_moves_the_first_other_to_to() {
        let login: Login = "agent@ironmoon.dev".parse().unwrap();
        let (to, cc) = reply_all(
            &login,
            &[],
            &[],
            &strings(&["agent+pool@ironmoon.dev"]),
            &strings(&["ironmoon@ironmoon.dev", "bob@agent.ironmoon.dev"]),
        )
        .unwrap();
        assert_eq!(
            (to, cc),
            (
                addresses_of(&["ironmoon@ironmoon.dev"]),
                addresses_of(&["bob@agent.ironmoon.dev"])
            )
        );
        let nobody = reply_all(
            &login,
            &[],
            &[],
            &strings(&["agent@ironmoon.dev"]),
            &strings(&["agent+x@ironmoon.dev"]),
        );
        assert!(nobody.is_err());
    }

    #[test]
    fn the_reply_guard_prompts_for_newer_and_same_second_mail_from_others() {
        let login: Login = "agent@ironmoon.dev".parse().unwrap();
        let parent = mail("p", "2026-10-17T10:00:00Z", "ironmoon@ironmoon.dev");
        let thread = [
            mail("old", "2026-10-17T09:00:00Z", "ironmoon@ironmoon.dev"),
            parent.clone(),
            mail("same", "2026-10-17T10:00:00Z", "ironmoon@ironmoon.dev"),
            mail("ours", "2026-10-17T11:00:00Z", "agent+tooling@ironmoon.dev"),
            mail("his", "2026-10-17T12:00:00Z", "ironmoon@ironmoon.dev"),
        ];
        let newer: Vec<&str> = newer_from_others(&login, &parent, &thread)
            .into_iter()
            .map(id)
            .collect();
        assert_eq!(newer, ["same", "his"]);
    }
}

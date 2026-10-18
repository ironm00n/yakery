use std::{
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use clap::ArgGroup;
use jmap_client::email::{self, Email, EmailBodyPart, Header, HeaderValue};
use serde_json::json;

use crate::{
    address::{Address, MessageId, Topic},
    auth::Auth,
    body::Body,
    config::Config,
    identity::Login,
    mail::{AGENT_MAIL, ANSWERED, Mail},
    outbox::{Delivery, Outbox, Submitted, describe, not_processed, require_all},
    policy::Recipients,
    show::{addresses, emails, id, out, plain, preview, received, utc, warn},
    signature::{ClaudeSession, Signature, host, subject_prefix},
};

#[derive(clap::Args)]
#[command(group(ArgGroup::new("about").required(true).multiple(true).args(["subject", "reply"])))]
pub struct Args {
    #[arg(long, required_unless_present = "all")]
    to: Vec<Address>,
    #[arg(long)]
    cc: Vec<Address>,
    /// Send from the subaddress <login>+<topic>.
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

pub async fn run(args: Args) -> Result<()> {
    let config = Config::load()?;
    Recipients::check(&config.login, args.to.clone(), args.cc.clone())?;
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
    let (host, me) = (host(), ClaudeSession::own());
    let body = Body::new(&markdown, &Signature::new(&host, me.as_ref()))?;

    let mail = Mail::connect(Auth::new(config)?).await?;
    let login = &mail.auth.login;
    let outbox = Outbox::open(&mail).await?;
    let reply = match &args.reply {
        Some(parent) => Some(Reply::to(&mail, parent, args.past).await?),
        None => None,
    };
    let recipients = match (&reply, args.all) {
        (Some(reply), true) => {
            let (to, cc) = reply_all(login, &args.to, &args.cc, &reply.from, &reply.rest)?;
            Recipients::check(login, to, cc)?
        }
        _ => Recipients::check(login, args.to, args.cc)?,
    };
    let subject = match (&args.subject, &reply) {
        (Some(subject), _) => plain(&format!("{} {subject}", subject_prefix(&host, me.as_ref()))),
        (None, Some(reply)) => reply.subject.clone(),
        (None, None) => bail!("send needs --subject or --reply"),
    };
    let from = match &args.topic {
        Some(topic) => login.topic_address(topic),
        None => reply
            .as_ref()
            .and_then(|r| r.topic.clone())
            .unwrap_or_else(|| login.to_string()),
    };

    let mut parts = Vec::with_capacity(attachments.len());
    for a in &attachments {
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

    let everyone: Vec<Address> = recipients
        .to
        .iter()
        .chain(&recipients.cc)
        .cloned()
        .collect();
    let mut created = mail
        .call(|r| {
            let draft = r.set_email().create_with_id("draft");
            draft
                .mailbox_ids([outbox.drafts.as_str()])
                .keywords(["$draft", "$seen", AGENT_MAIL])
                .from([from.as_str()])
                .to(recipients.to.iter().map(Address::to_string))
                .subject(&subject)
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
            if !recipients.cc.is_empty() {
                draft.cc(recipients.cc.iter().map(Address::to_string));
            }
            if let Some(reply) = &reply {
                draft
                    .in_reply_to(reply.message_id.iter().map(MessageId::bare))
                    .references(reply.references.iter().map(MessageId::bare));
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
    let draft = id(&draft).to_owned();
    let submitted = mail
        .call(|r| outbox.submit(r, &draft, &everyone))
        .await
        .and_then(Outbox::read);
    let (submission, delivery) = match submitted {
        Ok(Submitted::Created { id, delivery }) => (id, delivery),
        Ok(Submitted::NotCreated(e)) => return Err(discard(&mail, &draft, e).await),
        Err(e) if not_processed(&e) => return Err(discard(&mail, &draft, e).await),
        Err(e) => {
            match outbox.find(&mail, &draft).await {
                Ok(Some(found)) => found,
                Ok(None) => {
                    return Err(e.context(format!(
                        "not known to be sent; the copy is still in Drafts as {draft}"
                    )));
                }
                Err(f) => {
                    return Err(e.context(format!("unknown whether it was sent ({f:#}); the copy is still in Drafts as {draft}")));
                }
            }
        }
    };
    if let Err(e) = require_all(&submission, &delivery) {
        return Err(discard(&mail, &draft, e).await);
    }
    if let Err(e) = outbox.file(&mail, &draft).await {
        warn(&format!(
            "sent (submission {submission}), but filing {draft} into Sent failed: {e:#}"
        ));
    }
    match &delivery {
        Delivery::Accepted { refused } if refused.is_empty() => {
            if let Some(reply) = &reply
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
        "sent {draft} \"{subject}\" from {from} to {} (submission {submission})",
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

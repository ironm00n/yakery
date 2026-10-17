use std::{io, process::ExitCode};

use clap::ArgGroup;
use comrak::{Options, markdown_to_html};
use lol_html::{RewriteStrSettings, element, rewrite_str};

use super::say;
use crate::{
    address::{Address, MessageId, Topic},
    mail::{Addr, Current, Email, Keyword, Mail, keywords},
    out::{Delivery, Entity, Head, Leftover, Outbox, describe},
    policy::{Login, Recipients},
    prelude::*,
    private::{now, token},
    show::{emails, plain, preview},
    store::Config,
};

#[derive(clap::Args)]
#[command(group(ArgGroup::new("about").required(true).multiple(true).args(["subject", "reply"])))]
pub struct Args {
    #[arg(long, required_unless_present = "all")]
    pub to: Vec<Address>,
    #[arg(long)]
    pub cc: Vec<Address>,
    /// Send from the subaddress <login>+<topic>. A login every session shares sends only from one: this, or a --reply
    /// to mail sent to one.
    #[arg(long)]
    pub topic: Option<Topic>,
    /// Thread under this email and take its subject.
    #[arg(long, value_name = "EMAIL_ID")]
    pub reply: Option<String>,
    /// Reply even though the thread holds newer mail than --reply.
    #[arg(long, requires = "reply")]
    pub past: bool,
    /// Also send to everyone on the --reply mail but us: its From to To, its To and Cc to Cc.
    #[arg(long, requires = "reply")]
    pub all: bool,
    #[arg(long)]
    pub subject: Option<String>,
    #[arg(long, value_name = "FILE")]
    pub attach: Vec<PathBuf>,
    /// Send this MIME entity, built elsewhere, instead of rendering Markdown from stdin. It may carry only
    /// Content-* fields and MIME-Version; agent-mail writes every other field and adds no signature.
    #[arg(long, value_name = "FILE", conflicts_with = "attach")]
    pub mime: Option<PathBuf>,
}

enum Content {
    Mime(Entity),
    Markdown([String; 2], Vec<(String, &'static str, Vec<u8>)>),
}

const DRAFT: &[Keyword] = &[Keyword::Draft, Keyword::Seen, Keyword::AgentMail];
/// Inline, since mail clients drop `<style>` unevenly; a later selector's style replaces an earlier one's. Tints are
/// `background-color` because Bulwark Mobile's dark mode re-inverts any inline `background:`.
pub const STYLES: [(&str, &str); 6] = [
    (
        "pre",
        "padding: 8px 12px; border-radius: 4px; background-color: rgba(127, 127, 127, 0.12); overflow-x: auto",
    ),
    (
        "code",
        "font-family: ui-monospace, Menlo, Consolas, monospace; padding: 1px 4px; border-radius: 3px; background-color: rgba(127, 127, 127, 0.15)",
    ),
    ("pre > code", "font-family: ui-monospace, Menlo, Consolas, monospace"),
    ("table", "border-collapse: collapse"),
    ("th, td", "border: 1px solid rgba(127, 127, 127, 0.5); padding: 4px 8px"),
    (
        "blockquote",
        "margin: 0; padding-left: 12px; border-left: 3px solid rgba(127, 127, 127, 0.5)",
    ),
];

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub session_id: String,
    pub name: Option<String>,
    pub cwd: String,
    pub bridge_session_id: Option<String>,
}

fn session() -> Option<Session> {
    let socket = PathBuf::from(std::env::var_os("CLAUDE_CODE_MESSAGING_SOCKET")?);
    let name = socket.file_name()?.to_str()?;
    let config = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from);
    let config =
        config.or_else(|| Some(PathBuf::from(std::env::var_os("HOME")?).join(".claude")))?;
    let entry = config
        .join("sessions")
        .join(format!("{}.json", name.strip_suffix(".sock").unwrap_or(name)));
    serde_json::from_slice(&fs::read(entry).ok()?).ok()
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or_default() {
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

fn escape(s: &str) -> String {
    let mut out = String::new();
    comrak::html::escape(&mut out, s).expect("writing to a String cannot fail");
    out
}

pub fn signature(host: &str, session: Option<&Session>) -> [String; 2] {
    let line = |text: String, html: fn(String) -> String| (text.clone(), html(escape(&text)));
    let lines = match session {
        None => vec![line(
            format!("Sent from a shell on {host}, not from a Claude Code session."),
            |s| s,
        )],
        Some(s) => {
            let name = s.name.as_deref().unwrap_or("unnamed");
            let mut lines = vec![
                line(format!("Claude Code session \"{name}\" on {host}, in {}", s.cwd), |s| s),
                line(format!("claude --resume {}", s.session_id), |s| format!("<code>{s}</code>")),
            ];
            let link = |s| format!("<a href=\"{s}\">{s}</a>");
            lines.extend(
                s.bridge_session_id
                    .iter()
                    .map(|id| line(format!("https://claude.ai/code/{id}"), link)),
            );
            lines
        }
    };
    let (text, html): (Vec<_>, Vec<_>) = lines.into_iter().unzip();
    [format!("-- \n{}", text.join("\n")), format!("<p>-- <br>\n{}</p>\n", html.join("<br>\n"))]
}

pub fn body(markdown: &str, [text, html]: [String; 2]) -> Result<[String; 2]> {
    let mut options = Options::default();
    let e = &mut options.extension;
    (e.table, e.strikethrough, e.autolink, e.tasklist, options.render.escape) =
        (true, true, true, true, true);
    let page = format!(
        "<!DOCTYPE html>\n<html>\n<head><meta charset=\"utf-8\"></head>\n<body>\n{}{html}</body>\n</html>\n",
        markdown_to_html(markdown, &options)
    );
    let settings = STYLES.iter().fold(RewriteStrSettings::new(), |settings, &(selector, style)| {
        settings.append_element_content_handler(element!(selector, move |el| Ok(
            el.set_attribute("style", style)?
        )))
    });
    Ok([format!("{}\n\n{text}\n", markdown.trim_end()), rewrite_str(&page, settings)?])
}

pub fn newer<'a>(login: &Login, parent: &Email, thread: &'a [Email]) -> Vec<&'a Email> {
    let ours = |m: &Email| m.from.iter().any(|a| login.owns(&a.email));
    thread
        .iter()
        .filter(|m| m.id != parent.id && m.received >= parent.received && !ours(m))
        .collect()
}

pub fn reply_all(
    login: &Login,
    to: &[Address],
    cc: &[Address],
    parent: &Email,
) -> Result<(Vec<Address>, Vec<Address>)> {
    let others = |given: &[Address], on: Vec<&Addr>, without: &[Address]| {
        let theirs = on.into_iter().filter(|a| !login.owns(&a.email));
        let theirs = theirs.filter_map(|a| {
            a.email.parse().map_err(|e| note!("leaving out {}: {e:#}", plain(&a.email))).ok()
        });
        let mut kept: Vec<Address> = vec![];
        for a in given.iter().cloned().chain(theirs) {
            if !kept.contains(&a) && !without.contains(&a) {
                kept.push(a);
            }
        }
        kept
    };
    let mut all_to = others(to, parent.from.iter().collect(), &[]);
    let mut all_cc = others(cc, parent.to.iter().chain(&parent.cc).collect(), &all_to);
    if all_to.is_empty() && !all_cc.is_empty() {
        all_to.push(all_cc.remove(0));
    }
    ensure!(!all_to.is_empty(), "--all found nobody but us on the --reply mail");
    Ok((all_to, all_cc))
}

pub fn sender(login: &Login, topic: Option<&Topic>, parent: Option<&Email>) -> Result<Address> {
    let reached = parent.and_then(|p| {
        login.first_topic(p.from.iter().chain(&p.to).chain(&p.cc).map(|a| a.email.as_str()))
    });
    match (topic, reached) {
        (Some(topic), _) => login.address().topic(topic),
        (None, Some(address)) => Ok(address),
        (None, None) => login.bare_sender().context(
            "send needs --topic, or --reply to mail sent to one of the login's topic addresses",
        ),
    }
}

pub fn head(
    to: &Recipients,
    subject: Option<&str>,
    parent: Option<&Email>,
    date: i64,
    id: &str,
) -> Result<(String, Head)> {
    let subject = match (subject, parent) {
        (Some(subject), _) => plain(subject),
        (None, Some(p)) => match plain(&p.subject) {
            s if s.get(..3).is_some_and(|re| re.eq_ignore_ascii_case("re:")) => s,
            s => format!("Re: {s}"),
        },
        (None, None) => bail!("send needs --subject or --reply"),
    };
    let (ids, references) = match parent {
        Some(p) => (p.message_id.clone(), [&p.references[..], &p.message_id].concat()),
        None => Default::default(),
    };
    let head = Head::new(to, &subject, date, id)
        .ids("In-Reply-To", &MessageId::all(&ids))
        .ids("References", &MessageId::all(&references));
    Ok((subject, head.with("Disposition-Notification-To", to.from.to_string())))
}

pub async fn run(args: Args) -> Result<ExitCode> {
    let config = Config::load()?;
    let content = match &args.mime {
        Some(path) => {
            let bytes = fs::read(path).with_context(|| path.display().to_string())?;
            Content::Mime(
                Entity::parse(&bytes).with_context(|| format!("--mime {}", path.display()))?,
            )
        }
        None => {
            let markdown = io::read_to_string(io::stdin())?;
            ensure!(
                !markdown.trim().is_empty(),
                "send reads the body from stdin, and it was empty"
            );
            let attached = args.attach.iter().map(|path| {
                let name = path
                    .file_name()
                    .with_context(|| format!("{} names no file", path.display()))?;
                let bytes = fs::read(path).with_context(|| path.display().to_string())?;
                Ok((name.to_string_lossy().into_owned(), content_type(path), bytes))
            });
            let attached = attached.collect::<Result<_>>()?;
            let host = fs::read_to_string("/proc/sys/kernel/hostname");
            let host = host.map_or_else(|_| "unknown-host".into(), |h| h.trim().to_owned());
            Content::Markdown(body(&markdown, signature(&host, session().as_ref()))?, attached)
        }
    };

    let mail = Mail::with(config).await?;
    let (outbox, login) = (Outbox::open(&mail).await?, mail.login());
    let parent = reply_to(&mail, args.reply.as_deref(), args.past).await?;
    let from = sender(login, args.topic.as_ref(), parent.as_ref().map(|p| &p.1))?;
    let (to, cc) = match &parent {
        Some((_, p)) if args.all => reply_all(login, &args.to, &args.cc, p)?,
        _ => (args.to.clone(), args.cc.clone()),
    };
    let to = Recipients::check(login, from, to, cc)?;
    let reply = parent.as_ref().map(|p| &p.1);
    let (subject, head) = head(&to, args.subject.as_deref(), reply, now(), &token(24)?)?;
    let draft = match content {
        Content::Mime(entity) => outbox.import(&entity.message(&head), &outbox.drafts, DRAFT).await,
        Content::Markdown(rendered, attached) => {
            create(&mail, &outbox, &head, rendered, &attached).await
        }
    };
    let draft = draft.context("nothing was sent; the draft may or may not exist")?;
    let id = draft.as_str();
    let left = Leftover {
        unknown: format!("not known to be sent; the copy is still in Drafts as {id}"),
        removed: "not sent; the draft was removed".into(),
        unremoved: format!("not sent, and the draft {id} could not be removed"),
    };
    let (submission, delivery) = outbox.deliver(&draft, &to, left).await?;
    if let Err(e) = outbox.file(&draft).await {
        note!(
            "sent (submission {submission}), but filing {} into Sent failed: {e:#}",
            draft.as_str()
        );
    }
    match &delivery {
        Delivery::Accepted(refused) if !refused.is_empty() => {
            note!("submission {submission}: some recipients were refused\n{}", describe(refused))
        }
        Delivery::Unknown => {
            note!("submission {submission} was created, but its delivery status did not come back")
        }
        _ => {
            if let Some((answered, _)) = &parent
                && let Err(e) = mail.mark(&[(answered.clone(), &[Keyword::Answered][..])]).await
            {
                let id = answered.as_str();
                note!("sent, but marking {id} answered failed, so wait will remind of it: {e:#}");
            }
        }
    }
    let everyone: Vec<String> = to.everyone().map(Address::to_string).collect();
    say!(
        "sent {} \"{subject}\" from {} to {} (submission {submission})",
        draft.as_str(),
        &to.from,
        everyone.join(", ")
    )
}

async fn reply_to(mail: &Mail, id: Option<&str>, past: bool) -> Result<Option<(Current, Email)>> {
    let Some(id) = id else { return Ok(None) };
    let (current, parent): (_, Email) = mail.email(id).await?;
    let thread: Vec<Email> = mail.get(&mail.members(&[&parent.thread]).await?).await?;
    let newer = newer(mail.login(), &parent, &thread);
    if !newer.is_empty() && !past {
        let lines: Vec<String> = newer
            .iter()
            .map(|m| {
                format!("  {}  {}  {}: {}", m.id, m.received, emails(&m.from), preview(m, 100))
            })
            .collect();
        bail!(
            "the thread has newer mail than {id}; read `agent-mail thread {id}` and reply to the newest, or pass --past\n{}",
            lines.join("\n")
        );
    }
    Ok(Some((current, parent)))
}

async fn create(
    mail: &Mail,
    outbox: &Outbox<'_>,
    head: &Head,
    [text, html]: [String; 2],
    attached: &[(String, &str, Vec<u8>)],
) -> Result<Current> {
    let mut parts = vec![];
    for (name, kind, bytes) in attached {
        let blob = mail.upload(bytes, kind).await?;
        parts.push(
            json!({ "blobId": blob, "type": kind, "name": name, "disposition": "attachment" }),
        );
    }
    let mut email: Map<_, _> = head.properties().collect();
    email.extend([
        ("mailboxIds".into(), json!({ outbox.drafts.as_str(): true })),
        ("keywords".into(), keywords(DRAFT)),
        ("bodyValues".into(), json!({ "text": { "value": text }, "html": { "value": html } })),
        ("textBody".into(), json!([{ "partId": "text", "type": "text/plain" }])),
        ("htmlBody".into(), json!([{ "partId": "html", "type": "text/html" }])),
        ("attachments".into(), json!(parts)),
    ]);
    let answer = mail
        .set(false, json!({ "create": { "draft": email } }))
        .await?
        .context("stalwart refused the draft")?;
    Mail::created(&answer, "draft")
}

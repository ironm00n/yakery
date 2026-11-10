use std::{num::NonZeroUsize, ops::ControlFlow, process::ExitCode};

use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, SecondsFormat, TimeDelta};
use jiff::Timestamp;
use lol_html::{RewriteStrSettings, element, rewrite_str};

use private::{Dir, Mode, Replace};
use text::Address;

use super::succeed;
use crate::{
    SUBMISSION_KEY,
    address::{MessageId, Topic},
    auth::Auth,
    authority::{self, Signed},
    jmap::MAIL,
    mail::{Addr, Body, Email, Keyword, Mail, Page, Part, Text, names},
    out::is_receipt,
    policy::Login,
    prelude::*,
    show::{Answering, emails, inbox_line, message, out, plain, utc},
    store::{Config, passwd_home},
};

macro_rules! attachments {
    ($($dir:literal)/+, $each:literal MiB each, $total:literal MiB in all) => {
        const FOLDER: &[&str] = &[$($dir),+];
        const MAX_FILE: usize = $each << 20;
        const MAX_TOTAL: usize = $total << 20;
        pub const ATTACHMENTS_HELP: &str = concat!(
            "Save each attachment of an email, and each image its HTML embeds, into <passwd home>/",
            $($dir, "/",)+ "<email id>/ and print the absolute ",
            "path of each, one per line; read-only. Names are cut down to [A-Za-z0-9._-]. A file saved by an earlier run ",
            "is printed again, not replaced. Exits 1 if any attachment was skipped, over ", $each, " MiB each or ",
            $total, " MiB in all, or failed; each is named on stderr with why"
        );
    };
}

attachments!(".cache" / "agent-mail" / "attachments", 50 MiB each, 200 MiB in all);

::jmap::view! {
    under MAIL {
        Files in "Email/get" with ("fetchHTMLBodyValues", MAX_TOTAL) {
            attachments: Vec<Part> = "attachments", text: Vec<Part> = "textBody",
            html: Vec<Part> = "htmlBody", values: HashMap<String, Text> = "bodyValues",
        }
        Attached in "Email/get" { id: String = "id", attachments: Vec<Part> = "attachments" }
        Dated in "Email/get" { id: String = "id", received: String = "receivedAt" }
    }
}

pub enum Source<'a> {
    Part(&'a Part),
    Embedded(Result<(String, Vec<u8>)>),
}

impl Source<'_> {
    pub fn name(&self, n: usize) -> String {
        match self {
            Source::Part(part) => part.name.clone(),
            Source::Embedded(Ok((kind, _))) => {
                format!("embedded-{n}.{}", kind.split_once('/').map_or("bin", |k| k.1))
            }
            Source::Embedded(Err(_)) => String::new(),
        }
    }
}

/// The attachments and the media either body shows inline, each part once, since JMAP leaves out of
/// `attachments` what both bodies show (RFC 8621 §4.1.4); then the images the HTML carries as `data:` URIs.
pub fn sources(files: &Files) -> Vec<Source<'_>> {
    let media = files.text.iter().chain(&files.html).filter(|p| !p.kind.starts_with("text/"));
    let mut seen = HashSet::new();
    let parts = files.attachments.iter().chain(media).filter(|p| seen.insert(&p.part));
    let html = files.html.iter().filter_map(|p| files.values.get(&p.part));
    let embedded = html.flat_map(|value| match value.truncated {
        true => {
            vec![Err(anyhow!("its HTML runs past {MAX_TOTAL} bytes, so no image in it was read"))]
        }
        false => embedded(&value.value),
    });
    parts.map(Source::Part).chain(embedded.map(Source::Embedded)).collect()
}

pub fn embedded(html: &str) -> Vec<Result<(String, Vec<u8>)>> {
    let mut sources = vec![];
    let images = element!(r#"img[src^="data:"]"#, |el| {
        sources.extend(el.get_attribute("src"));
        Ok(())
    });
    let decode = |src: &String| {
        let uri = src.strip_prefix("data:").and_then(|rest| rest.split_once(','));
        let (meta, data) = uri.context("a data: URI with no comma")?;
        let kind = meta.strip_suffix(";base64").context("a data: URI that isn't base64")?;
        ensure!(kind.contains('/'), "a data: URI with no media type");
        let data: String = data.split_ascii_whitespace().collect();
        Ok((kind.to_owned(), STANDARD.decode(data).context("its base64")?))
    };
    match rewrite_str(html, RewriteStrSettings::new().append_element_content_handler(images)) {
        Ok(_) => sources.iter().map(decode).collect(),
        Err(e) => vec![Err(anyhow!(e).context("the HTML body"))],
    }
}

#[derive(clap::Args)]
pub struct Inbox {
    /// Keep only mail to or cc the subaddress <login>+<topic> itself, not <login>+<topic>-<more>.
    #[arg(long)]
    pub topic: Vec<Topic>,
    /// Keep only the newest this many [default: 10 without --after]
    #[arg(long)]
    pub limit: Option<NonZeroUsize>,
    /// List every mail received after this time (RFC 3339), not only the newest ten.
    #[arg(long, value_name = "TIME")]
    pub after: Option<Timestamp>,
    /// Print each mail as one JSON object on a line of its own, `signedByOwen` saying whether its signature makes it
    /// his, and his mail's addressing, subject, text and signing time read from the bytes he signed. Like the rest of
    /// inbox, it claims nothing and marks nothing seen, and it is no approval: that comes only through `wait`.
    #[arg(long)]
    pub json: bool,
}

const NEWEST: NonZeroUsize = NonZeroUsize::new(10).unwrap();

#[derive(clap::Args)]
pub struct Export {
    /// Only mail received at or after this time (RFC 3339).
    #[arg(long, value_name = "TIME")]
    pub after: Option<Timestamp>,
}

#[derive(clap::Args)]
pub struct One {
    #[arg(value_name = "EMAIL_ID")]
    id: String,
}

#[derive(clap::Args)]
pub struct Many {
    #[arg(value_name = "EMAIL_ID", required = true)]
    ids: Vec<String>,
}

#[derive(clap::Args)]
pub struct Nothing {}

#[derive(clap::Args)]
pub struct Password {
    #[command(subcommand)]
    action: Rotation,
}

#[derive(clap::Subcommand)]
enum Rotation {
    /// Replace the password with a random one and store it in the password file.
    Rotate,
}

pub async fn inbox(args: Inbox) -> Result<ExitCode> {
    for line in lines(&Mail::open().await?, &args).await? {
        out(&line)?;
    }
    Ok(ExitCode::SUCCESS)
}

/// What `inbox` prints, a line per mail; nothing when a message it reads fails to download.
pub async fn lines(mail: &Mail, args: &Inbox) -> Result<Vec<String>> {
    let listed = match args.after {
        Some(after) => received_after(mail, &args.topic, after, args.limit).await?,
        None => newest(mail, &args.topic, args.limit.unwrap_or(NEWEST)).await?,
    };
    if args.json {
        let mut lines = vec![];
        for m in &listed {
            let signature = match mail.message(m).await? {
                Ok(raw) => authority::signed(SUBMISSION_KEY, &raw)
                    .map_or_else(Signature::NotHis, Signature::His),
                Err(why) => Signature::Unchecked(why),
            };
            lines.push(listing(m, &signature, mail.login()));
        }
        return Ok(lines);
    }
    let files: Vec<Attached> = mail.get(&listed.iter().map(|m| &m.id).collect::<Vec<_>>()).await?;
    let parts = |m: &Email| files.iter().find(|f| f.id == m.id).map_or(&[][..], |f| &f.attachments);
    Ok(listed.iter().map(|m| inbox_line(m, parts(m))).collect())
}

/// Every mail in the inbox and Sent, oldest first, read receipts left out, each one JSON object on a line: `id`,
/// `thread`, `received`, `from`, `to`, `cc`, `subject` and `text` (its text parts joined), read from headers as they
/// stand, so it says nothing of who signed it.
pub async fn export(args: Export) -> Result<ExitCode> {
    for line in exported(&Mail::open().await?, args.after).await? {
        out(&line)?;
    }
    Ok(ExitCode::SUCCESS)
}

/// What `export` prints, a line per mail.
pub async fn exported(mail: &Mail, after: Option<Timestamp>) -> Result<Vec<String>> {
    let boxes =
        ["inbox", "sent"].map(|role| mail.mailbox(role).map(|id| json!({ "inMailbox": id })));
    let mut filter =
        json!({ "operator": "OR", "conditions": boxes.into_iter().collect::<Result<Vec<_>>>()? });
    if let Some(at) = after {
        // Stalwart's `after` is strictly later, in whole seconds, so the given second needs one earlier
        let before = at - jiff::SignedDuration::from_secs(1);
        filter =
            json!({ "operator": "AND", "conditions": [filter, { "after": before.to_string() }] });
    }
    let mut mails: Vec<Email> = mail.get(&mail.query(filter).await?).await?;
    mails.retain(|m| !is_receipt(m));
    mails.sort_by(|a, b| a.received.cmp(&b.received));
    let bodies: Vec<Body> = mail.get(&mails.iter().map(|m| &m.id).collect::<Vec<_>>()).await?;
    let addresses = |list: &[Addr]| list.iter().map(|a| a.email.clone()).collect::<Vec<_>>();
    let line = |m: &Email| {
        let body = bodies.iter().find(|b| b.id == m.id);
        let parts = body.iter().flat_map(|b| b.text.iter().filter_map(|p| b.values.get(&p.part)));
        let text = parts.map(|v| v.value.as_str()).collect::<Vec<_>>().join("\n");
        json!({
            "id": m.id, "thread": m.thread, "received": m.received, "from": addresses(&m.from),
            "to": addresses(&m.to), "cc": addresses(&m.cc), "subject": m.subject, "text": text,
        })
        .to_string()
    };
    Ok(mails.iter().map(line).collect())
}

/// What the listing learned of a mail's signature.
pub enum Signature {
    His(Signed),
    NotHis(String),
    /// The message is too large to read whole, so whose it is wasn't checked
    Unchecked(String),
}

/// One inbox mail as one JSON line; when Owen signed it, its addressing, subject and text are what he signed.
/// `signedByOwen` (null when unchecked) says who wrote it and nothing more: it skips the wait's age and topic rules,
/// so it approves nothing, and the signature, which approvals are known by, isn't printed. His word to a session
/// comes only through `wait` ([`authority::check`]). `id`, `threadId` and `receivedAt` are the mailbox copy's, which
/// an import can set; `signedId` (a hash of the signature) and `signedAt` are the signed message's own. `toLogin` says
/// whether its To or Cc names `login` or any subaddress of it, and `topics` lists the topics of those that are topic
/// subaddresses, in lower case, as addresses compare.
pub fn listing(m: &Email, signature: &Signature, login: &Login) -> String {
    let addresses = |l: &[Addr]| l.iter().map(|a| a.email.clone()).collect::<Vec<_>>();
    let his = match signature {
        Signature::His(s) => Some(s),
        _ => None,
    };
    let (from, to, cc, subject) = match his {
        Some(s) => (s.from.clone(), s.to.clone(), s.cc.clone(), s.subject.clone()),
        None => (emails(&m.from), addresses(&m.to), addresses(&m.cc), m.subject.clone()),
    };
    let (owen, reason) = match signature {
        Signature::His(_) => (Some(true), None),
        Signature::NotHis(why) => (Some(false), Some(why)),
        Signature::Unchecked(why) => (None, Some(why)),
    };
    let id = |s: &Signed| {
        let digest = ring::digest::digest(&ring::digest::SHA256, s.signature.as_bytes());
        digest.as_ref().iter().map(|b| format!("{b:02x}")).collect::<String>()
    };
    let ours: Vec<&String> = to.iter().chain(&cc).filter(|a| login.reaches(a)).collect();
    let topics: BTreeSet<String> = ours
        .iter()
        .filter_map(|a| Some(login.topic(a)?.to_string().to_ascii_lowercase()))
        .collect();
    json!({
        "id": m.id, "threadId": m.thread, "receivedAt": m.received, "from": from, "to": to, "cc": cc,
        "toLogin": !ours.is_empty(), "topics": topics, "subject": subject, "signedByOwen": owen,
        "text": his.map(|s| &s.text), "signedAt": his.map(|s| s.at), "signedId": his.map(id), "reason": reason,
    })
    .to_string()
}

fn subaddresses(mail: &Mail, topics: &[Topic]) -> Result<Vec<Address>> {
    topics.iter().map(|t| mail.login().address().subaddress(t)).collect()
}

/// Every inbox mail received after `after`, newest first and at most `limit`, only that to or cc one of `topics` when
/// any is given. The dates are compared here, as [`names`] compares addresses: Stalwart answers a sort from its
/// full-text index, which lags ingest, and may answer a date filter the same way.
pub async fn received_after(
    mail: &Mail,
    topics: &[Topic],
    after: Timestamp,
    limit: Option<NonZeroUsize>,
) -> Result<Vec<Email>> {
    let topics = subaddresses(mail, topics)?;
    let dated: Vec<Dated> =
        mail.get(&mail.query(json!({ "inMailbox": mail.mailbox("inbox")? })).await?).await?;
    let late = |d: &Dated| !d.received.parse::<Timestamp>().is_ok_and(|t| t <= after);
    let ids: Vec<String> = dated.into_iter().filter(late).map(|d| d.id).collect();
    let mut found: Vec<Email> = mail.get(&ids).await?;
    found.retain(|m| topics.is_empty() || names(&topics, m.to.iter().chain(&m.cc)));
    found.sort_by(|a, b| b.received.cmp(&a.received));
    found.truncate(limit.map_or(usize::MAX, NonZeroUsize::get));
    Ok(found)
}

/// The newest `limit` inbox mail, only that to or cc one of `topics` when any is given. The server's filter is a first
/// cut, so it is paged until `limit` pass.
pub async fn newest(mail: &Mail, topics: &[Topic], limit: NonZeroUsize) -> Result<Vec<Email>> {
    let topics = subaddresses(mail, topics)?;
    let mut filter = json!({ "inMailbox": mail.mailbox("inbox")? });
    let addressed: Vec<_> = topics
        .iter()
        .map(Address::to_string)
        .flat_map(|a| [json!({ "to": a }), json!({ "cc": a })])
        .collect();
    if !addressed.is_empty() {
        let any = json!({ "operator": "OR", "conditions": addressed });
        filter = json!({ "operator": "AND", "conditions": [filter, any] });
    }
    let sort = [json!({ "property": "receivedAt", "isAscending": false })];
    let mut kept = vec![];
    let query = json!({ "filter": filter, "sort": sort, "limit": limit });
    mail.pages(query, async |page: Page| {
        if page.first {
            kept.clear();
        }
        let found: Vec<Email> = mail.get(&page.ids).await?;
        kept.extend(
            found
                .into_iter()
                .filter(|m| topics.is_empty() || names(&topics, m.to.iter().chain(&m.cc))),
        );
        Ok(if kept.len() >= limit.get() {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        })
    })
    .await?;
    kept.truncate(limit.get());
    Ok(kept)
}

pub fn place<'a>(
    mails: &[Email],
    receipts: &[(&'a Email, Vec<MessageId>)],
) -> (Vec<Vec<&'a Email>>, Vec<&'a Email>) {
    let (mut placed, mut unplaced) = (vec![vec![]; mails.len()], vec![]);
    for (receipt, answers) in receipts {
        match mails
            .iter()
            .position(|m| MessageId::all(&m.message_id).iter().any(|own| answers.contains(own)))
        {
            Some(i) => placed[i].push(*receipt),
            None => unplaced.push(*receipt),
        }
    }
    (placed, unplaced)
}

pub fn read_by(receipt: &Email) -> String {
    format!("=== read by {} at {}", emails(&receipt.from), plain(&receipt.received))
}

pub fn original_message_id(report: &str) -> Option<MessageId> {
    report.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("Original-Message-ID")
            .then(|| value.trim().parse().ok())?
    })
}

pub fn base_subject(subject: &str) -> &str {
    let mut rest = subject.trim();
    while let Some((prefix, tail)) = rest.split_once(':')
        && ["re", "fwd", "fw", "aw"].iter().any(|p| prefix.trim().eq_ignore_ascii_case(p))
    {
        rest = tail.trim();
    }
    rest
}

pub async fn thread(args: One) -> Result<ExitCode> {
    let mail = Mail::open().await?;
    let (_, start): (_, Email) = mail.email(&args.id).await?;
    let ids = mail.members(&[&start.thread]).await?;
    let mut members: Vec<Email> = mail.get(&ids).await?;
    members.sort_by(|a, b| a.received.cmp(&b.received));
    let (mut receipts, mails): (Vec<Email>, Vec<Email>) = members.into_iter().partition(is_receipt);
    // Stalwart threads a receipt apart from the mail it answers ("Read: ..."), so they are looked for by subject too.
    let subjects: HashSet<&str> =
        mails.iter().map(|m| base_subject(&m.subject)).filter(|s| !s.is_empty()).collect();
    if let (Some(since), false) = (mails.first(), subjects.is_empty()) {
        let by_subject: Vec<_> = subjects.iter().map(|s| json!({ "subject": s })).collect();
        let any = json!({ "operator": "OR", "conditions": by_subject });
        // Stalwart's `after` is strictly later, in whole seconds, so a receipt in the mail's own second needs one earlier
        let after = DateTime::parse_from_rfc3339(&since.received)
            .map_or(since.received.clone(), |t| {
                (t - TimeDelta::seconds(1)).to_rfc3339_opts(SecondsFormat::Secs, true)
            });
        let filter = json!({ "operator": "AND", "conditions": [{ "after": after }, any] });
        let outside: Vec<String> =
            mail.query(filter).await?.into_iter().filter(|i| !ids.contains(i)).collect();
        receipts.extend(mail.get::<Email>(&outside).await?.into_iter().filter(is_receipt));
    }
    receipts.sort_by(|a, b| a.received.cmp(&b.received));
    let shown: Vec<&String> = mails.iter().chain(&receipts).map(|m| &m.id).collect();
    let bodies: Vec<Body> = mail.get(&shown).await?;
    let body = |m: &Email| bodies.iter().find(|b| b.id == m.id);
    let mut answering = vec![];
    for receipt in &receipts {
        let mut answers = MessageId::all(&receipt.in_reply_to);
        let parts = body(receipt).map(|b| b.structure.parts.as_slice()).unwrap_or_default();
        let report =
            parts.iter().find(|p| p.kind.eq_ignore_ascii_case("message/disposition-notification"));
        if let Some(Part { blob, .. }) =
            report.filter(|p| p.size <= 16 * 1024 && !p.blob.is_empty())
            && let Some(bytes) = mail.download(blob.as_str(), 16 * 1024).await?
        {
            answers.extend(original_message_id(&String::from_utf8_lossy(&bytes)));
        }
        answering.push((receipt, answers));
    }
    let (placed, unplaced) = place(&mails, &answering);
    for (m, rs) in mails.iter().zip(placed) {
        let receipts: String = rs.into_iter().map(|r| format!("\n{}", read_by(r))).collect();
        let answering = Answering::of(m, &m.in_reply_to, &mails);
        out(&format!("{}{receipts}\n", message(m, body(m), answering)))?;
    }
    for r in unplaced.into_iter().filter(|r| ids.contains(&r.id)) {
        out(&format!("{} (for a mail not in this thread)", read_by(r)))?;
    }
    Ok(ExitCode::SUCCESS)
}

pub fn file_name(taken: &mut HashSet<String>, raw: &str, n: usize) -> String {
    let mut name = String::new();
    for c in raw.rsplit(['/', '\\']).next().unwrap_or_default().chars() {
        let c = if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' };
        if !(c == '_' && name.ends_with('_')) {
            name.push(c);
        }
    }
    let name = name.trim_start_matches(['.', '-']);
    let split = |name: &str| match name.rfind('.') {
        Some(i) if i > 0 && name.len() - i <= 16 => (name[..i].to_owned(), name[i..].to_owned()),
        _ => (name.to_owned(), String::new()),
    };
    let base = match (name.bytes().any(|b| b.is_ascii_alphanumeric()), split(name)) {
        (false, _) => format!("attachment-{n}"),
        (true, (stem, extension)) if name.len() > 200 => {
            format!("{}{extension}", &stem[..200 - extension.len()])
        }
        (true, _) => name.to_owned(),
    };
    let (stem, extension) = split(&base);
    let name = std::iter::once(base).chain((2..).map(|k| format!("{stem}-{k}{extension}")));
    let name = name
        .into_iter()
        .find(|name| !taken.contains(&name.to_ascii_lowercase()))
        .expect("the suffixes are endless");
    taken.insert(name.to_ascii_lowercase());
    name
}

pub fn folder(home: &Path, email: &str) -> Result<Dir> {
    let dir = Dir::open(home, &[FOLDER, &[email]].concat())?;
    dir.drop_stale_partials()?;
    Ok(dir)
}

pub async fn attachments(args: One) -> Result<ExitCode> {
    let mail = Mail::open().await?;
    let (current, files) = mail.email(&args.id).await?;
    let sources = sources(&files);
    ensure!(!sources.is_empty(), "{} has no attachments", plain(&args.id));
    let dir = folder(&passwd_home()?, current.as_str()).context("the attachments folder")?;
    let (mut taken, mut left, mut failed, count) = (HashSet::new(), MAX_TOTAL, 0, sources.len());
    for (n, source) in sources.into_iter().enumerate() {
        let name = file_name(&mut taken, &source.name(n + 1), n + 1);
        let limit = |bytes: usize| match bytes < MAX_FILE {
            true => format!("the {bytes} bytes left of the {MAX_TOTAL}-byte total"),
            false => format!("the {MAX_FILE}-byte limit"),
        };
        let saved: Result<Option<usize>> = async {
            if dir.existing(&name)?.is_some() {
                return Ok(None);
            }
            let most = MAX_FILE.min(left);
            let bytes = match source {
                Source::Part(part) => {
                    ensure!(!part.blob.is_empty(), "it has no blob");
                    ensure!(part.size <= most, "{} bytes, over {}", part.size, limit(most));
                    let bytes = mail.download(&part.blob, most).await?;
                    bytes.with_context(|| format!("its download ran past {}", limit(most)))?
                }
                Source::Embedded(image) => image?.1,
            };
            ensure!(bytes.len() <= most, "{} bytes, over {}", bytes.len(), limit(most));
            Ok(dir.write(&name, &bytes, Mode::Private, Replace::No)?.then_some(bytes.len()))
        }
        .await;
        match saved {
            Ok(saved) => {
                if saved.is_none() {
                    note!("{name} was saved already");
                }
                left -= saved.unwrap_or(0);
                out(&dir.path(&name).display().to_string())?;
            }
            Err(e) => {
                warn!("skipped {name}: {e:#}");
                failed += 1;
            }
        }
    }
    if failed > 0 {
        bail!("skipped {failed} of {count} attachments");
    }
    Ok(ExitCode::SUCCESS)
}

pub async fn done(args: Many) -> Result<ExitCode> {
    let mail = Mail::open().await?;
    let (mut found, mut missing) = (BTreeSet::new(), vec![]);
    for id in &args.ids {
        match mail.current(id).await? {
            Some(current) => found.extend([current]),
            None => missing.push(id.as_str()),
        }
    }
    ensure!(missing.is_empty(), "no email {}", missing.join(", "));
    mail.mark(&found.into_iter().map(|id| (id, &[Keyword::Handled][..])).collect::<Vec<_>>())
        .await?;
    succeed!("handled {}", args.ids.join(", "))
}

pub async fn login(_: Nothing) -> Result<ExitCode> {
    let auth = Auth::new(Config::load()?)?;
    let access = auth.access().await?;
    let expiry =
        access.expires_at.map_or("names no expiry".into(), |at| format!("expires at {}", utc(at)));
    succeed!("signed in as {}; the access token {expiry}", auth.login)
}

pub async fn rotate(_: Password) -> Result<ExitCode> {
    let auth = Auth::new(Config::load()?)?;
    auth.rotate_password().await?;
    succeed!("rotated the password of {}", auth.login)
}

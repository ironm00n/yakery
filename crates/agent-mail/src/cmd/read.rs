use std::process::ExitCode;

use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, SecondsFormat, TimeDelta};
use lol_html::{RewriteStrSettings, element, rewrite_str};

use super::say;
use crate::{
    address::{MessageId, Topic},
    auth::Auth,
    jmap::wire,
    mail::{Body, Email, Keyword, Mail, Part, QUERY, Text},
    out::is_receipt,
    prelude::*,
    private::{Dir, home},
    show::{emails, inbox_line, message, out, plain, utc},
    store::Config,
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

wire! {
    views {
        Files in "Email/get" with ("fetchHTMLBodyValues", MAX_TOTAL) {
            attachments: Vec<Part> = "attachments", text: Vec<Part> = "textBody",
            html: Vec<Part> = "htmlBody", values: HashMap<String, Text> = "bodyValues",
        }
    }
    objects {}
}

/// What a file is saved from: a part of the mail, or an image its HTML carries itself, as a media type and bytes.
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

/// Each `<img>` with a `data:` source, in document order; one that isn't base64 media is an error in its place.
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
    /// Keep only mail addressed to the subaddress <login>+<topic>.
    #[arg(long)]
    topic: Vec<Topic>,
    #[arg(long, default_value_t = 10)]
    limit: usize,
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
    let mail = Mail::open().await?;
    let mut filter = json!({ "inMailbox": mail.mailbox("inbox")? });
    let topics = args.topic.iter().map(|t| mail.login().address().topic(t));
    let topics = topics.map(|a| a.map(|a| a.to_string())).collect::<Result<Vec<_>>>()?;
    let addressed: Vec<_> =
        topics.iter().flat_map(|a| [json!({ "to": a }), json!({ "cc": a })]).collect();
    if !addressed.is_empty() {
        let any = json!({ "operator": "OR", "conditions": addressed });
        filter = json!({ "operator": "AND", "conditions": [filter, any] });
    }
    let sort = [json!({ "property": "receivedAt", "isAscending": false })];
    let found =
        mail.call(QUERY, json!({ "filter": filter, "sort": sort, "limit": args.limit })).await?;
    for m in mail.get::<Email>(&found.ids).await? {
        out(&inbox_line(&m))?;
    }
    Ok(ExitCode::SUCCESS)
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
        out(&format!("{}{receipts}\n", message(m, body(m))))?;
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
    let dir = folder(&home(true)?, current.as_str()).context("the attachments folder")?;
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
            Ok(dir.write(&name, &bytes, false)?.then_some(bytes.len()))
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
                note!("skipped {name}: {e:#}");
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
    say!("handled {}", args.ids.join(", "))
}

pub async fn login(_: Nothing) -> Result<ExitCode> {
    let auth = Auth::new(Config::load()?)?;
    let access = auth.access().await?;
    say!("signed in as {}; the access token expires at {}", auth.login, utc(access.expires_at))
}

pub async fn rotate(_: Password) -> Result<ExitCode> {
    let auth = Auth::new(Config::load()?)?;
    auth.rotate_password().await?;
    say!("rotated the password of {}", auth.login)
}

use private::{now, random};
use text::Address;

use crate::{
    address::MessageId,
    mail::{
        Current, Email, IMPORT, Identity, Keyword, Mail, SUBMISSIONS, SUBMIT, Status, Submission,
        keywords,
    },
    policy::{Login, Recipients},
    prelude::*,
    show::plain,
};

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn fresh_id() -> Result<String> {
    random::chars(random::ALPHANUMERIC, 24)
}

pub fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        out.extend((0..4).map(|i| {
            if i <= chunk.len() {
                char::from(BASE64[(n >> (18 - 6 * i) & 63) as usize])
            } else {
                '='
            }
        }));
    }
    out
}

pub fn base64_body(bytes: &[u8]) -> String {
    let encoded = base64(bytes);
    encoded
        .as_bytes()
        .chunks(76)
        .map(|l| String::from_utf8_lossy(l))
        .collect::<Vec<_>>()
        .join("\r\n")
}

pub fn text(value: &str) -> String {
    let mut clean = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\x00'..='\x1f' | '\x7f' if clean.ends_with(' ') => {}
            '\x00'..='\x1f' | '\x7f' => clean.push(' '),
            c => clean.push(c),
        }
    }
    let clean = clean.trim();
    if clean.is_ascii() {
        return clean.to_owned();
    }
    let mut words = vec![String::new()];
    for c in clean.chars() {
        if words.last().is_some_and(|w| w.len() + c.len_utf8() > (75 - 12) / 4 * 3) {
            words.push(String::new());
        }
        words.last_mut().expect("words starts non-empty").push(c);
    }
    words
        .iter()
        .map(|w| format!("=?UTF-8?B?{}?=", base64(w.as_bytes())))
        .collect::<Vec<_>>()
        .join("\r\n ")
}

fn list<T: ToString>(items: &[T], by: &str) -> String {
    items.iter().map(T::to_string).collect::<Vec<_>>().join(by)
}

pub struct Head(Vec<(&'static str, String)>);

impl Head {
    pub fn new(to: &Recipients, subject: &str, date: i64, id: &str) -> Head {
        let date = chrono::DateTime::from_timestamp(date, 0).unwrap_or_default().to_rfc2822();
        let mut head =
            Head(vec![("Date", date), ("From", to.from.to_string()), ("To", list(&to.to, ", "))]);
        if !to.cc.is_empty() {
            head.0.push(("Cc", list(&to.cc, ", ")));
        }
        head.with("Subject", text(subject))
            .with("Message-ID", format!("<{id}@{}>", to.from.domain()))
    }

    pub fn with(mut self, name: &'static str, value: String) -> Head {
        self.0.push((name, value));
        self
    }

    pub fn ids(self, name: &'static str, ids: &[MessageId]) -> Head {
        if ids.is_empty() { self } else { self.with(name, list(ids, " ")) }
    }

    pub fn raw(&self) -> String {
        self.0.iter().map(|(name, value)| format!("{name}: {value}\r\n")).collect()
    }

    pub fn properties(&self) -> impl Iterator<Item = (String, Value)> {
        self.0.iter().map(|(name, value)| (format!("header:{name}:asRaw"), json!(value)))
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

pub struct Entity(Vec<u8>, Vec<u8>);

impl Entity {
    pub fn parse(bytes: &[u8]) -> Result<Entity> {
        let mut crlf = Vec::with_capacity(bytes.len() + bytes.len() / 32);
        for (i, &b) in bytes.iter().enumerate() {
            if b == b'\n' && (i == 0 || bytes[i - 1] != b'\r') {
                crlf.push(b'\r');
            }
            crlf.push(b);
        }
        let end = match crlf.starts_with(b"\r\n") {
            true => 0,
            false => find(&crlf, b"\r\n\r\n").context("no blank line ends its header fields")? + 2,
        };
        let header = std::str::from_utf8(&crlf[..end]).ok().filter(|h| h.is_ascii());
        let header = header.context("its header fields are not ASCII")?;
        ensure!(!header.replace("\r\n", "").contains('\r'), "its header fields hold a bare CR");
        let mut lines: Vec<String> = vec![];
        for line in header.split_inclusive("\r\n") {
            match lines.last_mut() {
                Some(field) if line.starts_with([' ', '\t']) => field.push_str(line),
                None if line.starts_with([' ', '\t']) => {
                    bail!("its first header line is a continuation")
                }
                _ => lines.push(line.into()),
            }
        }
        let (mut fields, mut types) = (vec![], 0);
        for field in &lines {
            let (name, value) = field.split_once(':').with_context(|| {
                format!("header line {} has no colon", field.trim_end().escape_debug())
            })?;
            ensure!(
                !name.is_empty() && name.bytes().all(|b| (b'!'..=b'~').contains(&b)),
                "{} is not a header field name",
                name.escape_debug()
            );
            match name.to_ascii_lowercase().as_str() {
                "mime-version" => {
                    ensure!(value.trim() == "1.0", "MIME-Version {} is not 1.0", value.trim())
                }
                lower if lower.starts_with("content-") => {
                    types += usize::from(lower == "content-type");
                    fields.extend_from_slice(field.as_bytes());
                }
                _ => bail!(
                    "it sets {name}, which only agent-mail writes; an entity carries Content-* fields and MIME-Version"
                ),
            }
        }
        match types {
            1 => Ok(Entity(fields, crlf[end + 2..].to_vec())),
            0 => bail!("it has no Content-Type"),
            n => bail!("it has {n} Content-Type fields"),
        }
    }

    pub fn message(&self, head: &Head) -> Vec<u8> {
        [head.raw().as_bytes(), b"MIME-Version: 1.0\r\n", &self.0, b"\r\n", &self.1].concat()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Delivery {
    Accepted(Vec<(String, String)>),
    Refused(Vec<(String, String)>),
    Unknown,
}

impl Delivery {
    pub fn of(statuses: Option<&HashMap<String, Status>>) -> Delivery {
        let Some(statuses) = statuses else { return Delivery::Unknown };
        let mut refused: Vec<_> = statuses
            .iter()
            .filter(|(_, s)| s.delivered == "no")
            .map(|(a, s)| (a.clone(), s.reply.trim().to_owned()))
            .collect();
        refused.sort();
        if refused.len() == statuses.len() && !statuses.is_empty() {
            Delivery::Refused(refused)
        } else {
            Delivery::Accepted(refused)
        }
    }
}

pub fn describe(refused: &[(String, String)]) -> String {
    refused
        .iter()
        .map(|(address, reply)| format!("  {address}: {reply}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub type Submitted = (String, Delivery);

pub struct Leftover {
    pub unknown: String,
    pub removed: String,
    pub unremoved: String,
}

pub struct Outbox<'a> {
    mail: &'a Mail,
    identity: String,
    /// Stalwart refuses any other envelope sender for this identity.
    sender: String,
    pub drafts: String,
    pub sent: String,
}

impl<'a> Outbox<'a> {
    pub async fn open(mail: &'a Mail) -> Result<Outbox<'a>> {
        let login = mail.login().to_string();
        let identities = mail.all::<Identity>().await?;
        let identity = identities
            .iter()
            .find(|i| i.email.eq_ignore_ascii_case(&login))
            .with_context(|| format!("no identity for {login}"))?;
        let (identity, sender) = (identity.id.clone(), identity.email.clone());
        let (drafts, sent) = (mail.mailbox("drafts")?, mail.mailbox("sent")?);
        Ok(Outbox { mail, identity, sender, drafts, sent })
    }

    pub async fn deliver(
        &self,
        email: &Current,
        to: &Recipients,
        left: Leftover,
    ) -> Result<Submitted> {
        let submitted = match self.submit(email, to).await {
            Err(e) if Failure::of(&e).nothing_ran() => Err(e),
            Err(e) => match self.find(email).await {
                Ok(Some(found)) => Ok(found),
                Ok(None) => return Err(e.context(left.unknown)),
                Err(f) => {
                    let e = e.context(format!("looking for its submission failed: {f:#}"));
                    return Err(e.context(left.unknown));
                }
            },
            submitted => submitted,
        };
        let why = match submitted {
            Ok((id, Delivery::Refused(refused))) => anyhow!(
                "submission {id}: every recipient was refused, nothing was sent\n{}",
                describe(&refused)
            ),
            Ok(delivered) => return Ok(delivered),
            Err(e) => e,
        };
        Err(match self.mail.set(false, json!({ "destroy": [email] })).await {
            Ok(_) => why.context(left.removed),
            Err(d) => why.context(format!("{}: {d:#}", left.unremoved)),
        })
    }

    async fn submit(&self, email: &Current, to: &Recipients) -> Result<Submitted> {
        let rcpt_to: Vec<Value> =
            to.everyone().map(|a| json!({ "email": a.to_string() })).collect();
        let envelope = json!({ "mailFrom": { "email": self.sender }, "rcptTo": rcpt_to });
        let send = json!({ "emailId": email, "identityId": self.identity, "envelope": envelope });
        let create = json!({ "send": send });
        let answer = self.mail.call(SUBMIT, json!({ "create": create })).await?;
        let id = Mail::created(&answer, "send")
            .map_err(|e| Failure::Unprocessed.with(format!("{e:#}")))?;
        Ok((id.as_str().into(), self.status(id.as_str()).await))
    }

    async fn status(&self, submission: &str) -> Delivery {
        let found = self.mail.get::<Submission>(&[submission]).await;
        found.map_or(Delivery::Unknown, |found| {
            Delivery::of(found.first().and_then(|s| s.delivery.as_ref()))
        })
    }

    async fn find(&self, email: &Current) -> Result<Option<Submitted>> {
        let filter = json!({ "filter": { "emailIds": [email] } });
        let Some(id) = self.mail.call(SUBMISSIONS, filter).await?.ids.first().cloned() else {
            return Ok(None);
        };
        Ok(Some((id.clone(), self.status(&id).await)))
    }

    pub async fn file(&self, draft: &Current) -> Result<()> {
        let moves = [(&self.drafts, Value::Null), (&self.sent, json!(true))];
        let mut patch: Map<String, Value> =
            moves.into_iter().map(|(m, v)| (format!("mailboxIds/{m}"), v)).collect();
        patch.insert(format!("keywords/{}", Keyword::Draft.name()), Value::Null);
        self.mail.set(false, json!({ "update": { draft.as_str(): patch } })).await.map(drop)
    }

    pub async fn import(&self, message: &[u8], mailbox: &str, set: &[Keyword]) -> Result<Current> {
        let blob = self.mail.upload(message, "message/rfc822").await?;
        let email =
            json!({ "blobId": blob, "mailboxIds": { mailbox: true }, "keywords": keywords(set) });
        Mail::created(&self.mail.call(IMPORT, json!({ "emails": { "i0": email } })).await?, "i0")
    }
}

pub fn is_receipt(m: &Email) -> bool {
    let kind = m.content_type.trim().to_ascii_lowercase();
    kind.starts_with("multipart/report") && kind.contains("disposition-notification")
}

pub enum Asked {
    Nothing,
    Declined(String),
    Send(Receipt),
}

pub struct Receipt {
    pub to: Recipients,
    pub subject: String,
    pub original_recipient: Option<Address>,
    pub original_message_id: Option<MessageId>,
}

impl Asked {
    /// Only ever to the sender: RFC 8098's Return-Path rule can't apply, since Stalwart adds none to mail it delivers
    /// locally, and this keeps a third party from having us mail someone else.
    pub fn of(login: &Login, m: &Email) -> Asked {
        let asked: Vec<&str> = m.notify.iter().map(|a| a.email.as_str()).collect();
        let Some(to) = asked.first() else { return Asked::Nothing };
        if m.has(Keyword::MdnSent) {
            return Asked::Nothing;
        }
        let why = match receipt(login, m, to, &asked) {
            Ok(receipt) => return Asked::Send(receipt),
            Err(why) => why,
        };
        Asked::Declined(plain(&format!("no receipt for {}: {why:#}", m.id)))
    }
}

fn receipt(login: &Login, m: &Email, to: &str, asked: &[&str]) -> Result<Receipt> {
    ensure!(!is_receipt(m), "it is itself a receipt");
    let requester: Address = to.parse().map_err(|_| anyhow!("{to} is not a plain address"))?;
    ensure!(
        asked.iter().all(|a| a.parse().ok() == Some(requester.clone())),
        "it asks for receipts to several addresses ({})",
        asked.join(", ")
    );
    let senders: Vec<&str> = m.from.iter().map(|a| a.email.as_str()).collect();
    let single =
        matches!(senders.as_slice(), [sender] if sender.parse().ok() == Some(requester.clone()));
    ensure!(single, "it asks for a receipt to {to}, not to its sender {}", senders.join(", "));
    let reached = login.first_topic(m.to.iter().chain(&m.cc).map(|a| a.email.as_str()));
    let header = m.original_recipient.split_once(';').and_then(|(_, a)| a.trim().parse().ok());
    let original_recipient = header.or_else(|| reached.clone());
    let from = match reached {
        Some(address) => address,
        None => login.bare_sender()?,
    };
    let to = Recipients::check(login, from, vec![requester], vec![])?;
    Ok(Receipt {
        to,
        subject: m.subject.clone(),
        original_recipient,
        original_message_id: MessageId::all(&m.message_id).into_iter().next(),
    })
}

impl Receipt {
    pub fn message(&self, date: i64, token: &str) -> Vec<u8> {
        let (from, boundary) = (&self.to.from, format!("----=_MDN_{token}"));
        let content_type = format!(
            "multipart/report; report-type=disposition-notification;\r\n\tboundary=\"{boundary}\""
        );
        let replying =
            Head::new(&self.to, &format!("Read: {}", self.subject), date, &format!("mdn.{token}"));
        let head = replying
            .ids("In-Reply-To", self.original_message_id.as_slice())
            .with("MIME-Version", "1.0".into());
        let human = format!(
            "This is a return receipt for the message you sent to {from}.\r\n\r\nNote: This receipt only acknowledges that agent-mail displayed the message\r\nto an agent. There is no guarantee that it was read or understood."
        );
        let mut report = vec![
            format!("Reporting-UA: {}; agent-mail", from.domain()),
            format!("Final-Recipient: rfc822;{from}"),
        ];
        report.extend(
            self.original_recipient.iter().map(|a| format!("Original-Recipient: rfc822;{a}")),
        );
        report
            .extend(self.original_message_id.iter().map(|id| format!("Original-Message-ID: {id}")));
        report.push("Disposition: automatic-action/MDN-sent-automatically; displayed".into());
        let parts = [
            head.with("Content-Type", content_type).raw(),
            format!(
                "--{boundary}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{}\r\n",
                base64_body(human.as_bytes())
            ),
            format!(
                "--{boundary}\r\nContent-Type: message/disposition-notification\r\nContent-Transfer-Encoding: 7bit\r\n\r\n{}\r\n",
                report.join("\r\n")
            ),
            format!("--{boundary}--\r\n"),
        ];
        parts.join("\r\n").into_bytes()
    }

    pub async fn send(&self, outbox: &Outbox<'_>) -> Result<()> {
        let message = self.message(now(), &fresh_id()?);
        let set = [Keyword::Seen, Keyword::MdnSent, Keyword::AgentMail];
        let copy = outbox.import(&message, &outbox.sent, &set).await?;
        let id = copy.as_str();
        let left = Leftover {
            unknown: format!("not known to be sent; its copy {id} stays in Sent"),
            removed: "the receipt was not sent".into(),
            unremoved: format!("the receipt was not sent, and its copy {id} could not be removed"),
        };
        outbox.deliver(&copy, &self.to, left).await.map(drop)
    }
}

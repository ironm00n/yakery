//! The claim protocol: under the host's claims lock, each mail is marked `$seen` by a call of its own and taken only
//! if that call wrote, which of calls racing for one mail, from any client, one does. A mark whose reply is lost is
//! taken if the mail now carries `$seen`, which another host may have set: the one way a mail reaches two. Mail closed
//! between the snapshot and the mark is let go, and a receipt is sent only where this claim's `$mdnsent` wrote.

use crate::{
    address::MessageId,
    mail::{Current, Email, Keyword, Mail, Mark},
    out::{Asked, is_receipt},
    policy::Login,
    prelude::*,
};

#[derive(Default)]
pub struct Scan {
    pub due: Vec<Email>,
    pub remind: Vec<Email>,
}

pub fn ours(login: &Login, m: &Email) -> bool {
    m.from.iter().any(|a| login.owns(&a.email))
}

pub fn open(login: &Login, m: &Email) -> bool {
    !ours(login, m) && !m.has(Keyword::Answered) && !m.has(Keyword::Handled)
}

fn answers_receipt(login: &Login, m: &Email) -> bool {
    !is_receipt(m) && !matches!(Asked::of(login, m), Asked::Nothing)
}

pub async fn claim(
    server: &Mail,
    login: &Login,
    mut candidates: impl AsyncFnMut() -> Result<Scan>,
) -> (Result<Vec<Email>>, Vec<Email>) {
    let mut remind: Vec<Email> = vec![];
    let claimed = async {
        for _ in 0..20 {
            let scan = candidates().await?;
            remind.extend(
                scan.remind
                    .into_iter()
                    .filter(|m| !remind.iter().any(|r| r.id == m.id))
                    .collect::<Vec<_>>(),
            );
            if scan.due.is_empty() {
                return Ok(vec![]);
            }
            let ids = server
                .canonical(&scan.due.iter().map(|m| m.id.clone()).collect::<Vec<_>>())
                .await?;
            match locked(server, login, &ids).await {
                Ok(Some(claimed)) => return Ok(taken(server, login, claimed).await),
                Ok(None) => {}
                Err(e) if Failure::of(&e) == Failure::TokenRefused => {}
                Err(e) => return Err(e),
            }
        }
        bail!("other changes to the inbox kept beating the claim, 20 times in a row")
    }
    .await;
    (claimed, remind)
}

type Claimed = Vec<(Email, Current)>;

async fn taken(server: &Mail, login: &Login, claimed: Claimed) -> Vec<Email> {
    let ids: Vec<&Current> = claimed.iter().map(|(_, id)| id).collect();
    let now = server.get::<Email>(&ids).await.unwrap_or_default();
    let closed = |m: &Email| now.iter().any(|n| n.id == m.id && !open(login, n));
    let mut claimed: Claimed = claimed.into_iter().filter(|(m, _)| !closed(m)).collect();
    let mut asked: Vec<&mut (Email, Current)> =
        claimed.iter_mut().filter(|(m, _)| answers_receipt(login, m)).collect();
    for chunk in asked.chunks_mut(server.calls_per_request()) {
        let marks: Vec<Mark> =
            chunk.iter().map(|(_, id)| (id.clone(), &[Keyword::MdnSent][..])).collect();
        let wrote = match server.each(false, &marks).await {
            Ok(wrote) => wrote,
            Err(e) => marks.iter().map(|_| Err(anyhow!("{e:#}"))).collect(),
        };
        for ((m, _), wrote) in chunk.iter_mut().zip(wrote) {
            match wrote {
                Ok(true) => continue,
                Ok(false) => {}
                Err(e) => warn!("marking {} $mdnsent failed, so it gets no receipt: {e:#}", m.id),
            }
            m.keywords.insert(Keyword::MdnSent.name().into(), true);
        }
    }
    claimed.into_iter().map(|(m, _)| m).filter(|m| !is_receipt(m)).collect()
}

async fn locked(server: &Mail, login: &Login, ids: &[Current]) -> Result<Option<Claimed>> {
    let _held = server.exclusive().await?;
    let snapshot = server.snapshot(ids).await?.into_iter();
    let fresh: Claimed = snapshot
        .filter(|m| open(login, m) && !m.has(Keyword::Seen))
        .filter_map(|m| {
            let id = ids.iter().find(|c| c.as_str() == m.id)?.clone();
            Some((m, id))
        })
        .collect();
    let (mut claimed, mut failed) = (vec![], None);
    for chunk in fresh.chunks(server.calls_per_request()) {
        let marks: Vec<Mark> =
            chunk.iter().map(|(_, id)| (id.clone(), &[Keyword::Seen][..])).collect();
        let (mut unknown, mut why) = (vec![], None);
        match server.each(true, &marks).await {
            Ok(wrote) => {
                for (mail, wrote) in chunk.iter().zip(wrote) {
                    match wrote {
                        Ok(true) => claimed.push(mail.clone()),
                        Ok(false) => {}
                        Err(e) => {
                            unknown.push(mail);
                            why = Some(e);
                        }
                    }
                }
            }
            Err(e) if Failure::of(&e).nothing_ran() => {
                failed = Some(e);
                break;
            }
            Err(e) => (unknown, why) = (chunk.iter().collect(), Some(e)),
        }
        let Some(e) = why else { continue };
        let ids: Vec<Current> = unknown.iter().map(|(_, id)| id.clone()).collect();
        let now = server.snapshot(&ids).await.unwrap_or_default();
        let seen = |m: &Email| now.iter().any(|n| n.id == m.id && n.has(Keyword::Seen));
        let landed: Claimed = unknown.into_iter().filter(|(m, _)| seen(m)).cloned().collect();
        if landed.is_empty() {
            failed = Some(e);
            break;
        }
        warn!(
            "claiming reported {e:#}, but {} mail now carry $seen; delivering them",
            landed.len()
        );
        claimed.extend(landed);
    }
    match (failed, claimed.is_empty()) {
        (Some(e), true) => Err(e),
        (Some(e), false) => {
            warn!("claiming failed for some mail ({e:#}); delivering what was claimed");
            Ok(Some(claimed))
        }
        (None, empty) => Ok((!empty).then_some(claimed)),
    }
}

pub fn answered_in_sent(
    login: &Login,
    [inbox, sent]: [&str; 2],
    candidates: &[Email],
    members: &[Email],
) -> HashSet<String> {
    let in_inbox: Vec<&Email> =
        candidates.iter().chain(members).filter(|m| m.mailboxes.contains_key(inbox)).collect();
    let mut answered = HashSet::new();
    for r in members.iter().filter(|r| {
        r.mailboxes.contains_key(sent)
            && ours(login, r)
            && !is_receipt(r)
            && !r.has(Keyword::AgentMail)
    }) {
        let [reference] = &MessageId::all(&r.in_reply_to)[..] else { continue };
        let carriers: Vec<&&Email> = in_inbox
            .iter()
            .filter(|m| m.thread == r.thread && MessageId::all(&m.message_id).contains(reference))
            .collect();
        if let [only] = carriers[..] {
            answered.insert(only.id.clone());
        }
    }
    answered.retain(|a| candidates.iter().any(|c| c.id == *a));
    answered
}

#[derive(Clone, Default)]
pub struct Cursor {
    since: Option<String>,
    pending: BTreeSet<String>,
}

impl Cursor {
    pub async fn check<T>(
        &mut self,
        changes: impl AsyncFn(&str) -> Result<Option<(Vec<String>, String)>>,
        everything: impl AsyncFn() -> Result<(Vec<String>, String)>,
        triage: impl AsyncFnOnce(BTreeSet<String>) -> Result<(T, BTreeSet<String>)>,
    ) -> Result<T> {
        let changed = match &self.since {
            Some(since) => changes(since).await?,
            None => None,
        };
        let (ids, state) = match changed {
            Some(changed) => changed,
            None => everything().await?,
        };
        let (found, pending) =
            triage(ids.into_iter().chain(self.pending.iter().cloned()).collect()).await?;
        *self = Cursor { since: Some(state), pending };
        Ok(found)
    }
}

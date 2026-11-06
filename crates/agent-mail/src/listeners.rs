//! The waits the mail-listener mod runs, one per session, beside the session records. A plain wait stands down for
//! its own session's listener and leaves the others the mail that has Owen's authority: the wait that claims a mail
//! first decides whether it reaches the session as his word or as plain output.

use private::Dir;
use text::Address;

use crate::{RECHECK, address::SessionId, prelude::*, store::Lock};

const FILE: &str = "listeners.json";
const LOCK: &str = "listeners.lock";

/// A listener as its last scan left it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Heartbeat {
    pub pid: u32,
    pub topics: BTreeSet<String>,
    pub threads: BTreeSet<String>,
    pub scanned: i64,
}

impl Heartbeat {
    /// Its process runs, and it scanned within three recheck periods.
    fn live(&self, now: i64) -> bool {
        now - self.scanned <= 3 * RECHECK.as_secs() as i64
            && Path::new(&format!("/proc/{}", self.pid)).exists()
    }
}

pub struct Listeners(pub Dir);

impl Listeners {
    fn edit(&self, _held: Lock, edit: impl FnOnce(&mut BTreeMap<String, Heartbeat>)) -> Result<()> {
        let mut all = self.0.load(FILE)?;
        edit(&mut all);
        self.0.save(FILE, &all)
    }

    async fn change(&self, edit: impl FnOnce(&mut BTreeMap<String, Heartbeat>)) -> Result<()> {
        self.edit(Lock::take(&self.0, LOCK).await?, edit)
    }

    /// Records that this process scanned for `session` at `now`, watching `topics` and the threads of `threads`.
    pub async fn beat(
        &self,
        session: &SessionId,
        topics: &[Address],
        threads: &[String],
        now: i64,
    ) -> Result<()> {
        let topics = topics.iter().map(|t| t.to_string().to_lowercase()).collect();
        let threads = threads.iter().cloned().collect();
        let beat = Heartbeat { pid: std::process::id(), topics, threads, scanned: now };
        self.put(session, beat).await
    }

    pub async fn put(&self, session: &SessionId, beat: Heartbeat) -> Result<()> {
        self.change(|all| drop(all.insert(session.to_string(), beat))).await
    }

    /// Takes out `session`'s heartbeat if this process wrote it, so a newer listener for the session keeps its own.
    /// It does not wait for the lock, which a drop cannot do: while another process holds it, the heartbeat stays,
    /// and counts as gone once its process ends.
    pub fn leave(&self, session: &SessionId) -> Result<()> {
        let held = Lock::try_take(&self.0, LOCK)?.with_context(|| {
            format!(
                "another process holds {LOCK}, so {session} stays listed until this process ends"
            )
        })?;
        let (key, pid) = (session.to_string(), std::process::id());
        self.edit(held, |all| {
            if all.get(&key).is_some_and(|b| b.pid == pid) {
                all.remove(&key);
            }
        })
    }

    pub fn all(&self) -> Result<BTreeMap<String, Heartbeat>> {
        self.0.load(FILE)
    }

    /// The topics a live listener watches.
    pub fn watched(&self, now: i64) -> Result<Vec<Address>> {
        let all = self.all()?;
        let topics: BTreeSet<&String> =
            all.values().filter(|b| b.live(now)).flat_map(|b| &b.topics).collect();
        Ok(topics.into_iter().filter_map(|t| t.parse().ok()).collect())
    }

    /// `session`'s listener, while it is live.
    pub fn listening(&self, session: &SessionId, now: i64) -> Result<Option<Heartbeat>> {
        Ok(self.all()?.remove(&session.to_string()).filter(|b| b.live(now)))
    }
}

/// Takes a session's listener out when its wait ends, however it ends.
pub struct Leave(pub Listeners, pub SessionId);

impl Drop for Leave {
    fn drop(&mut self) {
        if let Err(e) = self.0.leave(&self.1) {
            warn!("{e:#}");
        }
    }
}

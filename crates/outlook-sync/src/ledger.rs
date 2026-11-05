//! The mirror and the sync's record of what it wrote there. A record may miss a file the sync
//! wrote, which the file's UID still names as the sync's, but never names a file the sync deleted,
//! which would read as the owner's delete of Outlook's event.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use private::Dir;
use serde::{Deserialize, Serialize};

use crate::caldav::{Entry, Mirror};
use crate::{local, state};

#[derive(Default, Serialize, Deserialize)]
struct Record {
    files: BTreeMap<String, Mirrored>,
    token: Option<String>,
}

/// The ETag of the sync's own last write to a file tells the owner's edits apart.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mirrored {
    pub id: String,
    pub etag: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Edit {
    Unchanged,
    Deleted(Mirrored),
    Edited(Mirrored),
    New,
}

pub fn plan(entry: &Entry, mirrored: Option<&Mirrored>) -> Edit {
    match (mirrored, entry.gone) {
        (Some(m), true) => Edit::Deleted(m.clone()),
        (None, true) => Edit::Unchanged,
        (Some(m), false) if entry.etag.as_deref() == Some(m.etag.as_str()) => Edit::Unchanged,
        (Some(m), false) => Edit::Edited(m.clone()),
        (None, false) => Edit::New,
    }
}

pub struct Ledger<'a> {
    mirror: &'a Mirror,
    state: &'a Dir,
    host: &'a str,
    record: Record,
}

impl<'a> Ledger<'a> {
    pub fn open(mirror: &'a Mirror, state: &'a Dir, host: &'a str) -> Result<Ledger<'a>> {
        Ok(Ledger { mirror, state, host, record: state.load(state::WRITTEN)? })
    }

    pub fn save(&self) -> Result<()> {
        self.state.save(state::WRITTEN, &self.record)
    }

    pub fn looked(&self) -> bool {
        self.record.token.is_some()
    }

    pub async fn changes(&self) -> Result<(Vec<(Entry, Edit)>, String)> {
        let (entries, token) = self.mirror.changes(self.record.token.as_deref()).await?;
        let planned = entries.into_iter().map(|e| {
            let edit = plan(&e, self.record.files.get(&e.file));
            (e, edit)
        });
        Ok((planned.collect(), token))
    }

    pub fn looked_up_to(&mut self, token: String) -> Result<()> {
        self.record.token = Some(token);
        self.save()
    }

    pub async fn get(&self, file: &str) -> Result<Option<(String, String)>> {
        self.mirror.get(file).await
    }

    pub fn record(&mut self, file: String, mirrored: Mirrored) -> Result<()> {
        self.record.files.insert(file, mirrored);
        self.save()
    }

    pub fn forget(&mut self, file: &str) -> Result<()> {
        self.record.files.remove(file);
        self.save()
    }

    /// With `guarded`, only over the sync's own last version: false when a newer one, an owner edit
    /// not yet sent, is there. Recorded in memory; [`Ledger::save`] writes it.
    pub async fn write(
        &mut self,
        file: &str,
        id: String,
        ics: String,
        guarded: bool,
    ) -> Result<bool> {
        let over = self.record.files.get(file).filter(|_| guarded).map(|m| m.etag.clone());
        let Some(etag) = self.mirror.put(file, ics, over.as_deref()).await? else {
            return Ok(false);
        };
        self.record.files.insert(file.to_string(), Mirrored { id, etag });
        Ok(true)
    }

    pub async fn delete(&mut self, files: impl IntoIterator<Item = String>) -> Result<()> {
        let files: Vec<String> = files.into_iter().collect();
        for file in &files {
            self.record.files.remove(file);
        }
        self.save()?;
        for file in &files {
            self.mirror.delete(file).await?;
        }
        Ok(())
    }

    /// The files a listing didn't see; with `owners_kept`, only those of the sync's.
    pub async fn sweep(&mut self, seen: &BTreeSet<String>, owners_kept: bool) -> Result<()> {
        let mut gone = vec![];
        for file in self.mirror.files().await?.difference(seen) {
            let ours = !owners_kept
                || self.record.files.contains_key(file)
                || (self.get(file).await?)
                    .is_some_and(|(ics, _)| local::stamped(file, &ics, self.host));
            if ours {
                gone.push(file.clone());
            }
        }
        self.delete(gone).await
    }
}

#[cfg(test)]
#[path = "../tests/ledger.rs"]
mod tests;

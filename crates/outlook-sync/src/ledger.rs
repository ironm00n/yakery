//! The mirror and the sync's record of what it wrote there. A stop at any point may leave the
//! record missing a file the sync wrote, whose UID still names it as the sync's, or holding one in
//! flight; never naming a file the sync deleted, or one with an ETag older than the sync's last
//! write to it, which would read as the owner's delete or edit of Outlook's event.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use private::Dir;
use serde::{Deserialize, Serialize};

use crate::file::FileName;
use crate::{local, state};

#[path = "caldav.rs"]
mod caldav;
pub use caldav::{Entry, Mirror};

#[derive(Default, Serialize, Deserialize)]
struct Record {
    files: BTreeMap<FileName, Mirrored>,
    token: Option<String>,
}

/// The ETag of the sync's own last write to a file tells the owner's edits apart; None while a
/// write of the file is in flight.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mirrored {
    pub id: String,
    pub etag: Option<String>,
}

pub struct Put {
    pub file: FileName,
    pub id: String,
    pub ics: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Edit {
    Unchanged,
    Unsettled,
    Deleted(Mirrored),
    Edited(Mirrored),
    New,
}

pub fn plan(entry: &Entry, mirrored: Option<&Mirrored>) -> Edit {
    match (mirrored, entry.gone) {
        (Some(Mirrored { etag: None, .. }), _) => Edit::Unsettled,
        (Some(m), true) => Edit::Deleted(m.clone()),
        (None, true) => Edit::Unchanged,
        (Some(m), false) if entry.etag == m.etag => Edit::Unchanged,
        (Some(m), false) => Edit::Edited(m.clone()),
        (None, false) => Edit::New,
    }
}

pub struct Ledger<'a> {
    mirror: &'a Mirror,
    state: &'a Dir,
    record: Record,
}

impl<'a> Ledger<'a> {
    pub fn open(mirror: &'a Mirror, state: &'a Dir) -> Result<Ledger<'a>> {
        Ok(Ledger { mirror, state, record: state.load(state::WRITTEN)? })
    }

    fn save(&self) -> Result<()> {
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

    pub fn record(&mut self, file: FileName, mirrored: Mirrored) -> Result<()> {
        self.record.files.insert(file, mirrored);
        self.save()
    }

    pub fn forget(&mut self, file: &FileName) -> Result<()> {
        self.record.files.remove(file);
        self.save()
    }

    /// With `guarded`, each file only over the sync's own last version; the files a newer one, an
    /// owner edit not yet sent, kept are returned.
    pub async fn write(&mut self, puts: Vec<Put>, guarded: bool) -> Result<Vec<FileName>> {
        if puts.is_empty() {
            return Ok(vec![]);
        }
        let overs: Vec<Option<String>> = (puts.iter())
            .map(|p| {
                self.record.files.get(&p.file).filter(|_| guarded).and_then(|m| m.etag.clone())
            })
            .collect();
        for p in &puts {
            self.record.files.insert(p.file.clone(), Mirrored { id: p.id.clone(), etag: None });
        }
        self.save()?;
        let mut kept = vec![];
        for (Put { file, id, ics }, over) in puts.into_iter().zip(overs) {
            let etag = self.mirror.put(&file, ics, over.as_deref()).await?;
            if etag.is_none() {
                kept.push(file.clone());
            }
            self.record.files.insert(file, Mirrored { id, etag: etag.or(over) });
        }
        self.save()?;
        Ok(kept)
    }

    pub async fn delete(&mut self, files: impl IntoIterator<Item = FileName>) -> Result<()> {
        let files: Vec<FileName> = files.into_iter().collect();
        if files.is_empty() {
            return Ok(());
        }
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
    pub async fn sweep(&mut self, seen: &BTreeSet<FileName>, owners_kept: bool) -> Result<()> {
        let mut gone = vec![];
        for file in self.mirror.files().await?.difference(seen) {
            let ours = !owners_kept
                || self.record.files.contains_key(file)
                || (self.mirror.get(file).await?)
                    .is_some_and(|(ics, _)| local::stamped(file, &ics));
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

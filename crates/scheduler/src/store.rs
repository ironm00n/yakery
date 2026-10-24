//! The links file, which only [`Store::apply`] changes: it writes the next links whole, through
//! `private::Dir`, before the core acts on them, and after a failed write it holds what the disk
//! does.

use std::path::Path;

use anyhow::Result;
use private::Dir;

use crate::link::Links;

const FILE: &str = "links.json";

pub struct Store {
    dir: Dir,
    links: Links,
}

impl Store {
    pub fn open(state: &Path) -> Result<Store> {
        let dir = Dir::open(state, &[])?;
        Ok(Store { links: dir.load(FILE)?, dir })
    }

    pub fn links(&self) -> &Links {
        &self.links
    }

    pub fn apply(&mut self, change: impl FnOnce(&Links) -> Result<Links>) -> Result<()> {
        let next = change(&self.links)?;
        let saved = self.dir.save(FILE, &next);
        self.links = if saved.is_ok() { next } else { self.dir.load(FILE)? };
        saved
    }
}

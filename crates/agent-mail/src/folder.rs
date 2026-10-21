//! Attachments saved where only agent-mail writes, `<home>/.cache/agent-mail/attachments/<email id>/`, under names
//! reduced to plain ASCII.

use std::{
    collections::HashSet,
    fs::File,
    io::Write,
    os::fd::OwnedFd,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use rustix::{
    fs::{
        AtFlags, CWD, Dir, FileType, Mode, OFlags, RenameFlags, fstat, mkdirat, openat,
        renameat_with, statat, unlinkat,
    },
    io::Errno,
    process::getuid,
};

use crate::{
    mail::JmapId,
    random::{ALPHANUMERIC, Draws},
};

const ROOT: [&str; 3] = [".cache", "agent-mail", "attachments"];
/// Saved names never start with a dot, so no attachment can take a partial file's name.
const PARTIAL: &str = ".partial-";
/// Far longer than a write takes, since a partial file is written only once its download is in memory.
const PARTIAL_EXPIRY: Duration = Duration::from_secs(600);
/// Below the 255-byte limit of common file systems, with room for a collision suffix.
const MAX_NAME_BYTES: usize = 200;
const MAX_EXTENSION_BYTES: usize = 16;

/// File names unique among those given out, ignoring case: a sender's name reduced to a basename of
/// `[A-Za-z0-9._-]` with no leading dot or dash, or `attachment-<n>` when no letter or digit is left of it.
#[derive(Default)]
pub struct Names(HashSet<String>);

impl Names {
    pub fn next(&mut self, raw: Option<&str>, n: usize) -> String {
        let base = basename(raw.unwrap_or_default()).unwrap_or_else(|| format!("attachment-{n}"));
        let (stem, extension) = split_extension(&base);
        let name = std::iter::once(base.clone())
            .chain((2..).map(|k| format!("{stem}-{k}{extension}")))
            .find(|name| !self.0.contains(&name.to_ascii_lowercase()))
            .expect("the suffixes are endless");
        self.0.insert(name.to_ascii_lowercase());
        name
    }
}

fn basename(raw: &str) -> Option<String> {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let mut name = String::with_capacity(last.len());
    for c in last.chars() {
        let c = match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '.' | '-' => c,
            _ => '_',
        };
        if !(c == '_' && name.ends_with('_')) {
            name.push(c);
        }
    }
    let name = name.trim_start_matches(['.', '-']);
    name.bytes()
        .any(|b| b.is_ascii_alphanumeric())
        .then(|| shorten(name))
}

/// `.png` of `a.png`; none when the dot is the first character.
fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 && name.len() - i <= MAX_EXTENSION_BYTES => name.split_at(i),
        _ => (name, ""),
    }
}

fn shorten(name: &str) -> String {
    if name.len() <= MAX_NAME_BYTES {
        return name.to_owned();
    }
    let (stem, extension) = split_extension(name);
    format!("{}{extension}", &stem[..MAX_NAME_BYTES - extension.len()])
}

pub enum Kept {
    Saved(PathBuf),
    /// By an earlier run: a file gets its name only once it is whole.
    Already(PathBuf),
}

/// One email's folder. Everything in it is reached through the directory's handle, so moving a directory on the
/// way to it cannot redirect a write.
pub struct Folder {
    dir: OwnedFd,
    path: PathBuf,
}

impl Folder {
    /// Each directory below `home` is created private to the user, or must already be a real directory of the
    /// user's that no one else may write to. Each is opened from its parent's handle, never through a symlink.
    pub fn of(home: &Path, email: JmapId) -> Result<Folder> {
        let mut dir = openat(
            CWD,
            home,
            OFlags::DIRECTORY | OFlags::RDONLY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .with_context(|| home.display().to_string())?;
        let mut path = home.to_owned();
        for name in ROOT.into_iter().chain([email.as_str()]) {
            path.push(name);
            dir = private_child(&dir, name).with_context(|| path.display().to_string())?;
        }
        let folder = Folder { dir, path };
        folder.drop_stale_partials()?;
        Ok(folder)
    }

    pub fn existing(&self, name: &str) -> Result<Option<PathBuf>> {
        match statat(&self.dir, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) if FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile => {
                Ok(Some(self.path.join(name)))
            }
            Ok(_) => bail!("{name} is there but is not a file"),
            Err(Errno::NOENT) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, name: &str, bytes: &[u8]) -> Result<Kept> {
        let partial = self.stage(bytes)?;
        let renamed = renameat_with(&self.dir, &partial, &self.dir, name, RenameFlags::NOREPLACE);
        if renamed.is_err() {
            let _ = unlinkat(&self.dir, &partial, AtFlags::empty());
        }
        match renamed {
            Ok(()) => Ok(Kept::Saved(self.path.join(name))),
            Err(Errno::EXIST) => Ok(Kept::Already(self.path.join(name))),
            Err(e) => Err(e.into()),
        }
    }

    /// Writes `bytes` to a new partial file, private to the user. A write cut short leaves only that.
    fn stage(&self, bytes: &[u8]) -> Result<String> {
        let partial = format!("{PARTIAL}{}", Draws::default().string(ALPHANUMERIC, 16)?);
        let mut file = File::from(openat(
            &self.dir,
            &partial,
            OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )?);
        if let Err(e) = file.write_all(bytes).and_then(|()| file.sync_all()) {
            let _ = unlinkat(&self.dir, &partial, AtFlags::empty());
            return Err(e.into());
        }
        Ok(partial)
    }

    /// Partial files a killed run left. Only old ones, since a run saving into this folder now may own the others.
    fn drop_stale_partials(&self) -> Result<()> {
        let expired = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .saturating_sub(PARTIAL_EXPIRY)
            .as_secs() as i64;
        for entry in Dir::read_from(&self.dir)? {
            let entry = entry?;
            let name = entry.file_name();
            if !name.to_bytes().starts_with(PARTIAL.as_bytes()) {
                continue;
            }
            match statat(&self.dir, name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(stat) if stat.st_mtime < expired => unlinkat(&self.dir, name, AtFlags::empty())?,
                Ok(_) | Err(Errno::NOENT) => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
}

fn private_child(parent: &OwnedFd, name: &str) -> Result<OwnedFd> {
    match mkdirat(parent, name, Mode::RWXU) {
        Ok(()) | Err(Errno::EXIST) => {}
        Err(e) => return Err(e.into()),
    }
    let dir = openat(
        parent,
        name,
        OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| match e {
        Errno::LOOP | Errno::NOTDIR => anyhow!("not a directory, or a symlink"),
        e => e.into(),
    })?;
    let stat = fstat(&dir)?;
    if stat.st_uid != getuid().as_raw() {
        bail!("owned by uid {}, not this user", stat.st_uid);
    }
    if stat.st_mode & 0o022 != 0 {
        bail!("others may write to it (mode {:o})", stat.st_mode & 0o777);
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, Permissions},
        os::unix::fs::{PermissionsExt, symlink},
    };

    use super::*;
    use crate::auth::store::tests::scratch;

    fn name(raw: &str) -> String {
        Names::default().next(Some(raw), 3)
    }

    #[test]
    fn names_are_plain_ascii_that_cannot_leave_the_folder() {
        for (raw, saved) in [
            ("../../x", "x"),
            ("/etc/passwd", "passwd"),
            ("..\\..\\evil.exe", "evil.exe"),
            ("a\nb", "a_b"),
            ("invoice\u{202E}fdp.exe", "invoice_fdp.exe"),
            ("a\u{200B}.png", "a_.png"),
            ("a.png ", "a.png_"),
            ("\u{FEFF}.bashrc", "_.bashrc"),
            ("..\u{FF0F}..\u{FF0F}home\u{FF0F}.ssh", "_.._home_.ssh"),
            ("\u{E0041}tag", "_tag"),
            ("x  \u{2028}\t y.png", "x_y.png"),
            (".bashrc", "bashrc"),
            ("--rf", "rf"),
            ("Fünf Bilder.JPG", "F_nf_Bilder.JPG"),
            ("é.png", "_.png"),
        ] {
            assert_eq!(name(raw), saved, "{raw:?}");
        }
        for nothing in [
            "", "..", "a/", "\n", "/", " ", "\u{200B}", "\u{0301}", "-._",
        ] {
            assert_eq!(name(nothing), "attachment-3", "{nothing:?}");
        }
        assert_eq!(Names::default().next(None, 1), "attachment-1");
    }

    #[test]
    fn long_names_keep_their_extension() {
        let long = name(&format!("{}.png", "a".repeat(300)));
        assert_eq!(long.len(), MAX_NAME_BYTES);
        assert!(long.ends_with("a.png"), "{long}");
    }

    #[test]
    fn duplicate_names_get_a_suffix_ignoring_case() {
        let mut names = Names::default();
        let given: Vec<String> = [
            Some("a.png"),
            Some("A.PNG"),
            Some("dir/a.png"),
            Some("a-2.png"),
            None,
            Some("attachment-5"),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, raw)| names.next(raw, i + 1))
        .collect();
        assert_eq!(
            given,
            [
                "a.png",
                "A-2.PNG",
                "a-3.png",
                "a-2-2.png",
                "attachment-5",
                "attachment-5-2"
            ]
        );
    }

    fn id(id: &str) -> JmapId<'_> {
        JmapId::new(id).unwrap()
    }

    fn mode(path: &Path) -> u32 {
        fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn saved(kept: Kept) -> PathBuf {
        match kept {
            Kept::Saved(path) => path,
            Kept::Already(path) => panic!("{} was there already", path.display()),
        }
    }

    #[test]
    fn the_folder_chain_is_created_private() {
        let home = scratch("folder-chain");
        fs::create_dir(&home).unwrap();
        let folder = Folder::of(&home, id("Mx1")).unwrap();
        let path = saved(folder.save("a.png", b"one").unwrap());
        assert_eq!(path, home.join(".cache/agent-mail/attachments/Mx1/a.png"));
        for dir in [
            ".cache",
            ".cache/agent-mail",
            ".cache/agent-mail/attachments",
            ".cache/agent-mail/attachments/Mx1",
        ] {
            assert_eq!(mode(&home.join(dir)), 0o700, "{dir}");
        }
        assert_eq!(mode(&path), 0o600);
        assert_eq!(fs::read(&path).unwrap(), b"one");
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_symlinked_or_shared_ancestor_is_refused() {
        let home = scratch("folder-refused");
        let elsewhere = home.join("elsewhere");
        fs::create_dir_all(elsewhere.join("agent-mail")).unwrap();
        symlink(&elsewhere, home.join(".cache")).unwrap();
        let linked = Folder::of(&home, id("Mx1")).err().unwrap();
        assert!(format!("{linked:#}").contains("symlink"), "{linked:#}");
        assert!(!elsewhere.join("agent-mail/attachments").exists());
        fs::remove_file(home.join(".cache")).unwrap();
        fs::create_dir_all(home.join(".cache/agent-mail")).unwrap();
        fs::set_permissions(
            home.join(".cache/agent-mail"),
            Permissions::from_mode(0o770),
        )
        .unwrap();
        let shared = Folder::of(&home, id("Mx1")).err().unwrap();
        assert!(format!("{shared:#}").contains("mode 770"), "{shared:#}");
        assert!(!home.join(".cache/agent-mail/attachments").exists());
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn moving_an_ancestor_mid_run_does_not_redirect_writes() {
        let home = scratch("folder-moved");
        fs::create_dir(&home).unwrap();
        let folder = Folder::of(&home, id("Mx1")).unwrap();
        fs::rename(home.join(".cache"), home.join("moved")).unwrap();
        fs::create_dir_all(home.join("decoy/agent-mail/attachments/Mx1")).unwrap();
        symlink(home.join("decoy"), home.join(".cache")).unwrap();
        folder.save("a.png", b"one").unwrap();
        assert!(home.join("moved/agent-mail/attachments/Mx1/a.png").exists());
        assert!(!home.join("decoy/agent-mail/attachments/Mx1/a.png").exists());
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn saving_again_finds_the_file_already_saved() {
        let home = scratch("folder-again");
        fs::create_dir(&home).unwrap();
        let folder = Folder::of(&home, id("Mx1")).unwrap();
        assert!(folder.existing("a.png").unwrap().is_none());
        let path = saved(folder.save("a.png", b"one").unwrap());
        let again = Folder::of(&home, id("Mx1")).unwrap();
        assert_eq!(again.existing("a.png").unwrap(), Some(path.clone()));
        assert!(matches!(again.save("a.png", b"two").unwrap(), Kept::Already(p) if p == path));
        assert_eq!(fs::read(&path).unwrap(), b"one");
        symlink(&path, path.with_file_name("link")).unwrap();
        assert!(again.existing("link").is_err());
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_killed_write_leaves_only_a_partial_file_that_later_runs_ignore_then_drop() {
        let home = scratch("folder-killed");
        fs::create_dir(&home).unwrap();
        let folder = Folder::of(&home, id("Mx1")).unwrap();
        let partial = folder.stage(b"on").unwrap();
        let dir = home.join(".cache/agent-mail/attachments/Mx1");
        let entries = || {
            let mut names: Vec<String> = fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            names.sort();
            names
        };
        assert_eq!(entries(), [partial.as_str()]);
        let again = Folder::of(&home, id("Mx1")).unwrap();
        assert!(again.existing("a.png").unwrap().is_none());
        saved(again.save("a.png", b"one").unwrap());
        assert_eq!(entries(), [partial.as_str(), "a.png"]);
        let old = SystemTime::now() - PARTIAL_EXPIRY - Duration::from_secs(60);
        File::options()
            .write(true)
            .open(dir.join(&partial))
            .unwrap()
            .set_modified(old)
            .unwrap();
        Folder::of(&home, id("Mx1")).unwrap();
        assert_eq!(entries(), ["a.png"]);
        assert_eq!(fs::read(dir.join("a.png")).unwrap(), b"one");
        fs::remove_dir_all(&home).unwrap();
    }
}

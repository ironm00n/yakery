use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use jmap_client::email::{EmailBodyPart, Property as P};

use crate::{
    auth::Auth,
    config::Config,
    folder::{Folder, Kept, Names},
    home::home,
    mail::{Bounds, JmapId, Mail},
    show::{out, plain, warn},
};

const MAX_FILE_BYTES: usize = 50 << 20;
const MAX_TOTAL_BYTES: usize = 200 << 20;
const DOWNLOAD_DEADLINE: Duration = Duration::from_secs(300);

/// Saves each attachment of an email into its folder and prints each file's path. Read-only on the server.
pub async fn run(id: String) -> Result<()> {
    let mail = Mail::connect(Auth::new(Config::load()?)?).await?;
    let current = mail
        .current(&id)
        .await?
        .with_context(|| format!("no email {}", plain(&id)))?;
    let found = mail
        .emails(std::slice::from_ref(&current), &[P::Attachments], |_| {})
        .await?;
    let email = found.first().context("the email went away")?;
    let attachments = email.attachments().unwrap_or_default();
    ensure!(!attachments.is_empty(), "{} has no attachments", plain(&id));
    let folder = Folder::of(&home()?, JmapId::new(&current)?).context("the attachments folder")?;
    let mut names = Names::default();
    let mut left = MAX_TOTAL_BYTES;
    let mut failed = 0;
    for (n, part) in attachments.iter().enumerate() {
        let name = names.next(part.name(), n + 1);
        match keep(&mail, &folder, part, &name, left).await {
            Ok((Kept::Saved(path), size)) => {
                left -= size;
                out(&path.display().to_string())?;
            }
            Ok((Kept::Already(path), _)) => {
                warn(&format!("{name} was saved already"));
                out(&path.display().to_string())?;
            }
            Err(e) => {
                warn(&format!("skipped {name}: {e:#}"));
                failed += 1;
            }
        }
    }
    if failed > 0 {
        bail!("skipped {failed} of {} attachments", attachments.len());
    }
    Ok(())
}

/// The file kept and the bytes downloaded for it.
async fn keep(
    mail: &Mail,
    folder: &Folder,
    part: &EmailBodyPart,
    name: &str,
    left: usize,
) -> Result<(Kept, usize)> {
    if let Some(path) = folder.existing(name)? {
        return Ok((Kept::Already(path), 0));
    }
    let blob = part.blob_id().context("it has no blob")?;
    let bounds = Bounds {
        bytes: MAX_FILE_BYTES.min(left),
        time: DOWNLOAD_DEADLINE,
    };
    if part.size() > bounds.bytes {
        bail!("{} bytes, over {}", part.size(), limit(bounds.bytes));
    }
    let bytes = mail
        .download(blob, bounds)
        .await?
        .with_context(|| format!("its download ran past {}", limit(bounds.bytes)))?;
    Ok((folder.save(name, &bytes)?, bytes.len()))
}

fn limit(bytes: usize) -> String {
    if bytes < MAX_FILE_BYTES {
        format!("the {bytes} bytes left of the {MAX_TOTAL_BYTES}-byte total")
    } else {
        format!("the {MAX_FILE_BYTES}-byte limit")
    }
}

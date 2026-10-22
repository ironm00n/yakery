use std::collections::HashSet;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail, ensure};
use jmap_client::email::{Email, EmailBodyPart, Property as P};

use crate::{
    auth::Auth,
    config::Config,
    embedded::{self, Embedded},
    folder::{Folder, Kept, Names},
    home::home,
    mail::{Bounds, JmapId, Mail},
    show::{out, plain, warn},
};

const MAX_FILE_BYTES: usize = 50 << 20;
const MAX_TOTAL_BYTES: usize = 200 << 20;
const DOWNLOAD_DEADLINE: Duration = Duration::from_secs(300);

/// What a file is saved from.
enum Source<'a> {
    /// A part of the mail, downloaded by its blob.
    Part(&'a EmailBodyPart),
    /// An image the HTML carries in itself, or why it couldn't be read.
    Embedded(Result<Embedded>),
}

impl Source<'_> {
    /// The name it came with, if any.
    fn name(&self, n: usize) -> Option<String> {
        match self {
            Source::Part(part) => part.name().map(str::to_owned),
            Source::Embedded(Ok(image)) => Some(image.name(n)),
            Source::Embedded(Err(_)) => None,
        }
    }
}

/// Saves each attachment of an email, and each image its HTML embeds, into its folder and prints each
/// file's path. Read-only on the server.
pub async fn run(id: String) -> Result<()> {
    let mail = Mail::connect(Auth::new(Config::load()?)?).await?;
    let current = mail
        .current(&id)
        .await?
        .with_context(|| format!("no email {}", plain(&id)))?;
    let properties = [P::Attachments, P::TextBody, P::HtmlBody, P::BodyValues];
    let found = mail
        .emails(std::slice::from_ref(&current), &properties, |a| {
            a.fetch_html_body_values(true)
                .max_body_value_bytes(MAX_TOTAL_BYTES);
        })
        .await?;
    let email = found.first().context("the email went away")?;
    let sources = sources(email);
    ensure!(!sources.is_empty(), "{} has no attachments", plain(&id));
    let folder = Folder::of(&home()?, JmapId::new(&current)?).context("the attachments folder")?;
    let mut names = Names::default();
    let mut left = MAX_TOTAL_BYTES;
    let mut failed = 0;
    let count = sources.len();
    for (n, source) in sources.into_iter().enumerate() {
        let name = names.next(source.name(n + 1).as_deref(), n + 1);
        match keep(&mail, &folder, source, &name, left).await {
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
        bail!("skipped {failed} of {count} attachments");
    }
    Ok(())
}

/// The attachments and the media either body shows inline, each part once, since JMAP leaves a part out
/// of `attachments` when both bodies show it (RFC 8621 section 4.1.4); then the images the HTML carries.
fn sources(email: &Email) -> Vec<Source<'_>> {
    let html = email.html_body().unwrap_or_default();
    let bodies = email.text_body().unwrap_or_default().iter().chain(html);
    let media = bodies.filter(|part| !part.content_type().is_some_and(|t| t.starts_with("text/")));
    let mut seen = HashSet::new();
    let parts = email.attachments().unwrap_or_default().iter().chain(media);
    let parts = parts
        .filter(|part| seen.insert(part.part_id()))
        .map(Source::Part);
    let values = html
        .iter()
        .filter_map(|part| email.body_value(part.part_id()?));
    let embedded = values.flat_map(|value| {
        if value.is_truncated() {
            vec![Err(anyhow!(
                "its HTML runs past {MAX_TOTAL_BYTES} bytes, so no image in it was read"
            ))]
        } else {
            embedded::images(value.value())
        }
    });
    parts.chain(embedded.map(Source::Embedded)).collect()
}

/// The file kept and the bytes written for it.
async fn keep(
    mail: &Mail,
    folder: &Folder,
    source: Source<'_>,
    name: &str,
    left: usize,
) -> Result<(Kept, usize)> {
    if let Some(path) = folder.existing(name)? {
        return Ok((Kept::Already(path), 0));
    }
    let most = MAX_FILE_BYTES.min(left);
    let bytes = match source {
        Source::Part(part) => download(mail, part, most).await?,
        Source::Embedded(image) => image?.bytes,
    };
    ensure!(
        bytes.len() <= most,
        "{} bytes, over {}",
        bytes.len(),
        limit(most)
    );
    Ok((folder.save(name, &bytes)?, bytes.len()))
}

async fn download(mail: &Mail, part: &EmailBodyPart, most: usize) -> Result<Vec<u8>> {
    let blob = part.blob_id().context("it has no blob")?;
    if part.size() > most {
        bail!("{} bytes, over {}", part.size(), limit(most));
    }
    let bounds = Bounds {
        bytes: most,
        time: DOWNLOAD_DEADLINE,
    };
    mail.download(blob, bounds)
        .await?
        .with_context(|| format!("its download ran past {}", limit(most)))
}

fn limit(bytes: usize) -> String {
    if bytes < MAX_FILE_BYTES {
        format!("the {bytes} bytes left of the {MAX_TOTAL_BYTES}-byte total")
    } else {
        format!("the {MAX_FILE_BYTES}-byte limit")
    }
}

//! Images a mail's HTML carries in itself as `data:` URIs, as a phone's mail app
//! sends a photo; JMAP lists no part for them.

use anyhow::{Context, Result, ensure};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use lol_html::{RewriteStrSettings, element, rewrite_str};

pub struct Embedded {
    media_type: String,
    pub bytes: Vec<u8>,
}

impl Embedded {
    /// `embedded-<n>`, with its media subtype as the extension.
    pub fn name(&self, n: usize) -> String {
        let subtype = self
            .media_type
            .split_once('/')
            .map_or("bin", |(_, sub)| sub);
        format!("embedded-{n}.{subtype}")
    }
}

/// Each `<img>` with a `data:` source, in document order; one that isn't base64
/// media is an error in its place.
pub fn images(html: &str) -> Vec<Result<Embedded>> {
    let mut sources = Vec::new();
    let settings = RewriteStrSettings::new().append_element_content_handler(element!(
        r#"img[src^="data:"]"#,
        |el| {
            sources.extend(el.get_attribute("src"));
            Ok(())
        }
    ));
    match rewrite_str(html, settings) {
        Ok(_) => sources.iter().map(|src| decode(src)).collect(),
        Err(e) => vec![Err(e).context("the HTML body")],
    }
}

fn decode(src: &str) -> Result<Embedded> {
    let (meta, data) = src
        .strip_prefix("data:")
        .and_then(|rest| rest.split_once(','))
        .context("a data: URI with no comma")?;
    let media_type = meta
        .strip_suffix(";base64")
        .context("a data: URI that isn't base64")?;
    ensure!(media_type.contains('/'), "a data: URI with no media type");
    let data: String = data.split_ascii_whitespace().collect();
    let bytes = STANDARD.decode(data).context("its base64")?;
    Ok(Embedded {
        media_type: media_type.to_owned(),
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_in_the_html_is_decoded_and_named_by_its_type() {
        let html = "<p>see img</p><img alt=x src=\"data:image/png;base64,iVBORw0K\nGgo=\">";
        let found = images(html);
        assert_eq!(found.len(), 1);
        let image = found.into_iter().next().unwrap().expect("it decodes");
        assert_eq!(image.bytes, b"\x89PNG\r\n\x1a\n");
        assert_eq!(image.name(3), "embedded-3.png");
    }

    #[test]
    fn a_remote_image_is_left_and_one_not_in_base64_is_an_error() {
        let html = r#"<img src="https://example.com/a.png"><img src="data:text/plain,hi">"#;
        let found = images(html);
        assert_eq!(found.len(), 1, "the remote one is no data: URI");
        assert!(found[0].is_err());
    }
}

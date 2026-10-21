//! A MIME entity built outside agent-mail, as `send --mime` takes it. It may carry only `Content-*` header fields
//! and `MIME-Version`, so who a mail is from, whom it goes to and what it is about come from agent-mail alone.

use anyhow::{Result, bail, ensure};

pub struct Entity {
    /// Its `Content-*` fields, each ending in CRLF, continuation lines kept.
    fields: Vec<u8>,
    body: Vec<u8>,
}

impl Entity {
    /// Bare LFs become CRLF throughout, so a binary transfer encoding does not survive; mail needs none.
    pub fn parse(bytes: &[u8]) -> Result<Entity> {
        let bytes = crlf(bytes);
        let end = match bytes.starts_with(b"\r\n") {
            true => 0,
            false => find(&bytes, b"\r\n\r\n")
                .map(|i| i + 2)
                .ok_or_else(|| anyhow::anyhow!("no blank line ends its header fields"))?,
        };
        let (header, body) = (&bytes[..end], &bytes[end + 2..]);
        ensure!(header.is_ascii(), "its header fields are not ASCII");
        ensure!(
            !header.windows(2).any(|w| w[0] == b'\r' && w[1] != b'\n'),
            "its header fields hold a bare CR"
        );
        let mut fields = Vec::new();
        let mut content_types = 0;
        for field in Field::split(header)? {
            match field.name.to_ascii_lowercase().as_str() {
                "mime-version" => ensure!(
                    field.value().trim() == "1.0",
                    "MIME-Version {} is not 1.0",
                    field.value().trim()
                ),
                name if name.starts_with("content-") => {
                    content_types += usize::from(name == "content-type");
                    fields.extend_from_slice(field.raw);
                }
                _ => bail!(
                    "it sets {}, which only agent-mail writes; an entity carries Content-* fields and MIME-Version",
                    field.name
                ),
            }
        }
        match content_types {
            0 => bail!("it has no Content-Type"),
            1 => Ok(Entity {
                fields,
                body: body.to_vec(),
            }),
            n => bail!("it has {n} Content-Type fields"),
        }
    }

    /// The message: `head`, agent-mail's own fields each ending in CRLF, then this entity.
    pub fn message(&self, head: &str) -> Vec<u8> {
        [
            head.as_bytes(),
            b"MIME-Version: 1.0\r\n",
            &self.fields,
            b"\r\n",
            &self.body,
        ]
        .concat()
    }
}

/// One header field: its name, and its lines as written, the last CRLF included.
struct Field<'a> {
    name: &'a str,
    raw: &'a [u8],
}

impl<'a> Field<'a> {
    /// `header` ends in CRLF; a line starting with a space or tab continues the field above it.
    fn split(header: &'a [u8]) -> Result<Vec<Field<'a>>> {
        let mut spans: Vec<(usize, usize)> = Vec::new();
        let mut start = 0;
        while start < header.len() {
            let end = start + find(&header[start..], b"\r\n").expect("the header ends in CRLF") + 2;
            match (header[start], spans.last_mut()) {
                (b' ' | b'\t', Some(span)) => span.1 = end,
                (b' ' | b'\t', None) => bail!("its first header line is a continuation"),
                _ => spans.push((start, end)),
            }
            start = end;
        }
        spans
            .into_iter()
            .map(|(start, end)| Field::new(&header[start..end]))
            .collect()
    }

    fn new(raw: &'a [u8]) -> Result<Field<'a>> {
        let text = std::str::from_utf8(raw).expect("checked ASCII");
        let Some((name, _)) = text.split_once(':') else {
            bail!(
                "header line {} has no colon",
                text.trim_end().escape_debug()
            );
        };
        ensure!(
            !name.is_empty() && name.bytes().all(|b| (b'!'..=b'~').contains(&b)),
            "{} is not a header field name",
            name.escape_debug()
        );
        Ok(Field { name, raw })
    }

    fn value(&self) -> &str {
        let text = std::str::from_utf8(self.raw).expect("checked ASCII");
        text.split_once(':').map_or("", |(_, value)| value)
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn crlf(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + bytes.len() / 32);
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' && (i == 0 || bytes[i - 1] != b'\r') {
            out.push(b'\r');
        }
        out.push(b);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(text: &str) -> Result<Entity> {
        Entity::parse(text.as_bytes())
    }

    fn refusal(text: &str) -> String {
        format!("{:#}", entity(text).err().expect("refused"))
    }

    #[test]
    fn content_fields_and_mime_version_pass_and_mime_version_is_written_once() {
        let e = entity(
            "MIME-Version: 1.0\r\nContent-Type: multipart/alternative;\r\n\tboundary=\"b\"\r\nContent-Language: en\r\n\r\n--b--\r\n",
        )
        .unwrap();
        let message = String::from_utf8(e.message("From: a@b.cd\r\n")).unwrap();
        assert_eq!(
            message,
            "From: a@b.cd\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative;\r\n\tboundary=\"b\"\r\nContent-Language: en\r\n\r\n--b--\r\n"
        );
    }

    #[test]
    fn any_other_field_is_refused_by_name() {
        for name in [
            "From",
            "To",
            "cc",
            "BCC",
            "Sender",
            "Reply-To",
            "Subject",
            "Date",
            "Message-ID",
            "X-Anything",
        ] {
            let why = refusal(&format!(
                "Content-Type: text/plain\r\n{name}: x\r\n\r\nbody"
            ));
            assert!(why.contains(name), "{name}: {why}");
        }
        assert!(refusal("Content-Type: text/plain\r\nMIME-Version: 2.0\r\n\r\nx").contains("2.0"));
    }

    #[test]
    fn a_folded_field_cannot_smuggle_a_header() {
        let e = entity("Content-Type: text/plain;\r\n charset=utf-8\r\n\r\nBcc: not a header\r\n")
            .unwrap();
        let message = String::from_utf8(e.message("")).unwrap();
        assert!(
            message.ends_with("\r\n\r\nBcc: not a header\r\n"),
            "{message}"
        );
        assert!(refusal(" Content-Type: text/plain\r\n\r\nx").contains("continuation"));
        assert!(refusal("Content-Type: text/plain\rBcc: x@y.zz\r\n\r\nx").contains("bare CR"));
        assert!(refusal("Content-Type: text/plain\r\nnot a field\r\n\r\nx").contains("no colon"));
    }

    #[test]
    fn content_type_is_required_once() {
        assert!(refusal("Content-Language: en\r\n\r\nx").contains("no Content-Type"));
        assert!(refusal("\r\nbody").contains("no Content-Type"));
        assert!(
            refusal("Content-Type: text/plain\r\ncontent-type: text/html\r\n\r\nx")
                .contains("2 Content-Type")
        );
        assert!(refusal("Content-Type: text/plain\r\n").contains("blank line"));
        assert!(refusal("Content-Type: text/plain; name=\"é\"\r\n\r\nx").contains("ASCII"));
    }

    #[test]
    fn bare_lf_becomes_crlf_and_crlf_stays() {
        let e = entity("Content-Type: text/plain\n\nline one\nline two\r\nline three\n").unwrap();
        assert_eq!(
            String::from_utf8(e.message("")).unwrap(),
            "MIME-Version: 1.0\r\nContent-Type: text/plain\r\n\r\nline one\r\nline two\r\nline three\r\n"
        );
    }
}

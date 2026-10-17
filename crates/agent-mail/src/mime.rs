//! Pieces of a raw RFC 5322 message: base64, and RFC 2047 encoded words for free text that cannot break a line.

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            out.push(match i <= chunk.len() {
                true => char::from(BASE64[(n >> (18 - 6 * i) & 63) as usize]),
                false => '=',
            });
        }
    }
    out
}

/// A base64 body, wrapped at 76 characters (RFC 2045 §6.8).
pub fn base64_body(text: &str) -> String {
    let encoded = base64(text.as_bytes());
    let lines: Vec<&str> = encoded
        .as_bytes()
        .chunks(76)
        .map(|line| std::str::from_utf8(line).expect("base64 is ASCII"))
        .collect();
    lines.join("\r\n")
}

/// Free text for a header: runs of controls (CR and LF among them) become one space; non-ASCII becomes
/// RFC 2047 encoded words of at most 75 characters each.
pub fn text(value: &str) -> String {
    let mut clean = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\x00'..='\x1f' | '\x7f' if clean.ends_with(' ') => {}
            '\x00'..='\x1f' | '\x7f' => clean.push(' '),
            c => clean.push(c),
        }
    }
    let clean = clean.trim();
    if clean.is_ascii() {
        return clean.to_owned();
    }
    let (open, close) = ("=?UTF-8?B?", "?=");
    let room = (75 - open.len() - close.len()) / 4 * 3;
    let mut words = vec![String::new()];
    for c in clean.chars() {
        if words.last().is_some_and(|w| w.len() + c.len_utf8() > room) {
            words.push(String::new());
        }
        words.last_mut().expect("words starts non-empty").push(c);
    }
    let words: Vec<String> = words
        .iter()
        .map(|w| format!("{open}{}{close}", base64(w.as_bytes())))
        .collect();
    words.join("\r\n ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc_4648_vectors() {
        for (plain, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(plain.as_bytes()), encoded);
        }
    }

    #[test]
    fn bodies_wrap_at_76() {
        let body = base64_body(&"x".repeat(100));
        assert!(body.split("\r\n").all(|line| line.len() <= 76));
        assert_eq!(body.replace("\r\n", ""), base64("x".repeat(100).as_bytes()));
    }

    #[test]
    fn header_values_cannot_inject_lines() {
        assert_eq!(
            text("Hi\r\nBcc: victim@example.com\r\n\r\nbody"),
            "Hi Bcc: victim@example.com body"
        );
        let folded = text("é\r\nBcc: x");
        assert!(
            !folded.replace("\r\n ", "").contains(['\r', '\n']),
            "{folded}"
        );
    }

    #[test]
    fn non_ascii_becomes_encoded_words_of_at_most_75() {
        assert_eq!(text("Read: x · y"), "=?UTF-8?B?UmVhZDogeCDCtyB5?=");
        let long = text(&"é".repeat(60));
        assert!(
            long.split("\r\n ")
                .all(|w| w.len() <= 75 && w.starts_with("=?UTF-8?B?"))
        );
        assert_eq!(text("plain"), "plain");
    }
}

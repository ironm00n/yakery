//! Which Claude Code session is sending, for the subject prefix and the signature.

use std::{env, fs, path::PathBuf};

use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeSession {
    session_id: String,
    name: Option<String>,
    cwd: String,
    bridge_session_id: Option<String>,
}

impl ClaudeSession {
    /// The session whose messaging socket this process inherited, read from its registry entry.
    pub fn own() -> Option<ClaudeSession> {
        let socket = PathBuf::from(env::var_os("CLAUDE_CODE_MESSAGING_SOCKET")?);
        let name = socket.file_name()?.to_str()?;
        let id = name.strip_suffix(".sock").unwrap_or(name);
        let config = env::var_os("CLAUDE_CONFIG_DIR")
            .map(PathBuf::from)
            .or_else(|| Some(PathBuf::from(env::var_os("HOME")?).join(".claude")))?;
        let registry = config.join("sessions").join(format!("{id}.json"));
        serde_json::from_slice(&fs::read(registry).ok()?).ok()
    }
}

pub fn host() -> String {
    fs::read_to_string("/proc/sys/kernel/hostname")
        .map_or_else(|_| "unknown-host".into(), |h| h.trim().into())
}

pub fn subject_prefix(host: &str, session: Option<&ClaudeSession>) -> String {
    let name = session.and_then(|s| s.name.as_deref()).unwrap_or("shell");
    format!("[{host} · {name}]")
}

enum Line {
    Text(String),
    Command(String),
    Link(String),
}

pub struct Signature(Vec<Line>);

impl Signature {
    pub fn new(host: &str, session: Option<&ClaudeSession>) -> Signature {
        let Some(s) = session else {
            return Signature(vec![Line::Text(format!(
                "Sent from a shell on {host}, not from a Claude Code session."
            ))]);
        };
        let name = s.name.as_deref().unwrap_or("unnamed");
        let mut lines = vec![
            Line::Text(format!(
                "Claude Code session \"{name}\" on {host}, in {}",
                s.cwd
            )),
            Line::Command(format!("claude --resume {}", s.session_id)),
        ];
        lines.extend(
            s.bridge_session_id
                .as_ref()
                .map(|id| Line::Link(format!("https://claude.ai/code/{id}"))),
        );
        Signature(lines)
    }

    pub fn text(&self) -> String {
        let lines = self.0.iter().map(|line| match line {
            Line::Text(s) | Line::Command(s) | Line::Link(s) => s.as_str(),
        });
        format!("-- \n{}", lines.collect::<Vec<_>>().join("\n"))
    }

    pub fn html(&self) -> String {
        let lines = self.0.iter().map(|line| match line {
            Line::Text(s) => escape(s),
            Line::Command(s) => format!("<code>{}</code>", escape(s)),
            Line::Link(s) => format!("<a href=\"{0}\">{0}</a>", escape(s)),
        });
        format!(
            "<p>-- <br>\n{}</p>\n",
            lines.collect::<Vec<_>>().join("<br>\n")
        )
    }
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    comrak::html::escape(&mut out, s).expect("writing to a String cannot fail");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(bridge: Option<&str>) -> ClaudeSession {
        ClaudeSession {
            session_id: "abc".into(),
            name: Some("mail <port>".into()),
            cwd: "/home/u".into(),
            bridge_session_id: bridge.map(Into::into),
        }
    }

    #[test]
    fn shell_signature() {
        let sig = Signature::new("desktop", None);
        assert_eq!(
            sig.text(),
            "-- \nSent from a shell on desktop, not from a Claude Code session."
        );
        assert_eq!(subject_prefix("desktop", None), "[desktop · shell]");
    }

    #[test]
    fn session_signature_in_both_parts() {
        let s = session(Some("br1"));
        let sig = Signature::new("desktop", Some(&s));
        assert_eq!(
            sig.text(),
            "-- \nClaude Code session \"mail <port>\" on desktop, in /home/u\nclaude --resume abc\nhttps://claude.ai/code/br1"
        );
        let html = sig.html();
        assert!(html.contains("&quot;mail &lt;port&gt;&quot;"), "{html}");
        assert!(html.contains("<code>claude --resume abc</code>"));
        assert!(
            html.contains("<a href=\"https://claude.ai/code/br1\">https://claude.ai/code/br1</a>")
        );
        assert_eq!(
            subject_prefix("desktop", Some(&s)),
            "[desktop · mail <port>]"
        );
    }
}

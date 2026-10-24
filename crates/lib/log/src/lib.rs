use std::{
    env,
    fmt::Arguments,
    fs::File,
    io::{Write, stderr},
    os::{fd::AsFd, unix::fs::MetadataExt},
    path::Path,
    sync::OnceLock,
};

#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
pub enum Priority {
    Err = 3,
    Warning = 4,
    Notice = 5,
}

#[macro_export]
macro_rules! fail {
    ($($arg:tt)*) => { $crate::emit($crate::Priority::Err, format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => { $crate::emit($crate::Priority::Warning, format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! note {
    ($($arg:tt)*) => { $crate::emit($crate::Priority::Notice, format_args!($($arg)*)) };
}

#[doc(hidden)]
pub fn emit(priority: Priority, message: Arguments) {
    static TO: OnceLock<To> = OnceLock::new();
    let to = TO.get_or_init(|| {
        let stderr =
            stderr().as_fd().try_clone_to_owned().map(File::from).and_then(|f| f.metadata());
        match stderr {
            Ok(m) if journal(env::var("JOURNAL_STREAM").ok().as_deref(), (m.dev(), m.ino())) => {
                To::Journal
            }
            _ => To::Terminal(program()),
        }
    });
    let _ = stderr().lock().write_all(to.render(priority, &message.to_string()).as_bytes());
}

#[derive(Debug)]
enum To {
    Journal,
    Terminal(String),
}

impl To {
    /// The journal records each line's priority and the program, so only a terminal or a file gets the name.
    fn render(&self, priority: Priority, text: &str) -> String {
        match self {
            To::Journal => {
                text.split('\n').map(|line| format!("<{}>{line}\n", priority as u8)).collect()
            }
            To::Terminal(program) => format!("{program}: {text}\n"),
        }
    }
}

/// systemd.exec(5): `$JOURNAL_STREAM` is the journal stream's `device:inode`, which a redirected stderr no longer is.
fn journal(stream: Option<&str>, stderr: (u64, u64)) -> bool {
    let parsed =
        stream.and_then(|s| s.split_once(':')).map(|(dev, ino)| (dev.parse(), ino.parse()));
    matches!(parsed, Some((Ok(dev), Ok(ino))) if (dev, ino) == stderr)
}

fn program() -> String {
    let argv0 = env::args_os().next().unwrap_or_default();
    Path::new(&argv0).file_name().unwrap_or_default().to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_journal_gets_a_priority_per_line_and_a_terminal_the_program() {
        let journal = To::Journal.render(Priority::Warning, "refused\nsecond line");
        assert_eq!(journal, "<4>refused\n<4>second line\n");
        let terminal = To::Terminal("agentd".into()).render(Priority::Err, "refused\n  detail");
        assert_eq!(terminal, "agentd: refused\n  detail\n");
    }

    #[test]
    fn only_the_stream_journald_named_is_the_journal() {
        assert!(journal(Some("8:1234"), (8, 1234)));
        for stream in [None, Some("8:1235"), Some("9:1234"), Some("8"), Some("x:1234"), Some("")] {
            assert!(!journal(stream, (8, 1234)), "{stream:?}");
        }
    }
}

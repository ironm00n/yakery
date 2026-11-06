//! Mail about something going wrong, which Owen reads first on waking so he can change plans. He reads subjects
//! before mail, so the subject carries the news; the body says what broke, what was done and what he might change; and
//! dispatch's topic gets a copy.

use text::Address;

use crate::{policy::Login, prelude::*, show::plain};

const PREFIX: &str = "URGENT: ";
/// Shorter than this after the prefix, a subject can't say what happened.
const NEWS_CHARS: usize = 25;
/// The topic dispatch watches.
const COPY: &str = "dispatch";
/// Said after each send: `wait` skips the login's own mail, so the copy only lands in dispatch's inbox.
pub const TELL_DISPATCH: &str = "dispatch's copy waits in its inbox, but its wait skips this login's own mail, so tell dispatch too";

#[derive(clap::Args)]
pub struct Urgent {
    /// Something is going wrong: put URGENT before the subject, copy <login>+dispatch, and open the body with --broke,
    /// --done and --change. --subject is only the news, what happened and whether he must act, as in "d0149 lost at
    /// 03:10, jobs on the hourly pool".
    #[arg(long, requires_all = ["broke", "done", "change", "subject"], conflicts_with = "mime")]
    urgent: bool,
    /// What broke.
    #[arg(long, requires = "urgent", allow_hyphen_values = true)]
    broke: Option<String>,
    /// What was done about it; Markdown, so a `- ` list is fine.
    #[arg(long, requires = "urgent", allow_hyphen_values = true)]
    done: Option<String>,
    /// What he might change.
    #[arg(long, requires = "urgent", allow_hyphen_values = true)]
    change: Option<String>,
}

/// What an --urgent mail says; clap takes all of its flags or none.
pub struct Report<'a> {
    pub broke: &'a str,
    pub done: &'a str,
    pub change: &'a str,
}

impl Urgent {
    pub fn report(&self) -> Option<Report<'_>> {
        let (broke, done, change) =
            (self.broke.as_deref()?, self.done.as_deref()?, self.change.as_deref()?);
        self.urgent.then_some(Report { broke, done, change })
    }
}

/// The word URGENT in any case, where a subject opens with it past any punctuation: (what comes before it, the word,
/// the rest). None where the word runs on into a longer one (`Urgently`, `URGENT_SUBJECT`, `urgent.rs`).
fn leading(subject: &str) -> Option<(&str, &str, &str)> {
    let lead = subject.trim_start();
    let s = lead.trim_start_matches(|c: char| !c.is_alphanumeric());
    let (word, rest) = (s.get(..6)?, s.get(6..)?);
    let mut next = rest.chars();
    let runs_on = match next.next() {
        Some('_') => rest.trim_start_matches('_').starts_with(char::is_alphanumeric),
        Some('.') => next.next().is_some_and(char::is_alphabetic),
        c => c.is_some_and(char::is_alphanumeric),
    };
    (word.eq_ignore_ascii_case("urgent") && !runs_on)
        .then(|| (&lead[..lead.len() - s.len()], word, rest))
}

/// Whether a subject opens with an URGENT tag: the word in capitals (`URGENT:`, `_URGENT_ …`, `URGENT.5%`), or any
/// case set off as one (`[Urgent]`, `[Urgent 🚨]`, `(Urgent?)`, `**Urgent**`, `Urgent!`, `Urgent:`, `Urgent - …`).
/// Prose that opens with the word (`Urgent questions`, `Urgent? No.`, `(urgent-ish)`, `**Urgent questions**`) isn't.
fn tagged(subject: &str) -> bool {
    const CLOSERS: [char; 4] = [']', ')', '*', '_'];
    let Some((before, word, rest)) = leading(subject) else { return false };
    let spaced = rest.trim_start();
    // past spaces and symbols like 🚨 only, and not a `*` or `_` that opens a word (`*important*`, `_or not_`)
    let closed = rest
        .trim_start_matches(|c: char| c.is_whitespace() || !(c.is_ascii() || c.is_alphanumeric()));
    let closes = closed.starts_with(CLOSERS) && !closed[1..].starts_with(char::is_alphanumeric);
    let bracketed =
        before.contains(['[', '(', '*', '_']) && (closes || rest.starts_with([',', '/', '?']));
    word == "URGENT"
        || bracketed
        || rest.starts_with('!')
        || spaced.starts_with([':', '—', '–', '|'])
        || (spaced.len() < rest.len() && spaced.starts_with('-'))
}

/// The subject as sent. An URGENT one must come through --urgent, which takes only the news and puts URGENT before it,
/// so the news is sent as written.
pub fn subject(urgent: Option<&Report>, given: Option<&str>) -> Result<Option<String>> {
    if urgent.is_none() {
        if let Some(s) = given.filter(|s| tagged(s)) {
            bail!(
                "\"{s}\" opens with an URGENT tag. If something is going wrong, send it with --urgent and only the news \
                 as --subject: --urgent puts URGENT before it, asks for --broke, --done and --change and writes the \
                 body itself, so not with --mime. If not, word the subject without the tag"
            );
        }
        return Ok(given.map(str::to_owned));
    }
    let given = plain(given.context("--urgent needs --subject")?);
    let news = given.trim();
    ensure!(
        !news.split(|c: char| !c.is_alphanumeric()).any(|w| w.eq_ignore_ascii_case("urgent")),
        "\"{news}\" says urgent: --urgent puts URGENT at the start itself, so give only the news"
    );
    let chars = news.chars().count();
    ensure!(
        chars >= NEWS_CHARS,
        "\"{news}\" is {chars} characters, under {NEWS_CHARS}: an URGENT subject has to say what happened, since he \
         reads subjects before mail"
    );
    Ok(Some(format!("{PREFIX}{news}")))
}

/// Where each part of the Markdown sent starts, so a line of it can be named by the part it came from: (the part, the
/// line it starts on, the number its first line has in that part).
pub struct Layout(Vec<(&'static str, usize, usize)>);

impl Layout {
    /// The part a line of the Markdown sent came from, as a prefix for a message, and its number in that part.
    pub fn name(&self, line: usize) -> (&'static str, usize) {
        let &(part, start, first) = self.0.iter().rev().find(|p| p.1 <= line).unwrap_or(&self.0[0]);
        (part, line - start + first)
    }
}

/// The Markdown sent: under --urgent, a paragraph each for what broke, what was done and what he might change, then
/// stdin's.
pub fn markdown(urgent: Option<&Report>, stdin: String) -> Result<(String, Layout)> {
    let Some(u) = urgent else { return Ok((stdin, Layout(vec![("", 1, 1)]))) };
    let parts = [
        ("--broke ", "What broke", u.broke),
        ("--done ", "What I did", u.done),
        ("--change ", "What you might change", u.change),
    ];
    let (mut paragraphs, mut layout, mut line) = (vec![], vec![], 1);
    for (part, label, text) in parts {
        ensure!(!text.trim().is_empty(), "{}is empty", part);
        // the label gets a line of its own, so a definition opening the text (`Q2: …`) still opens a line
        let paragraph = format!("**{label}.**\n{}", text.trim());
        let lead = text.split_inclusive('\n').take_while(|l| l.trim().is_empty()).count();
        layout.push((part, line + 1, lead + 1));
        line += paragraph.lines().count() + 1;
        paragraphs.push(paragraph);
    }
    let blank = stdin.split_inclusive('\n').take_while(|l| l.trim().is_empty()).count();
    let rest: String = stdin.split_inclusive('\n').skip(blank).collect();
    if !rest.trim().is_empty() {
        layout.push(("stdin ", line, blank + 1));
        paragraphs.push(rest.trim_end().to_owned());
    }
    Ok((paragraphs.join("\n\n"), Layout(layout)))
}

/// The Cc sent: under --urgent, with dispatch's topic added unless the mail already goes there.
pub fn cc(
    urgent: Option<&Report>,
    login: &Login,
    to: &[Address],
    mut cc: Vec<Address>,
) -> Result<Vec<Address>> {
    if urgent.is_some() {
        let copy = login.address().subaddress(&COPY)?;
        if !to.iter().chain(&cc).any(|a| *a == copy) {
            cc.push(copy);
        }
    }
    Ok(cc)
}

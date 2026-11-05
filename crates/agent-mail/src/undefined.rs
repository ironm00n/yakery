//! References a mail leaves undefined for a reader who wasn't in the session: phrases that point outside it (an earlier
//! message, a decision the reader is said to have made) and labels it never introduces (option letters, numbered
//! items). What a reader already knows isn't in the text, so these miss some and flag some he knew; `send` warns and
//! never refuses.

/// Phrases that point at something outside the mail.
const BACK: &[&str] = &[
    "my last message",
    "my earlier message",
    "my previous message",
    "my last mail",
    "my last note",
    "still open",
    "you deferred",
    "you decided",
    "you picked",
    "you approved",
    "you asked for",
    "as before",
    "as discussed",
];

/// Words a label follows, as in "option B" or "item 19", singular or plural.
const LABELLED: &[&str] = &[
    "option", "pick", "choice", "card", "item", "decision", "row", "question", "step", "slot",
    "probe", "finding",
];

/// Letters that prefix a numbered label, as in "Q2" or "E34".
const PREFIXES: &str = "QDERST";

#[derive(Debug, PartialEq, Eq)]
pub struct Hit {
    pub line: usize,
    pub text: String,
}

/// Each undefined reference in `markdown`, in order; fenced code, inline code and paths are left out, being code.
pub fn undefined(markdown: &str) -> Vec<Hit> {
    let prose = prose(markdown);
    let mut hits = vec![];
    for (n, line) in prose.lines().enumerate() {
        let mut found: Vec<(usize, String)> = back_refs(line);
        found.extend(
            labels(line).into_iter().filter(|(at, label)| !defined(&prose, line, *at, label)),
        );
        found.sort();
        hits.extend(found.into_iter().map(|(_, text)| Hit { line: n + 1, text }));
    }
    hits
}

/// The text with fenced blocks blanked, inline code spans and every word holding a `/` dropped, keeping line numbers.
fn prose(markdown: &str) -> String {
    let mut fenced = false;
    let line = |l: &str| {
        let uncoded = l.split('`').step_by(2).collect::<Vec<_>>().join(" ");
        uncoded.split(' ').filter(|w| !w.contains('/')).collect::<Vec<_>>().join(" ")
    };
    markdown
        .lines()
        .map(|l| {
            let fence = l.trim_start().starts_with("```");
            fenced ^= fence;
            if fenced || fence { String::new() } else { line(l) }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every phrase is ASCII, so lowercasing ASCII alone keeps each offset in `lower` an offset in `line`.
fn back_refs(line: &str) -> Vec<(usize, String)> {
    let lower = line.to_ascii_lowercase();
    let mut found: Vec<(usize, String)> = BACK
        .iter()
        .flat_map(|p| lower.match_indices(p).filter(|(at, _)| bounded(&lower, *at, p.len())))
        .map(|(at, p)| (at, line[at..at + p.len()].to_string()))
        .collect();
    if let Some(two) = lower.find("the two ") {
        let sentence = &lower[two..lower[two..].find('.').map_or(lower.len(), |end| two + end)];
        if let Some(options) = sentence.find("options").filter(|o| *o < 60) {
            found.push((two, line[two..two + options + "options".len()].to_string()));
        }
    }
    found
}

/// Whether the `len` bytes at `at` in `text` stand as words of their own.
fn bounded(text: &str, at: usize, len: usize) -> bool {
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric());
    !word(text[..at].chars().next_back()) && !word(text[at + len..].chars().next())
}

/// Each word of `line` with its byte offset, a word being a run of letters and digits.
fn words(line: &str) -> Vec<(usize, &str)> {
    let mut out = vec![];
    let mut start = None;
    for (i, c) in line.char_indices().chain([(line.len(), ' ')]) {
        match (c.is_alphanumeric(), start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push((s, &line[s..i]));
                start = None;
            }
            _ => {}
        }
    }
    out
}

fn letter(w: &str) -> bool {
    w.len() == 1 && ('A'..='F').contains(&w.chars().next().unwrap_or(' '))
}

/// An option code (`1A`) or a prefixed number (`Q2`, `E34`).
fn code(w: &str) -> bool {
    let digits = |s: &str| (1..=3).contains(&s.len()) && s.chars().all(|c| c.is_ascii_digit());
    let mut chars = w.chars();
    let option = matches!(
        (chars.next(), chars.next(), chars.next()),
        (Some('1'..='9'), Some('A'..='F'), None)
    );
    option || w.strip_prefix(|c: char| PREFIXES.contains(c)).is_some_and(digits)
}

/// Option letters and numbered labels in `line`, each with its offset.
fn labels(line: &str) -> Vec<(usize, String)> {
    let words = words(line);
    let mut found = vec![];
    for (i, &(at, w)) in words.iter().enumerate() {
        let lower = w.to_lowercase();
        let after = &line[at + w.len()..];
        let labelled = LABELLED.iter().any(|l| lower == *l || lower == format!("{l}s"));
        match words.get(i + 1) {
            Some(&(next_at, next))
                if labelled && (letter(next) || next.chars().all(|c| c.is_ascii_digit())) =>
            {
                found.push((at, line[at..next_at + next.len()].to_string()));
            }
            _ if code(w) && !line[..at].ends_with(['.', ',']) => found.push((at, w.to_string())),
            _ if letter(w) && (after.starts_with([',', '?', ')']) || after.starts_with(" or ")) => {
                let previous = i.checked_sub(1).map(|p| words[p].1.to_lowercase());
                if !previous
                    .is_some_and(|p| LABELLED.iter().any(|l| p == *l || p == format!("{l}s")))
                {
                    found.push((at, w.to_string()));
                }
            }
            _ => {}
        }
    }
    found
}

/// Whether the mail introduces `label`: a gloss right after it on its line, or a line of the mail opening with it.
fn defined(prose: &str, line: &str, at: usize, label: &str) -> bool {
    let after = &line[at + label.len()..];
    let glossed =
        [" (", " —", " –", ": ", " is ", " means ", " names "].iter().any(|g| after.starts_with(g));
    let key = label.rsplit(' ').next().unwrap_or(label);
    let opens = |l: &str| {
        let l = l.trim_start().trim_start_matches(['-', '*', ' ', '(']).trim_start_matches("**");
        let rest = l.strip_prefix(key).map(|r| r.trim_start_matches("**"));
        rest.is_some_and(|r| {
            r.starts_with([':', '.', ')']) || r.starts_with(" —") || r.starts_with(" is ")
        }) && l.len() > key.len() + 2
    };
    glossed || prose.lines().any(opens)
}

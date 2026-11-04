use clap::Parser;

use super::{HIM, rows};
use crate::Cli;

fn parse(command: &str, args: &[&str]) -> Vec<String> {
    match Cli::try_parse_from(["agent-mail", command].iter().chain(args)) {
        Ok(_) => vec!["parsed".into()],
        Err(e) => e.render().to_string().split("\n\n").map(str::to_owned).collect(),
    }
}

#[test]
fn a_reply_names_its_recipients_by_to_or_all() {
    let missing = "error: the following required arguments were not provided:\n  <--to <TO>|--all>";
    rows!(|args| parse("send", args).swap_remove(0);
        &["--reply", "M1"], &["--subject", "s"] => missing;
        &["--reply", "M1", "--to", HIM], &["--reply", "M1", "--all"], &["--subject", "s", "--to", HIM] => "parsed";
    );
}

#[test]
fn send_help_shows_a_reply_needs_to_or_all() {
    let usage = "Usage: agent-mail send [OPTIONS] <--to <TO>|--all> <--subject <SUBJECT>|--reply <EMAIL_ID>>";
    assert_eq!(parse("send", &["--help"])[1], usage);
}

#[test]
fn inbox_lists_at_least_one_and_subscriptions_takes_any_session() {
    let line = |l: &str| {
        let words: Vec<&str> = l.split(' ').collect();
        parse(words[0], &words[1..]).swap_remove(0)
    };
    rows!(line;
        "inbox --limit 1", "subscriptions", "subscriptions --session a-1" => "parsed";
        "inbox --limit 0" => "error: invalid value '0' for '--limit <LIMIT>': number would be zero for non-zero type";
    );
}

use clap::Parser;

use super::{HIM, rows};
use crate::Cli;

fn send(args: &[&str]) -> Vec<String> {
    match Cli::try_parse_from(["agent-mail", "send"].iter().chain(args)) {
        Ok(_) => vec!["parsed".into()],
        Err(e) => e.render().to_string().split("\n\n").map(str::to_owned).collect(),
    }
}

#[test]
fn a_reply_names_its_recipients_by_to_or_all() {
    let missing = "error: the following required arguments were not provided:\n  <--to <TO>|--all>";
    rows!(|args| send(args).swap_remove(0);
        &["--reply", "M1"], &["--subject", "s"] => missing;
        &["--reply", "M1", "--to", HIM], &["--reply", "M1", "--all"], &["--subject", "s", "--to", HIM] => "parsed";
    );
}

#[test]
fn send_help_shows_a_reply_needs_to_or_all() {
    let usage = "Usage: agent-mail send [OPTIONS] <--to <TO>|--all> <--subject <SUBJECT>|--reply <EMAIL_ID>>";
    assert_eq!(send(&["--help"])[1], usage);
}

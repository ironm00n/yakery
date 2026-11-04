use clap::Parser;

use super::{HIM, POOL, address, login, rows};
use crate::{Cli, Command, cmd::send, mail::Current};

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

#[test]
fn a_send_with_no_watch_records_nothing_for_its_session() {
    let watched = |extra: &[&str]| {
        let argv = [&["agent-mail", "send", "--to", HIM, "--subject", "s"][..], extra].concat();
        let Command::Send(args) = Cli::try_parse_from(argv).unwrap().command else {
            panic!("not a send")
        };
        let draft = Current::test("d1");
        let watch = send::watching(&args, &login("agent@example.org"), &address(POOL), &draft);
        watch.map(|w| (w.topic.iter().map(ToString::to_string).collect::<Vec<_>>(), w.thread))
    };
    assert_eq!(watched(&[]), Some((vec!["pool".into()], vec!["d1".into()])));
    assert_eq!(watched(&["--no-watch"]), None);
}

#[test]
fn unsubscribe_names_a_topic_or_thread_and_takes_any_session() {
    let missing = "error: the following required arguments were not provided:\n  <--topic <TOPIC>|--thread <EMAIL_ID>>";
    rows!(|args| parse("unsubscribe", args).swap_remove(0);
        &["--topic", "pool"], &["--thread", "e1", "--session", "a-1"] => "parsed";
        &[], &["--session", "a-1"] => missing;
    );
}

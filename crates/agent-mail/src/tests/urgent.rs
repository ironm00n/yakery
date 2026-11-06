use clap::Parser;

use super::{
    HIM, Scratch, addresses, login, rows, shown,
    wire::{Canned, Stalwart, mail},
};
use crate::{
    Cli, Command,
    cmd::send,
    prelude::*,
    urgent::{self, Report},
};

const LOST: &str = "d0149 lost at 03:10, jobs on the hourly pool";
const BROKE: &str = "Explorer took node d0149 back at 03:10, with four lanes' jobs on it.";

fn parsed(args: &[&str]) -> Result<send::Args, String> {
    let argv = [&["agent-mail", "send", "--to", HIM][..], args].concat();
    match Cli::try_parse_from(argv) {
        Ok(Cli { command: Command::Send(args) }) => Ok(args),
        Ok(_) => Err("not a send".into()),
        Err(e) => Err(e.render().to_string().split("\n\n").next().unwrap().to_owned()),
    }
}

fn urgent_advice() -> &'static str {
    "If something is going wrong, send it with --urgent and only the news as --subject: --urgent puts URGENT before \
     it, asks for --broke, --done and --change and writes the body itself, so not with --mime. If not, word the \
     subject without the tag"
}

fn report(broke: &str) -> Report<'_> {
    Report { broke, done: "moved them", change: "nothing" }
}

#[test]
fn urgent_takes_all_three_parts_and_a_subject_and_no_mime() {
    let outcome = |args: &[&str]| {
        parsed(args).map(|a| a.urgent.report().map(|r| [r.broke, r.done, r.change].join(" ")))
    };
    let parts = ["--broke", "b", "--done", "d", "--change", "c"];
    let with = |extra: &[&'static str]| [&parts[..], extra].concat();
    let missing = |flag: &str| {
        Err(format!("error: the following required arguments were not provided:\n  {flag}"))
    };
    assert_eq!(outcome(&with(&["--urgent", "--subject", "s"])), Ok(Some("b d c".into())));
    assert_eq!(outcome(&["--subject", "s"]), Ok(None));
    assert_eq!(outcome(&with(&["--subject", "s"])), missing("--urgent"));
    assert_eq!(
        outcome(&["--urgent", "--subject", "s", "--broke", "b", "--done", "d"]),
        missing("--change <CHANGE>")
    );
    assert_eq!(outcome(&with(&["--urgent", "--reply", "M1"])), missing("--subject <SUBJECT>"));
    let dashes = [
        "--urgent",
        "--subject",
        "-40% on the hourly pool",
        "--broke",
        "- a",
        "--done",
        "- b",
        "--change",
        "-c",
    ];
    assert_eq!(outcome(&dashes), Ok(Some("- a - b -c".into())));
    assert_eq!(
        outcome(&with(&["--urgent", "--subject", "s", "--mime", "m.eml"])),
        Err("error: the argument '--urgent' cannot be used with '--mime <FILE>'".into())
    );
}

#[test]
fn only_urgent_sends_a_subject_opening_with_an_urgent_tag() {
    let refused = |s: &str| format!("error: \"{s}\" opens with an URGENT tag. {}", urgent_advice());
    rows!(|s: &str| shown(urgent::subject(None, Some(s)).map(Option::unwrap));
        "URGENT: d0149 lost" => refused("URGENT: d0149 lost");
        "  urgent - d0149 lost" => refused("  urgent - d0149 lost");
        "URGENT" => refused("URGENT");
        "[URGENT] d0149 lost" => refused("[URGENT] d0149 lost");
        "[Urgent] d0149 lost" => refused("[Urgent] d0149 lost");
        "Urgent: Olek's pick on c6 needed" => refused("Urgent: Olek's pick on c6 needed");
        "*URGENT*: d0149 lost" => refused("*URGENT*: d0149 lost");
        "🚨 URGENT: d0149 lost" => refused("🚨 URGENT: d0149 lost");
        "!! URGENT d0149 lost" => refused("!! URGENT d0149 lost");
        "\u{1}URGENT: d0149 lost" => refused("\u{1}URGENT: d0149 lost");
        "(URGENT) d0149 lost" => refused("(URGENT) d0149 lost");
        "Urgent! d0149 lost" => refused("Urgent! d0149 lost");
        "(Urgent) d0149 lost" => refused("(Urgent) d0149 lost");
        "**Urgent** d0149 lost" => refused("**Urgent** d0149 lost");
        "Urgent  - d0149 lost" => refused("Urgent  - d0149 lost");
        "[🚨 Urgent] d0149 lost" => refused("[🚨 Urgent] d0149 lost");
        "[Urgent 🚨] d0149 lost" => refused("[Urgent 🚨] d0149 lost");
        "[🚨 Urgent 🚨] d0149 lost" => refused("[🚨 Urgent 🚨] d0149 lost");
        "[Urgent/ops] d0149 lost" => refused("[Urgent/ops] d0149 lost");
        "(Urgent?) d0149 lost" => refused("(Urgent?) d0149 lost");
        "**Urgent ** d0149 lost" => refused("**Urgent ** d0149 lost");
        "URGENT.5% of hourly jobs fail" => refused("URGENT.5% of hourly jobs fail");
        "_URGENT_ d0149 lost" => refused("_URGENT_ d0149 lost");
        "__URGENT__: d0149 lost" => refused("__URGENT__: d0149 lost");
        "Urgently, a pick" => "Urgently, a pick";
        "(urgent-ish) Olek's pick on c6 needed by Friday" => "(urgent-ish) Olek's pick on c6 needed by Friday";
        "(Urgent & *important*) pick" => "(Urgent & *important*) pick";
        "**Urgent … _or not_** weekly usage" => "**Urgent … _or not_** weekly usage";
        "(\"Urgent\") label on c6 is a false alarm" => "(\"Urgent\") label on c6 is a false alarm";
        "(urgent `_private` fix) landed" => "(urgent `_private` fix) landed";
        "**Urgent questions** for Olek on c6, whenever he's up"
            => "**Urgent questions** for Olek on c6, whenever he's up";
        "[urgent.rs] one more pass" => "[urgent.rs] one more pass";
        "Urgent questions for Olek, whenever he's up" => "Urgent questions for Olek, whenever he's up";
        "Urgent? No. Just the weekly Explorer usage" => "Urgent? No. Just the weekly Explorer usage";
        "Urgent-ish: Olek's pick on c6 by Friday" => "Urgent-ish: Olek's pick on c6 by Friday";
        "urgent.rs: one more pass before deploy" => "urgent.rs: one more pass before deploy";
        "URGENT_SUBJECT regex test cases" => "URGENT_SUBJECT regex test cases";
        "URGENT__SUBJECT regex test cases" => "URGENT__SUBJECT regex test cases";
        "[ops][URGENT] d0149 lost" => "[ops][URGENT] d0149 lost";
        "2 urgent questions for when you are up" => "2 urgent questions for when you are up";
        "Re: URGENT: d0149 lost" => "Re: URGENT: d0149 lost";
        "NOT URGENT: a c6 question for when you are up" => "NOT URGENT: a c6 question for when you are up";
        "the urgent fix landed" => "the urgent fix landed";
    );
    assert_eq!(shown(urgent::subject(None, None).map(|s| format!("{s:?}"))), "None");
}

#[test]
fn an_urgent_subject_is_the_news_as_written() {
    let subject =
        |given: &str| shown(urgent::subject(Some(&report(BROKE)), Some(given)).map(Option::unwrap));
    let short = |news: &str, n: usize| {
        format!(
            "error: \"{news}\" is {n} characters, under 25: an URGENT subject has to say what happened, since he \
             reads subjects before mail"
        )
    };
    let again = |news: &str| {
        format!("error: \"{news}\" says urgent: --urgent puts URGENT at the start itself, so give only the news")
    };
    rows!(subject;
        LOST, &format!("  {LOST} ") => format!("URGENT: {LOST}");
        "-40% throughput on the hourly pool since 03:10" => "URGENT: -40% throughput on the hourly pool since 03:10";
        "../migrate.sh failed on every host since 02:10" => "URGENT: ../migrate.sh failed on every host since 02:10";
        "[CI] main red since 02:00, nothing merges tonight" => "URGENT: [CI] main red since 02:00, nothing merges tonight";
        &format!("URGENT: {LOST}") => again(&format!("URGENT: {LOST}"));
        &format!("[ops][URGENT] {LOST}") => again(&format!("[ops][URGENT] {LOST}"));
        &format!("_Urgent_ {LOST}") => again(&format!("_Urgent_ {LOST}"));
        "xb keeper down since 02:10, non-urgent lanes paused"
            => again("xb keeper down since 02:10, non-urgent lanes paused");
        "d0149 lost" => short("d0149 lost", 10);
        &format!("d0149 lost{}", "\u{1}".repeat(15)) => short("d0149 lost", 10);
        &"é".repeat(24) => short(&"é".repeat(24), 24);
    );
}

#[test]
fn a_line_of_the_body_is_named_by_the_part_it_came_from() {
    let two = Report { broke: "two\nlines", ..report(BROKE) };
    let (_, layout) = urgent::markdown(Some(&two), "\n\nfirst\nsecond".into()).unwrap();
    rows!(|line| layout.name(line);
        2 => ("--broke ", 1); 3 => ("--broke ", 2); 6 => ("--done ", 1); 9 => ("--change ", 1);
        11 => ("stdin ", 3); 12 => ("stdin ", 4);
    );
    let leading = Report { broke: "\n\nsee option B", ..report(BROKE) };
    let (_, layout) = urgent::markdown(Some(&leading), String::new()).unwrap();
    assert_eq!(layout.name(2), ("--broke ", 3));
    let (_, plain) = urgent::markdown(None, "a\nb".into()).unwrap();
    assert_eq!(plain.name(2), ("", 2));
}

#[test]
fn an_urgent_body_opens_with_its_three_parts() {
    let body =
        |stdin: &str| shown(urgent::markdown(Some(&report(BROKE)), stdin.into()).map(|(m, _)| m));
    let parts = format!(
        "**What broke.**\n{BROKE}\n\n**What I did.**\nmoved them\n\n**What you might change.**\nnothing"
    );
    assert_eq!(body(""), parts);
    assert_eq!(body("\n  the log is below\n\n"), format!("{parts}\n\n  the log is below"));
    assert_eq!(body("\n\n    a = 1\n    b = 2\n"), format!("{parts}\n\n    a = 1\n    b = 2"));
    assert_eq!(shown(urgent::markdown(None, " as typed\n".into()).map(|(m, _)| m)), " as typed\n");
    let blank = Report { done: "  ", ..report(BROKE) };
    assert_eq!(
        shown(urgent::markdown(Some(&blank), String::new()).map(|(m, _)| m)),
        "error: --done is empty"
    );
}

#[test]
fn an_urgent_mail_copies_dispatch_once() {
    let me = login("agent@example.org");
    let cc = |to: &str, cc: &str, urgent| {
        let kept = urgent::cc(urgent, &me, &addresses(to), addresses(cc)).unwrap();
        kept.iter().map(ToString::to_string).collect::<Vec<_>>().join(",")
    };
    let u = Some(&report(BROKE));
    assert_eq!(cc(HIM, "", u), "agent+dispatch@example.org");
    assert_eq!(
        cc(HIM, "colleague@example.com", u),
        "colleague@example.com,agent+dispatch@example.org"
    );
    assert_eq!(cc(HIM, "agent+dispatch@example.org", u), "agent+dispatch@example.org");
    assert_eq!(cc(&format!("{HIM},agent+dispatch@example.org"), "", u), "");
    assert_eq!(cc(HIM, "colleague@example.com", None), "colleague@example.com");
}

#[tokio::test]
async fn an_urgent_send_goes_out_as_checked_with_dispatch_copied() {
    let dir = Scratch::new("urgent");
    let mut stalwart = Stalwart::new(vec![]);
    let status = json!({ HIM: { "delivered": "yes" }, "agent+dispatch@example.org": { "delivered": "yes" } });
    stalwart.canned.extend([
        ("EmailSubmission/set", Canned::Answer(json!({ "created": { "send": { "id": "S1" } } }))),
        ("EmailSubmission/get", Canned::Answer(json!({ "list": [{ "deliveryStatus": status }] }))),
    ]);
    let (_wire, server, mail) = mail(&dir, stalwart).await;
    let parts = ["--done", "moved them", "--change", "nothing", "--topic", "pool", "--urgent"];
    let args = parsed(
        &[&parts[..], &["--subject", LOST, "--broke", BROKE]].concat(),
    );
    send::send(&args.unwrap(), async { mail }, "\n  the log\n".into()).await.unwrap();
    let server = server.borrow();
    let [draft] = &server.emails[..] else { panic!("{:?}", server.emails) };
    assert_eq!(draft["header:Subject:asRaw"], format!("URGENT: {LOST}"));
    assert_eq!(draft["header:Cc:asRaw"], "agent+dispatch@example.org");
    let text = draft["bodyValues"]["text"]["value"].as_str().unwrap();
    let opens = format!(
        "**What broke.**\n{BROKE}\n\n**What I did.**\nmoved them\n\n**What you might change.**\nnothing\n\n  the log\n\n-- \n"
    );
    assert!(text.starts_with(&opens), "{text}");
}

#[tokio::test]
async fn a_refused_subject_never_signs_in() {
    let args = parsed(&["--topic", "pool", "--subject", "URGENT: d0149 lost"]).unwrap();
    let sent = send::send(&args, async { unreachable!("signed in") }, "body".into()).await;
    assert_eq!(
        shown(sent.map(|_| "sent")),
        format!("error: \"URGENT: d0149 lost\" opens with an URGENT tag. {}", urgent_advice())
    );
}

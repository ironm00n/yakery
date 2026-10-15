use std::cell::{Cell, RefCell};

use super::{
    wire::{Canned, Shared, Stalwart, Wired, mail},
    *,
};
use crate::{
    cmd::claim::{Cursor, Scan, claim, open},
    mail::{Email, Keyword, Mail},
    out::Asked,
};

/// One round of this host's claim over the open inbox mail, under its stale ids too, as a cursor may hold one.
async fn round(mail: &Mail, server: &Shared) -> Result<Vec<Email>> {
    let scan = async || {
        let s = server.borrow();
        let current = s.emails.iter().map(|m| m["id"].as_str().unwrap()).map(|i| (i, i));
        let under = |(id, now): (&str, &str)| {
            let mut m = s.emails.iter().find(|m| m["id"] == now).unwrap().clone();
            m["id"] = json!(id);
            view::<Email>(m)
        };
        let all = current.chain(s.stale.iter().map(|(old, now)| (old.as_str(), now.as_str())));
        let open = all.map(under).filter(|m| open(&login("agent@example.org"), m));
        let (remind, due) = open.partition(|m| m.has(Keyword::Seen));
        Ok(Scan { due, remind })
    };
    claim(mail, mail.login(), scan).await.0
}

async fn claimed(mail: &Mail, server: &Shared) -> Vec<String> {
    let mut ids: Vec<String> =
        round(mail, server).await.unwrap().into_iter().map(|m| m.id).collect();
    ids.sort();
    ids
}

/// How many marks reached Stalwart, and the keywords of `id`.
fn marked(server: &Shared, id: &str) -> (usize, Vec<String>) {
    let s = server.borrow();
    let sets = s.log.iter().filter(|l| l.contains("Email/set")).count();
    let m = s.emails.iter().find(|m| m["id"] == id).unwrap();
    let mut keywords: Vec<String> = m["keywords"].as_object().unwrap().keys().cloned().collect();
    keywords.sort();
    (sets, keywords)
}

fn asking(id: &str) -> Value {
    let asks = json!([{ "email": HIM }]);
    email(id, json!({ "threadId": id, "header:Disposition-Notification-To:asAddresses": asks }))
}

/// A host signed in to an inbox of `emails`, whose marks are answered as `canned` says, in turn.
async fn inbox(
    name: &str,
    emails: Vec<Value>,
    canned: Vec<Canned>,
) -> (Scratch, Wired, Shared, Mail) {
    let (dir, mut stalwart) = (Scratch::new(name), Stalwart::new(emails));
    stalwart.canned.extend(canned.into_iter().map(|c| ("Email/set", c)));
    stalwart.session["capabilities"]["urn:ietf:params:jmap:core"]["maxCallsInRequest"] = json!(2);
    let (wire, server, mail) = mail(&dir, stalwart).await;
    (dir, wire, server, mail.unwrap())
}

#[tokio::test]
async fn a_mail_another_client_marks_seen_first_is_not_taken() {
    let b1 = email("b1", json!({ "threadId": "B" }));
    let (_dir, _wire, server, mail) =
        inbox("rival", vec![asking("a1"), b1], vec![Canned::Rival("$seen")]).await;
    assert_eq!(claimed(&mail, &server).await, ["b1"]);
    assert_eq!(marked(&server, "a1"), (1, vec!["$seen".into()]), "nor marked $mdnsent");
}

#[tokio::test]
async fn a_mark_refused_as_raced_is_not_taken() {
    let raced = json!({ "oldState": "0", "newState": "0", "notUpdated": { "a1": { "type": "forbidden" } } });
    let (_dir, _wire, server, mail) =
        inbox("forbidden", vec![email("a1", json!({}))], vec![Canned::Answer(raced)]).await;
    assert_eq!(claimed(&mail, &server).await, ["a1"]);
    assert_eq!(marked(&server, "a1"), (2, vec!["$seen".into()]), "taken on the second try");
}

#[tokio::test]
async fn a_mail_closed_between_the_snapshot_and_the_mark_is_let_go() {
    let (_dir, _wire, server, mail) =
        inbox("closed", vec![email("a1", json!({}))], vec![Canned::Rival("$handled")]).await;
    assert!(claimed(&mail, &server).await.is_empty());
    assert_eq!(marked(&server, "a1").1, ["$handled", "$seen"]);
}

#[tokio::test]
async fn a_mark_answered_with_an_error_is_taken_only_if_it_landed() {
    let thread = |t: &str| json!({ "threadId": t });
    for (written, rounds) in [(false, [vec!["b1"], vec!["a1"]]), (true, [vec!["a1", "b1"], vec![]])]
    {
        let mails = vec![email("a1", thread("A")), email("b1", thread("B"))];
        let canned = vec![Canned::Error("serverFail", written), Canned::Pass];
        let (_dir, _wire, server, mail) = inbox("errored", mails, canned).await;
        for round in rounds {
            assert_eq!(claimed(&mail, &server).await, round, "written: {written}");
        }
    }
}

#[tokio::test]
async fn a_receipt_goes_out_only_where_this_claims_mdnsent_wrote() {
    let marks = [
        ("wrote", Canned::Pass),
        ("failed", Canned::Http(500)),
        ("raced", Canned::Rival("$mdnsent")),
    ];
    for (how, mark) in marks {
        let (_dir, _wire, server, mail) =
            inbox("mdnsent", vec![asking("q1")], vec![Canned::Pass, mark]).await;
        let taken = round(&mail, &server).await.unwrap();
        let unasked = matches!(Asked::of(mail.login(), &taken[0]), Asked::Nothing);
        assert_eq!(unasked, how != "wrote", "{how}");
        assert_eq!(marked(&server, "q1").1.contains(&"$mdnsent".into()), how != "failed", "{how}");
    }
}

#[tokio::test]
async fn a_failed_mark_that_did_not_land_ends_the_round_with_its_error() {
    let (_dir, _wire, server, mail) =
        inbox("unlanded", vec![email("a1", json!({}))], vec![Canned::Http(500)]).await;
    let failed = round(&mail, &server).await.unwrap_err();
    assert!(format!("{failed:#}").contains("HTTP 500"), "{failed:#}");
    assert_eq!(marked(&server, "a1"), (1, vec![]));
}

#[tokio::test]
async fn a_claim_that_lands_but_errors_is_still_delivered() {
    let (_dir, _wire, server, mail) =
        inbox("lost", vec![email("a1", json!({}))], vec![Canned::Lost]).await;
    assert_eq!(claimed(&mail, &server).await, ["a1"]);
    assert_eq!(marked(&server, "a1"), (1, vec!["$seen".into()]));
}

#[tokio::test]
async fn a_refused_token_keeps_what_was_claimed_before_it() {
    let thread = |t: &str| json!({ "threadId": t });
    let mails = ["a1", "a2", "b1"].map(|id| email(id, thread(&id[..1]))).to_vec();
    let (_dir, _wire, server, mail) =
        inbox("refused", mails, vec![Canned::Pass, Canned::Pass, Canned::Http(401)]).await;
    assert_eq!(claimed(&mail, &server).await, ["a1", "a2"]);
    assert_eq!(claimed(&mail, &server).await, ["b1"]);
    assert!(claimed(&mail, &server).await.is_empty());
}

#[tokio::test]
async fn receipts_are_claimed_silently_and_receipt_requests_get_mdnsent() {
    let report = " multipart/report; report-type=disposition-notification; boundary=b";
    let receipt = email("r1", json!({ "header:Content-Type": report }));
    let (_dir, _wire, server, mail) = inbox("receipts", vec![receipt, asking("q1")], vec![]).await;
    assert_eq!(claimed(&mail, &server).await, ["q1"]);
    assert_eq!(
        (marked(&server, "r1").1, marked(&server, "q1").1),
        (vec!["$seen".into()], vec!["$mdnsent".into(), "$seen".into()])
    );
    assert!(claimed(&mail, &server).await.is_empty());
}

#[tokio::test]
async fn a_stale_id_and_the_current_one_claim_one_email_once() {
    let (_dir, _wire, server, mail) = inbox("stale", vec![email("a2", json!({}))], vec![]).await;
    server.borrow_mut().stale.insert("a1".into(), "a2".into());
    assert_eq!(claimed(&mail, &server).await, ["a2"]);
    assert_eq!(marked(&server, "a2"), (1, vec!["$seen".into()]));
}

/// Checks once against Email states `s0`, `s1`, ...: what changed after each state, and the full check that
/// finds `old` at `s0`. `due` is what the check leaves due, or `None` when it fails; the ids it read come back.
async fn check(
    cursor: &mut Cursor,
    changes: &[(&str, &[&str])],
    due: Option<&[&str]>,
    full: &Cell<u32>,
) -> (bool, Vec<String>) {
    let head = changes.last().map_or("s0", |c| c.0);
    let seen = RefCell::new(vec![]);
    let changed = async |since: &str| {
        let after = changes
            .iter()
            .find(|c| c.0 == since)
            .map(|c| c.1.iter().map(|i| i.to_string()).collect());
        Ok(after.map(|ids| (ids, head.to_owned())))
    };
    let everything = async || {
        full.set(full.get() + 1);
        Ok((vec!["old".to_owned()], "s0".to_owned()))
    };
    let triage = async |ids: BTreeSet<String>| {
        *seen.borrow_mut() = ids.into_iter().collect();
        Ok(((), due.context("the get failed")?.iter().map(|i| i.to_string()).collect()))
    };
    let ok = cursor.check(changed, everything, triage).await.is_ok();
    (ok, seen.take())
}

#[tokio::test]
async fn only_a_whole_check_moves_the_cursor() {
    let (mut cursor, full) = (Cursor::default(), Cell::new(0));
    let log: &[(&str, &[&str])] = &[("s0", &["m1"]), ("s1", &[])];
    for (due, want) in [
        (Some(&[][..]), (true, vec!["old"])),
        (None, (false, vec!["m1"])),
        (Some(&["m1"][..]), (true, vec!["m1"])),
        (Some(&[][..]), (true, vec!["m1"])),
        (Some(&[][..]), (true, vec![])),
    ] {
        assert_eq!(
            check(&mut cursor, log, due, &full).await,
            (want.0, want.1.iter().map(|s| s.to_string()).collect())
        );
    }
    assert_eq!(full.get(), 1);
    let (mut cursor, full) = (Cursor::default(), Cell::new(0));
    let log: &[(&str, &[&str])] = &[("s0", &[])];
    for (due, want) in [(None, false), (Some(&[][..]), true), (Some(&[][..]), true)] {
        assert_eq!(check(&mut cursor, log, due, &full).await.0, want);
    }
    assert_eq!(full.get(), 2, "a failed full check is redone in full");
}

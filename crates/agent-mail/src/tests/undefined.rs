use crate::undefined::{Hit, undefined};

fn texts(markdown: &str) -> Vec<String> {
    undefined(markdown).into_iter().map(|h| h.text).collect()
}

#[test]
fn references_outside_the_mail_are_flagged() {
    assert_eq!(
        texts("The layout question from my last message is still open, and you deferred the rest."),
        ["my last message", "still open", "you deferred"]
    );
    assert_eq!(
        texts("The opener is your call between the two framing options."),
        ["the two framing options"]
    );
}

#[test]
fn labels_the_mail_never_introduces_are_flagged() {
    assert_eq!(texts("Do you want option A, or another one?"), ["option A"]);
    assert_eq!(texts("Confirm 1A and 2C, which the branches build on."), ["1A", "2C"]);
    assert_eq!(texts("Logged for the batch: parsegen (item 19) matters more now."), ["item 19"]);
    assert_eq!(texts("Recorded as E26 to E34 in the requirements."), ["E26", "E34"]);
}

#[test]
fn labels_the_mail_introduces_pass() {
    let listed = "Two ways:\n- A: one shared layer per node\n- B: a layer per job\n\nPick A or B.";
    assert_eq!(texts(listed), Vec::<String>::new());
    assert_eq!(texts("Option B (keep the parameter) is what elab built."), Vec::<String>::new());
    assert_eq!(
        texts("1. Rebuild the cache.\n2. Rerun it.\n\nStep 2 waits on step 1."),
        Vec::<String>::new()
    );
}

#[test]
fn articles_code_and_paths_pass() {
    assert_eq!(
        texts("A run on the node took 9 s. A plan for the week is below."),
        Vec::<String>::new()
    );
    assert_eq!(
        texts("```\nstep 2 and Q3 here are code\n```\nThe log is at /tmp/runs/Q4/out."),
        Vec::<String>::new()
    );
}

#[test]
fn hits_carry_their_line() {
    assert_eq!(
        undefined("Fine line.\n\nAs before, see slot 9."),
        [Hit { line: 3, text: "As before".into() }, Hit { line: 3, text: "slot 9".into() }]
    );
}

#[test]
fn text_outside_ascii_is_read_not_choked_on() {
    assert_eq!(texts("In the λ-calculus, Γ ⊢ e under Δ is still open."), ["still open"]);
    assert_eq!(texts("\u{130}\u{e9} still open"), ["still open"]);
    assert_eq!(texts("İstanbul: the two Γ options are both fine."), ["the two Γ options"]);
}

#[test]
fn inline_code_passes() {
    assert_eq!(texts("Recorded as `E34`, run with `--step 2`."), Vec::<String>::new());
}

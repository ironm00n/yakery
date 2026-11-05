use super::*;

const TEXTS: [&str; 5] =
    ["plain", "a, b; c \\ d", "line one\nline two\n", "Ünïcödé — 会議", "trailing backslash \\"];

#[test]
fn text_reads_back_as_it_was_written() {
    for text in TEXTS {
        assert_eq!(unescape(&escape(text)), text, "{text:?}");
    }
}

#[test]
fn a_folded_line_reads_back_whole_with_no_physical_line_over_75_octets() {
    let value = format!("{}{}", "x".repeat(70), "会議".repeat(40));
    let folded = fold(&format!("SUMMARY:{value}"));
    assert!(folded.split("\r\n").all(|l| l.len() <= 75), "{folded}");
    let read = lines(&folded);
    assert_eq!(
        (read.len(), read[0].name.as_str(), read[0].value.as_str()),
        (1, "SUMMARY", &*value)
    );
}

#[test]
fn transp_reads_back_as_free_or_busy() {
    for free_time in [true, false] {
        assert_eq!(free(Some(transp(free_time))), free_time);
    }
    assert!(!free(None));
}

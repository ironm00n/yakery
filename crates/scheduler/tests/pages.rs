use super::*;
use crate::ask::{LinkView, Shown};
use crate::checked::Checked;
use crate::link::{Field, Form};

fn at(s: &str) -> Timestamp {
    s.parse().unwrap()
}

fn page(viewer: &Viewer, state: Shown, chosen: Option<&str>, problem: &str) -> String {
    let view = LinkView { title: None, duration: SignedDuration::from_mins(30), state };
    link(viewer, &view, chosen.map(at), &Form::default(), problem).into_string()
}

fn open(starts: &[&str]) -> Shown {
    Shown::Open(starts.iter().copied().map(at).collect())
}

/// No `<script`, and no `on…=` attribute.
fn no_script(page: &str) {
    assert!(!page.contains("<script"), "{page}");
    let handler = |rest: &str| {
        rest.split_once('=')
            .is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_lowercase()))
    };
    assert!(!page.match_indices(" on").any(|(i, _)| handler(&page[i + 3..])), "{page}");
}

#[test]
fn every_page_is_script_free() {
    let viewer = Viewer::new(Some("Europe/London"));
    let slots = ["2026-10-26T14:00:00Z", "2026-10-26T14:15:00Z", "2026-10-27T14:00:00Z"];
    let form = Form { name: "Ada".into(), address: "ada@example.org".into(), note: "".into() };
    let hour = Period { start: at("2026-10-26T14:00:00Z"), end: at("2026-10-26T15:00:00Z") };
    let day = Period { start: at("2026-10-26T14:00:00Z"), end: at("2026-10-26T22:00:00Z") };
    for page in [
        week(&viewer, 1, "2026-10-26".parse().unwrap(), &[(hour, Some("Work".into()))], &[day]),
        booked(&viewer, hour.start, SignedDuration::from_mins(30), &Checked::new(form).unwrap()),
        message(&viewer, "Not found", "No page here."),
    ]
    .map(|m| m.into_string())
    .into_iter()
    .chain([
        page(&viewer, open(&slots), None, ""),
        page(&viewer, open(&slots), Some(slots[1]), &Field::Address.to_string()),
        page(&viewer, Shown::Booked(hour.start), None, ""),
    ]) {
        no_script(&page);
    }
}

#[test]
fn choosing_a_slot_opens_the_form_in_a_dialog() {
    let viewer = Viewer::new(None);
    let slot = "2026-10-26T14:00:00Z";
    let closed = page(&viewer, open(&[slot]), None, "");
    assert!(
        !closed.contains("<dialog") && closed.contains(&format!("?slot={}", at(slot).as_second()))
    );
    let chosen = page(&viewer, open(&[slot]), Some(slot), "");
    for part in ["<dialog open>", r#"<form method="dialog">"#, r#"<form method="post">"#] {
        assert!(chosen.contains(part), "{part}");
    }
    let stale = page(&viewer, open(&[slot]), Some("2026-10-26T15:00:00Z"), "");
    assert!(!stale.contains("<dialog"), "a slot the link doesn't offer opens no form");
}

#[test]
fn times_print_in_the_chosen_zone_and_ride_in_every_link() {
    let slot = ["2026-10-26T14:00:00Z"];
    let london = page(&Viewer::new(Some("Europe/London")), open(&slot), None, "");
    assert!(london.contains(">14:00<") && london.contains("tz=Europe%2FLondon"));
    let unknown = page(&Viewer::new(Some("Mars/Olympus")), open(&slot), None, "");
    assert!(
        unknown.contains(">10:00<") && unknown.contains("America/New_York"),
        "the owner's zone"
    );
}

#[test]
fn the_week_marks_busy_half_hours_in_the_viewers_zone() {
    let hour = Period { start: at("2026-10-26T14:00:00Z"), end: at("2026-10-26T15:00:00Z") };
    let day = Period { start: at("2026-10-26T14:00:00Z"), end: at("2026-10-26T22:00:00Z") };
    let london = Viewer::new(Some("Europe/London"));
    let page =
        week(&london, 1, "2026-10-26".parse().unwrap(), &[(hour, Some("Work".into()))], &[day]);
    let page = page.into_string();
    assert_eq!(page.matches(r#"<td class="busy">Work</td>"#).count(), 2, "{page}");
    assert!(page.contains("<th>14:00</th>") && page.contains("<th>21:30</th>"));
    assert!(!page.contains("<th>22:00</th>") && !page.contains("<th>13:30</th>"));
}

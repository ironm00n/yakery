use super::*;
use crate::{CALENDARS, MAIL, PRINCIPALS};

const SESSION: &str = r#"{
    "username": "ada@example.org",
    "apiUrl": "https://mail.example.org/jmap/",
    "uploadUrl": "https://mail.example.org/jmap/upload/{accountId}/",
    "downloadUrl": "https://mail.example.org/jmap/download/{accountId}/{blobId}",
    "eventSourceUrl": "https://mail.example.org/jmap/eventsource/?types={types}",
    "accounts": {
        "c": { "name": "bookings@example.org" }, "b": { "name": "bookings@example.org" },
        "a": { "name": "ada@example.org", "isPersonal": true }
    },
    "primaryAccounts": { "urn:ietf:params:jmap:mail": "a", "urn:ietf:params:jmap:calendars": "a/b" },
    "capabilities": { "urn:ietf:params:jmap:core": { "maxObjectsInGet": 500, "maxObjectsInSet": 0 } }
}"#;

fn read(text: &str) -> Session {
    serde_json::from_str(text).unwrap()
}

#[test]
fn the_urls_are_reached_only_once_the_check_takes_each() {
    let on = |url: &str| {
        if url.starts_with("https://mail.example.org/") { Ok(()) } else { Err(url.to_owned()) }
    };
    let checked =
        |text: &str| read(text).checked(on).map(|u| [u.api, u.upload, u.download, u.events]);
    let urls = [
        "https://mail.example.org/jmap/",
        "https://mail.example.org/jmap/upload/{accountId}/",
        "https://mail.example.org/jmap/download/{accountId}/{blobId}",
        "https://mail.example.org/jmap/eventsource/?types={types}",
    ];
    assert_eq!(checked(SESSION), Ok(urls.map(String::from)));
    let api = SESSION.replace(r#""https://mail.example.org/jmap/""#, r#""https://evil.com/""#);
    assert_eq!(checked(&api), Err("https://evil.com/".to_owned()));
    let everywhere = SESSION.replace("https://mail.example.org/jmap/", "https://evil.com/");
    assert_eq!(checked(&everywhere), Err("https://evil.com/upload/{accountId}/".to_owned()));
}

#[test]
fn a_primary_account_must_be_named_and_an_id() {
    let session = read(SESSION);
    assert_eq!(session.primary(MAIL).unwrap(), "a");
    assert_eq!(session.primary(CALENDARS).unwrap_err().to_string(), "a/b is not a JMAP id");
    assert_eq!(
        session.primary(PRINCIPALS).unwrap_err().to_string(),
        "the JMAP session names no principals account"
    );
}

#[test]
fn an_account_is_found_by_name_the_first_by_id() {
    let session = read(SESSION);
    assert_eq!(session.account("bookings@example.org"), Some("b"));
    assert_eq!(session.account("ada@example.org"), Some("a"));
    assert_eq!(session.account("eve@example.org"), None);
    let names: Vec<&str> = session.accounts.values().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["ada@example.org", "bookings@example.org", "bookings@example.org"]);
}

#[test]
fn limits_are_at_least_1_and_100_where_unnamed() {
    assert_eq!(read(SESSION).limits(), Limits { get: 500, set: 1, calls: 100 });
    let none = read(&SESSION.replace("jmap:core", "jmap:other")).limits();
    assert_eq!(none, Limits { get: 100, set: 100, calls: 100 });
}

use super::*;
use crate::Auth;

fn endpoint() -> http::Uri {
    "https://auth.test/token".parse().unwrap()
}

#[test]
fn the_request_names_the_client_and_the_scope() {
    let client = Client { id: "agent-mail-test".into(), auth: Auth::Public };
    let form = Device::form(&client, "openid offline_access", &endpoint(), 0).unwrap();
    let expected = [("client_id", "agent-mail-test"), ("scope", "openid offline_access")];
    assert_eq!(form, expected.map(|(k, v)| (k, v.to_owned())));
}

/// RFC 8628 3.2's example.
const ANSWER: &str = r#"{"device_code":"GmRhmhcxhwAzkoEqiMEg_DnyEysNkuNhszIySk9eS","user_code":"WDJB-MJHT","verification_uri":"https://example.com/device","verification_uri_complete":"https://example.com/device?user_code=WDJB-MJHT","expires_in":1800,"interval":5}"#;

fn answered(body: &str) -> Device {
    Device::read(StatusCode::OK, body.as_bytes()).unwrap()
}

fn said(code: ErrorCode) -> OAuthError {
    OAuthError { code, description: None }
}

fn at(asked: Instant, seconds: u64) -> Instant {
    asked + Duration::from_secs(seconds)
}

#[test]
fn the_answer_names_the_codes_and_where_to_approve_them() {
    let device = answered(ANSWER);
    assert_eq!(device.device_code.expose(), "GmRhmhcxhwAzkoEqiMEg_DnyEysNkuNhszIySk9eS");
    assert_eq!(
        (device.user_code.as_str(), device.verification_uri.as_str()),
        ("WDJB-MJHT", "https://example.com/device")
    );
}

#[test]
fn an_answer_without_its_codes_lifetime_is_no_answer() {
    let body = ANSWER.replace(r#","expires_in":1800"#, "");
    assert!(matches!(Device::read(StatusCode::OK, body.as_bytes()), Err(Failed::Unacceptable(_))));
}

#[test]
fn a_pending_grant_is_polled_at_the_named_interval_or_five_seconds() {
    let asked = Instant::now();
    let mut named = answered(&ANSWER.replace(r#""interval":5"#, r#""interval":7"#)).polling(asked);
    let mut unnamed = answered(&ANSWER.replace(r#","interval":5"#, "")).polling(asked);
    let mut zero = answered(&ANSWER.replace(r#""interval":5"#, r#""interval":0"#)).polling(asked);
    let pending = || said(ErrorCode::AuthorizationPending);
    assert_eq!(named.next(pending(), asked), Ok(Duration::from_secs(7)));
    assert_eq!(unnamed.next(pending(), asked), Ok(Duration::from_secs(5)));
    assert_eq!(zero.next(pending(), asked), Ok(Duration::from_secs(5)));
}

#[test]
fn slow_down_waits_five_seconds_longer_for_that_and_every_later_poll() {
    let asked = Instant::now();
    let mut polling = answered(ANSWER).polling(asked);
    let waits = [
        ErrorCode::AuthorizationPending,
        ErrorCode::SlowDown,
        ErrorCode::AuthorizationPending,
        ErrorCode::SlowDown,
        ErrorCode::AuthorizationPending,
    ]
    .map(|code| polling.next(said(code), at(asked, 100)).unwrap().as_secs());
    assert_eq!(waits, [5, 10, 10, 15, 15]);
}

#[test]
fn no_poll_is_made_once_the_device_code_would_expire_first() {
    let asked = Instant::now();
    let mut polling = answered(ANSWER).polling(asked);
    let pending = || said(ErrorCode::AuthorizationPending);
    assert_eq!(polling.next(pending(), at(asked, 1794)), Ok(Duration::from_secs(5)));
    assert!(matches!(polling.next(pending(), at(asked, 1795)), Err(Failed::Unacceptable(_))));
}

#[test]
fn the_device_code_expires_its_lifetime_after_it_was_asked_for() {
    let asked = Instant::now();
    assert_eq!(answered(ANSWER).polling(asked).expiry(), Some(at(asked, 1800)));
    let forever = ANSWER.replace(r#""expires_in":1800"#, &format!(r#""expires_in":{}"#, u64::MAX));
    assert_eq!(answered(&forever).polling(asked).expiry(), None);
}

#[test]
fn a_denied_or_expired_grant_ends_the_polls() {
    let asked = Instant::now();
    for code in [ErrorCode::AccessDenied, ErrorCode::ExpiredToken] {
        let ended = answered(ANSWER).polling(asked).next(said(code.clone()), asked);
        assert_eq!(ended, Err(Failed::Refused(said(code))));
    }
}

#[test]
fn a_refused_client_is_a_refusal() {
    let refused = Device::read(StatusCode::UNAUTHORIZED, br#"{"error":"invalid_client"}"#);
    let error = OAuthError { code: ErrorCode::InvalidClient, description: None };
    assert_eq!(refused.err().unwrap(), Failed::Refused(error));
}

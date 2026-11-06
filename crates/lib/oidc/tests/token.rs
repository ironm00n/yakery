use super::*;
use crate::{Auth, ErrorCode, OAuthError, Pkce};

fn endpoint() -> Uri {
    "https://issuer.test/token".parse().unwrap()
}

fn public() -> Client {
    Client { id: "agent-mail-test".into(), auth: Auth::Public }
}

fn owned(fields: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
    fields.iter().map(|&(k, v)| (k, v.to_owned())).collect()
}

#[test]
fn a_refresh_names_the_client_then_the_grant() {
    let token = Secret::new("r1");
    let form = Grant::Refresh(&token).form(&public(), None, &endpoint(), 0).unwrap();
    let expected = [
        ("client_id", "agent-mail-test"),
        ("grant_type", "refresh_token"),
        ("refresh_token", "r1"),
    ];
    assert_eq!(form, owned(&expected));
}

#[test]
fn a_scope_asked_again_goes_between_the_client_and_the_grant() {
    let code = Secret::new("dc");
    let form = Grant::Device(&code).form(&public(), Some("openid"), &endpoint(), 0).unwrap();
    let expected = [
        ("client_id", "agent-mail-test"),
        ("scope", "openid"),
        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ("device_code", "dc"),
    ];
    assert_eq!(form, owned(&expected));
}

#[test]
fn a_code_carries_its_redirect_and_the_sign_ins_verifier() {
    let pkce = Pkce::kept(Secret::new("s"), Secret::new("the-verifier"));
    let code = crate::Returned::read("code=the-code&state=s").code(&pkce).ok().unwrap();
    let grant = Grant::Code { code: &code, redirect: "https://app.test/back" };
    let form = grant.form(&public(), None, &endpoint(), 0).unwrap();
    let expected = [
        ("client_id", "agent-mail-test"),
        ("grant_type", "authorization_code"),
        ("code", "the-code"),
        ("redirect_uri", "https://app.test/back"),
        ("code_verifier", "the-verifier"),
    ];
    assert_eq!(form, owned(&expected));
}

/// RFC 6749 5.1's example reply, to a request sent at 1000.
const GRANTED: &[u8] = br#"{"access_token":"2YotnFZFEjr1zCsicMWpAA","token_type":"example",
    "expires_in":3600,"refresh_token":"tGzv3JOkF0XG5Qx2TlKWIA","example_parameter":"example_value"}"#;

#[test]
fn a_token_reply_expires_counted_from_its_requests_sending() {
    let tokens = Tokens::read(StatusCode::OK, GRANTED, 1000).unwrap();
    assert_eq!(tokens.access.token.expose(), "2YotnFZFEjr1zCsicMWpAA");
    assert_eq!(tokens.access.expires_at, Some(4600));
    assert_eq!(tokens.token_type.as_deref(), Some("example"));
    assert_eq!(tokens.refresh.as_ref().map(Secret::expose), Some("tGzv3JOkF0XG5Qx2TlKWIA"));
}

#[test]
fn a_token_is_fresh_while_a_request_reaches_its_server_before_it_expires() {
    let access = Tokens::read(StatusCode::OK, GRANTED, 1000).unwrap().access;
    let reach = std::time::Duration::from_secs(30);
    assert!(access.fresh(4569, reach));
    assert!(!access.fresh(4570, reach));
}

#[test]
fn a_token_of_no_stated_expiry_is_fresh_until_refused() {
    let body = br#"{"access_token":"a"}"#;
    let tokens = Tokens::read(StatusCode::OK, body, 1000).unwrap();
    assert_eq!(
        (tokens.access.expires_at, tokens.token_type, tokens.refresh.is_none()),
        (None, None, true)
    );
    assert!(tokens.access.fresh(i64::MAX, std::time::Duration::MAX));
}

#[test]
fn an_expiry_past_the_clocks_end_is_no_reply_this_crate_takes() {
    let body = format!(r#"{{"access_token":"a","expires_in":{}}}"#, i64::MAX);
    let overflows = Tokens::read(StatusCode::OK, body.as_bytes(), 1000).err().unwrap();
    assert_eq!(
        overflows,
        Failed::Unacceptable("the token reply isn't of the shape it takes".into())
    );
}

#[test]
fn a_reply_of_another_shape_or_a_refusal_is_no_tokens() {
    let shape = Tokens::read(StatusCode::OK, br#"{"token":"a"}"#, 0).err().unwrap();
    assert_eq!(shape, Failed::Unacceptable("the token reply isn't of the shape it takes".into()));
    let pending = br#"{"error":"authorization_pending"}"#;
    let refused = Tokens::read(StatusCode::BAD_REQUEST, pending, 0).err().unwrap();
    let error = OAuthError { code: ErrorCode::AuthorizationPending, description: None };
    assert_eq!(refused, Failed::Refused(error));
}

#[test]
fn a_scope_covers_what_it_lists_in_any_order_and_nothing_else() {
    let read = "openid offline_access https://graph.microsoft.com/Calendars.Read";
    let write = "openid offline_access https://graph.microsoft.com/Calendars.ReadWrite";
    assert!(covers(read, read));
    assert!(covers(read, "https://graph.microsoft.com/Calendars.Read openid"));
    assert!(!covers(read, write));
    assert!(!covers(write, read));
    assert!(!covers(read, "openid offline_access https://graph.microsoft.com/calendars.read"));
}

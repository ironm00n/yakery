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

#[test]
fn a_token_reply_is_read_with_what_it_leaves_out_as_none() {
    let body = br#"{"access_token":"a","token_type":"Bearer","expires_in":3600}"#;
    let tokens = Tokens::read(StatusCode::OK, body).unwrap();
    assert_eq!((tokens.access.expose(), tokens.expires_in), ("a", Some(3600)));
    assert_eq!(tokens.token_type.as_deref(), Some("Bearer"));
    assert!(tokens.refresh.is_none());
}

#[test]
fn a_reply_of_another_shape_or_a_refusal_is_no_tokens() {
    let shape = Tokens::read(StatusCode::OK, br#"{"token":"a"}"#).err().unwrap();
    assert_eq!(shape, Failed::Unacceptable("the token reply isn't of the shape it takes".into()));
    let pending = br#"{"error":"authorization_pending"}"#;
    let refused = Tokens::read(StatusCode::BAD_REQUEST, pending).err().unwrap();
    let error = OAuthError { code: ErrorCode::AuthorizationPending, description: None };
    assert_eq!(refused, Failed::Refused(error));
}

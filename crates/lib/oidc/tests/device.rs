use super::*;
use crate::{Auth, ErrorCode, OAuthError};

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

#[test]
fn the_answer_names_the_codes_and_where_to_approve_them() {
    let body = br#"{"device_code":"dc","user_code":"ABCD-EFGH","verification_uri":"https://auth.test/device","expires_in":300}"#;
    let device = Device::read(StatusCode::OK, body).unwrap();
    assert_eq!(device.device_code.expose(), "dc");
    assert_eq!(
        (device.user_code.as_str(), device.verification_uri.as_str()),
        ("ABCD-EFGH", "https://auth.test/device")
    );
    assert_eq!(device.interval, None);
}

#[test]
fn a_refused_client_is_a_refusal() {
    let refused = Device::read(StatusCode::UNAUTHORIZED, br#"{"error":"invalid_client"}"#);
    let error = OAuthError { code: ErrorCode::InvalidClient, description: None };
    assert_eq!(refused.err().unwrap(), Failed::Refused(error));
}

use super::*;

#[test]
fn a_client_error_with_an_oauth_body_is_a_refusal_by_its_code() {
    let body = br#"{"error":"invalid_grant","error_description":"AADSTS70008: expired"}"#;
    let error = OAuthError {
        code: ErrorCode::InvalidGrant,
        description: Some("AADSTS70008: expired".into()),
    };
    assert_eq!(
        failure("token endpoint", http::StatusCode::BAD_REQUEST, body),
        Failed::Refused(error)
    );
}

#[test]
fn a_server_error_or_a_page_of_html_is_no_issuers_answer() {
    let oauth = br#"{"error":"invalid_grant"}"#;
    let says = "the issuer's token endpoint answered 503 Service Unavailable";
    let status = http::StatusCode::SERVICE_UNAVAILABLE;
    assert_eq!(failure("token endpoint", status, oauth), Failed::Unreached(says.into()));
    let page = b"<html>Bad Gateway</html>";
    let status = http::StatusCode::BAD_GATEWAY;
    let says = "the issuer's token endpoint answered 502 Bad Gateway";
    assert_eq!(failure("token endpoint", status, page), Failed::Unreached(says.into()));
}

#[test]
fn a_code_outside_the_rfcs_is_kept_but_never_printed() {
    let body = br#"{"error":"<script>"}"#;
    let failed = failure("token endpoint", http::StatusCode::BAD_REQUEST, body);
    let other = OAuthError { code: ErrorCode::Other("<script>".into()), description: None };
    assert_eq!(failed, Failed::Refused(other));
    assert_eq!(failed.to_string(), "the issuer refused it: a code outside OAuth's");
}

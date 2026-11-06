use std::collections::HashMap;

use super::*;

/// RFC 7636 Appendix B's verifier, and the challenge it gives.
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

fn kept() -> Pkce {
    Pkce::kept(Secret::new("the-state"), Secret::new(VERIFIER))
}

fn endpoints(authorization: &str) -> Endpoints {
    let token = "https://issuer.test/token".parse().unwrap();
    Endpoints { authorization: authorization.parse().unwrap(), token, userinfo: None, device: None }
}

fn query(url: &str) -> HashMap<String, String> {
    let (_, query) = url.split_once('?').unwrap();
    form_urlencoded::parse(query.as_bytes()).into_owned().collect()
}

#[test]
fn the_challenge_is_rfc_7636s_s256() {
    assert_eq!(kept().challenge(), CHALLENGE);
}

#[test]
fn random_ones_are_43_characters_of_base64url_and_differ() {
    let (one, two) = (Pkce::random().unwrap(), Pkce::random().unwrap());
    let base64url = |s: &Secret| {
        s.expose().len() == 43
            && s.expose().bytes().all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
    };
    assert!([one.state(), one.verifier(), two.state(), two.verifier()].into_iter().all(base64url));
    assert_ne!(one.verifier(), two.verifier());
    assert_ne!(one.state(), one.verifier());
}

#[test]
fn the_authorization_url_asks_for_a_code_with_the_s256_challenge() {
    let (pkce, at, client) = (kept(), endpoints("https://issuer.test/authorize"), "the-client");
    let url = pkce.authorization(&at, client, "https://app.test/back", "openid email").url();
    assert!(url.starts_with("https://issuer.test/authorize?"), "{url}");
    let expected = [
        ("response_type", "code"),
        ("client_id", "the-client"),
        ("redirect_uri", "https://app.test/back"),
        ("scope", "openid email"),
        ("state", "the-state"),
        ("code_challenge", CHALLENGE),
        ("code_challenge_method", "S256"),
    ];
    let expected = expected.map(|(k, v)| (k.to_owned(), v.to_owned())).into_iter().collect();
    assert_eq!(query(&url), expected);
}

#[test]
fn an_endpoint_with_a_query_of_its_own_keeps_it() {
    let (pkce, client) = (kept(), "the-client");
    let at = endpoints("https://issuer.test/authorize?p=signin");
    let url = pkce.authorization(&at, client, "https://app.test/back", "openid").url();
    assert!(url.starts_with("https://issuer.test/authorize?p=signin&response_type=code&"), "{url}");
}

#[test]
fn a_response_mode_is_asked_only_when_named() {
    let (pkce, at) = (kept(), endpoints("https://issuer.test/authorize"));
    let asked = pkce.authorization(&at, "the-client", "https://app.test/back", "openid");
    assert_eq!(query(&asked.response_mode("query").url())["response_mode"], "query");
}

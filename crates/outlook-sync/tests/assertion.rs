use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::signature::{RSA_PSS_2048_8192_SHA256, UnparsedPublicKey};
use serde_json::Value;

use super::*;

fn fixture() -> Credential {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    Credential::read(&Dir::open(&dir, &[]).unwrap()).unwrap()
}

fn decoded(part: &str) -> Value {
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).unwrap()).unwrap()
}

#[test]
fn the_assertion_is_a_ps256_jwt_with_the_certificate_thumbprint_and_a_short_life() {
    let credential = fixture();
    let now: Timestamp = "2026-11-04T12:00:00Z".parse().unwrap();
    let audience = "https://login.microsoftonline.com/organizations/oauth2/v2.0/token";
    let jwt = credential.assertion("client", audience, now).unwrap();
    let [header, claims, signature] = jwt.split('.').collect::<Vec<_>>()[..] else {
        panic!("{jwt}")
    };

    let header = decoded(header);
    assert_eq!(header["alg"], "PS256");
    assert_eq!(header["typ"], "JWT");
    // `openssl x509 -outform DER | openssl dgst -sha256 -binary`, base64url, of the fixture certificate
    assert_eq!(header["x5t#S256"], "2QumOmCuepnMN4-t42uuMwCklmq7BACrFRYLv0aKwdQ");

    let claims = decoded(claims);
    assert_eq!(
        (claims["aud"].as_str(), claims["iss"].as_str(), claims["sub"].as_str()),
        (Some(audience), Some("client"), Some("client"))
    );
    assert_eq!(claims["exp"].as_i64().unwrap() - claims["nbf"].as_i64().unwrap(), 300);
    assert_eq!(claims["iat"], now.as_second());
    assert!(claims["jti"].as_str().is_some_and(|j| j.len() == 32));

    let public =
        UnparsedPublicKey::new(&RSA_PSS_2048_8192_SHA256, credential.key.public().as_ref());
    let signed = jwt.rsplit_once('.').unwrap().0;
    public
        .verify(signed.as_bytes(), &URL_SAFE_NO_PAD.decode(signature).unwrap())
        .expect("the signature verifies");
}

#[test]
fn every_assertion_has_its_own_jti() {
    let credential = fixture();
    let now = Timestamp::now();
    let jti = |jwt: String| decoded(jwt.split('.').nth(1).unwrap())["jti"].clone();
    assert_ne!(
        jti(credential.assertion("c", "a", now).unwrap()),
        jti(credential.assertion("c", "a", now).unwrap())
    );
}

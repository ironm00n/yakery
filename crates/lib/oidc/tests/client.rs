use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;

use super::*;

/// A test key and its certificate, the same pair as outlook-sync's test fixture.
fn fixture() -> (Vec<u8>, Vec<u8>) {
    let read = |name: &str| {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
        let pem = std::fs::read(path).unwrap();
        rustls_pemfile::read_one(&mut pem.as_slice()).unwrap().unwrap()
    };
    let rustls_pemfile::Item::Pkcs8Key(key) = read("key.pem") else { panic!("no PKCS#8 key") };
    let rustls_pemfile::Item::X509Certificate(cert) = read("certificate.pem") else {
        panic!("no certificate")
    };
    (key.secret_pkcs8_der().to_vec(), cert.to_vec())
}

fn decoded(part: &str) -> Value {
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).unwrap()).unwrap()
}

#[test]
fn a_public_client_names_itself_alone() {
    let client = Client { id: "c".into(), auth: Auth::Public };
    assert_eq!(client.fields("https://issuer.test/token", 0).unwrap(), [("client_id", "c".into())]);
}

#[test]
fn the_assertion_is_a_ps256_jwt_with_the_certificate_thumbprint_and_a_short_life() {
    let (key, cert) = fixture();
    let client = Client {
        id: "client".into(),
        auth: Auth::Assertion(Box::new(Assertion::new(&key, &cert).unwrap())),
    };
    let audience = "https://login.microsoftonline.com/organizations/oauth2/v2.0/token";
    let now = 1_793_793_600;
    let fields = client.fields(audience, now).unwrap();
    let kind = "urn:ietf:params:oauth:client-assertion-type:jwt-bearer";
    assert_eq!(
        fields[..2],
        [("client_id", "client".into()), ("client_assertion_type", kind.into())]
    );
    let ("client_assertion", jwt) = &fields[2] else { panic!("{fields:?}") };
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
    assert_eq!((claims["nbf"].as_i64(), claims["iat"].as_i64()), (Some(now), Some(now)));
    assert_eq!(claims["exp"].as_i64(), Some(now + 300));
    assert!(claims["jti"].as_str().is_some_and(|j| j.len() == 32));

    let held = rustls_pki_types::CertificateDer::from(&cert[..]);
    let certificate = webpki::EndEntityCert::try_from(&held).unwrap();
    let signed = jwt.rsplit_once('.').unwrap().0;
    let alg = webpki::ring::RSA_PSS_2048_8192_SHA256_LEGACY_KEY;
    let signature = URL_SAFE_NO_PAD.decode(signature).unwrap();
    certificate
        .verify_signature(alg, signed.as_bytes(), &signature)
        .expect("it verifies under the certificate");
}

#[test]
fn two_assertions_never_share_a_jti() {
    let (key, cert) = fixture();
    let client = Client {
        id: "c".into(),
        auth: Auth::Assertion(Box::new(Assertion::new(&key, &cert).unwrap())),
    };
    let jti = |fields: Vec<(&str, String)>| {
        decoded(fields[2].1.split('.').nth(1).unwrap())["jti"].clone()
    };
    let (one, two) = (client.fields("a", 0).unwrap(), client.fields("a", 0).unwrap());
    assert_ne!(jti(one), jti(two));
}

#[test]
fn a_certificate_of_another_key_is_refused_at_construction() {
    let (key, _) = fixture();
    let other = rcgen::generate_simple_self_signed(["other.test".to_owned()]).unwrap();
    let refused = Assertion::new(&key, other.cert.der()).err().unwrap();
    assert_eq!(refused.to_string(), "the client certificate is of another key than the client key");
}

#[test]
fn the_certificate_is_kept_as_given() {
    let (key, cert) = fixture();
    assert_eq!(Assertion::new(&key, &cert).unwrap().certificate(), cert);
}

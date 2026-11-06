use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::digest::{SHA256, digest};

use super::*;

fn fixtures() -> Dir {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    Dir::open(&dir, &[]).unwrap()
}

#[test]
fn the_pem_files_give_the_fixture_certificate() {
    let certificate = read(&fixtures()).unwrap().certificate().to_vec();
    // `openssl x509 -outform DER | openssl dgst -sha256 -binary`, base64url, of the fixture certificate
    let thumbprint = URL_SAFE_NO_PAD.encode(digest(&SHA256, &certificate));
    assert_eq!(thumbprint, "2QumOmCuepnMN4-t42uuMwCklmq7BACrFRYLv0aKwdQ");
}

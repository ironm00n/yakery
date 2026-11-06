use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use private::Secret;
use tokio::time::Instant;

use super::{
    wire::{Wired, moved, reply, wired},
    *,
};
use crate::{
    auth::{Http, SCOPE, Zitadel},
    store::Tokens,
    web::Web,
};

fn page(html: &'static str) -> http::Response<reqwest::Body> {
    http::Response::builder().header("content-type", "text/html").body(html.into()).unwrap()
}

/// ZITADEL's device flow over its recorded login pages, the password's post answered with
/// `after_password`, and the token endpoint answering `polls` in turn, then pending; each poll
/// is logged at its second since the start.
fn zitadel(after_password: &'static str, polls: &[&'static str]) -> (Wired, Rc<RefCell<Vec<u64>>>) {
    let (start, polled) = (Instant::now(), Rc::new(RefCell::new(vec![])));
    let (log, mut polls) = (polled.clone(), polls.iter().copied().collect::<VecDeque<_>>());
    let device = json!({ "device_code": "dc", "user_code": "ABCD-EFGH", "expires_in": 300,
        "verification_uri": "https://auth.example.org/ui/login/device", "interval": 5 });
    let wire = wired(move |r| match (r.method().as_str(), r.url().path()) {
        ("POST", "/oauth/v2/device_authorization") => reply(200, device.to_string()),
        ("GET", "/ui/login/device") => page(include_str!("zitadel/1-user-code.html")),
        ("POST", "/ui/login/device") => page(include_str!("zitadel/2-loginname.html")),
        ("POST", "/ui/login/loginname") => page(include_str!("zitadel/3-password.html")),
        ("POST", "/ui/login/password") => page(after_password),
        ("POST", "/ui/login/mfa/prompt") => moved(302, "/ui/login/device/consent"),
        ("GET", "/ui/login/device/consent") => page(include_str!("zitadel/5-consent.html")),
        ("POST", "/ui/login/device/allowed") => page(include_str!("zitadel/6-done.html")),
        ("POST", "/oauth/v2/token") => {
            log.borrow_mut().push(start.elapsed().as_secs());
            match polls.pop_front().unwrap_or("authorization_pending") {
                "granted" => reply(200, r#"{"access_token":"t","expires_in":3600}"#),
                code => reply(400, json!({ "error": code }).to_string()),
            }
        }
        (method, path) => panic!("{method} {path}"),
    });
    (wire, polled)
}

async fn signed_in() -> Result<Tokens> {
    let http = Http(Web::new(false).unwrap());
    http.device_flow(&login("agent@example.org"), &Secret::new("pw"), SCOPE).await
}

const PASSED: &str = include_str!("zitadel/4-mfa-prompt.html");

#[tokio::test(start_paused = true)]
async fn the_grant_is_polled_at_its_interval_and_five_seconds_slower_after_slow_down() {
    let polls = ["authorization_pending", "slow_down", "authorization_pending", "granted"];
    let (_wire, polled) = zitadel(PASSED, &polls);
    assert_eq!(signed_in().await.unwrap().access.unwrap().token, Secret::new("t"));
    assert_eq!(*polled.borrow(), [0, 5, 15, 25]);
}

#[tokio::test(start_paused = true)]
async fn the_polls_end_before_the_device_code_expires_or_once_the_grant_is_denied() {
    let (wire, polled) = zitadel(PASSED, &[]);
    let expired = shown(signed_in().await.map(|_| ""));
    assert_eq!(
        expired,
        "error: device flow: the device code expires before its next poll; last page /ui/login/device/allowed"
    );
    assert_eq!((polled.borrow().len(), polled.borrow().last().copied()), (60, Some(295)));
    drop(wire);
    let (_wire, _) = zitadel(PASSED, &["authorization_pending", "access_denied"]);
    let denied = shown(signed_in().await.map(|_| ""));
    assert_eq!(
        denied,
        "error: device flow: the issuer refused it: access_denied; last page /ui/login/device/allowed"
    );
}

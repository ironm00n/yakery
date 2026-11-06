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

type Html = http::Response<reqwest::Body>;

fn page(html: impl Into<String>) -> Html {
    let page = http::Response::builder().header("content-type", "text/html");
    page.body(html.into().into()).unwrap()
}

/// A path ZITADEL answers otherwise than on the recorded walk, and how.
type Detour = (&'static str, fn() -> Html);

/// ZITADEL's device flow over its recorded login pages but for `detours`, the password's one post
/// answered with `after_password`, and the token endpoint answering `polls` in turn, then pending;
/// each poll is logged at its second since the start.
fn zitadel(
    detours: &[Detour],
    after_password: Html,
    polls: &[&'static str],
) -> (Wired, Rc<RefCell<Vec<u64>>>) {
    let (start, polled) = (Instant::now(), Rc::new(RefCell::new(vec![])));
    let (log, mut polls) = (polled.clone(), polls.iter().copied().collect::<VecDeque<_>>());
    let (mut after_password, detours) = (Some(after_password), detours.to_vec());
    let device = json!({ "device_code": "dc", "user_code": "ABCD-EFGH", "expires_in": 300,
        "verification_uri": "https://auth.example.org/ui/login/device", "interval": 5 });
    let wire = wired(move |r| match (r.method().as_str(), r.url().path()) {
        (_, path) if let Some((_, detour)) = detours.iter().find(|d| d.0 == path) => detour(),
        ("POST", "/oauth/v2/device_authorization") => reply(200, device.to_string()),
        ("GET", "/ui/login/device") => page(include_str!("zitadel/1-user-code.html")),
        ("POST", "/ui/login/device") => page(include_str!("zitadel/2-loginname.html")),
        ("POST", "/ui/login/loginname") => page(include_str!("zitadel/3-password.html")),
        ("POST", "/ui/login/password") => after_password.take().expect("one password post"),
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
    let (dir, http) = (Scratch::new("device-flow"), Http(Web::new(false).unwrap()));
    let held = dir.store("agent@example.org").held().await?;
    http.device_flow(&login("agent@example.org"), &Secret::new("pw"), SCOPE, &held).await
}

const PASSWORD: &str = include_str!("zitadel/3-password.html");

fn passed() -> Html {
    page(include_str!("zitadel/4-mfa-prompt.html"))
}

/// `page` as ZITADEL renders it for an error `id`: its error-message template inside the form.
fn erred(page: &str, id: &str) -> Html {
    let error = format!(
        r#"<div class="lgn-error dialog_content" title="{id}">
    <i class="lgn-icon-exclamation-circle-solid lgn-warn"></i>
    <p class="lgn-error-message">Password is invalid</p>
</div>
<div id="dialog_overlay" class="dialog_overlay "></div>
"#
    );
    assert_eq!(page.matches("</form>").count(), 1);
    self::page(page.replace("</form>", &format!("{error}</form>")))
}

/// ZITADEL's error page for an internal error, which has no form and gives the error's ID in its
/// text only.
const INTERNAL: &str = r#"<div class="lgn-head">
    <div class="lgn-error">
        <i class="lgn-icon-exclamation-circle-solid lgn-warn"></i>
        <p class="lgn-error-message">Internal (COMMAND-CahN2)</p>
    </div>
</div>"#;

#[tokio::test(start_paused = true)]
async fn a_page_past_the_password_that_cant_be_answered_is_a_refusal_unless_it_went_unchecked() {
    let loginname = include_str!("zitadel/2-loginname.html");
    let asks_code = r#"<form action="/ui/login/mfa/verify"><input name="code"></form>"#;
    let path = "/ui/login/password";
    let shown = |id: &str| format!("error {id:?} at {path}");
    let refused = |why: String| format!("PasswordRefused: {why}; taken as a refusal");
    let answered = |id| refused(format!("zitadel answered the password with {}", shown(id)));
    let unchecked =
        format!("zitadel failed before checking the password: {}", shown("COMMAND-Edf3g"));
    let (mut got, mut want) = (vec![], vec![]);
    for (after_password, wanted) in [
        (erred(PASSWORD, "COMMAND-3M0fs"), answered("COMMAND-3M0fs")),
        (erred(loginname, "EVENT-SDe2f"), answered("EVENT-SDe2f")),
        (erred(PASSWORD, "COMMAND-JLK35"), answered("COMMAND-JLK35")),
        (page(INTERNAL), answered("")),
        (page(PASSWORD), refused("zitadel asked for the password again without saying why".into())),
        (page(asks_code), refused(format!("zitadel asks for code at {path}"))),
        (erred(PASSWORD, "COMMAND-Edf3g"), format!("Transient: {unchecked}")),
        (reply(502, "bad gateway"), format!("Transient: {path}: HTTP 502 Bad Gateway")),
    ] {
        let (_wire, polled) = zitadel(&[], after_password, &[]);
        let e = signed_in().await.map(|_| ()).unwrap_err();
        got.push((format!("{:?}: {e:#}", Failure::of(&e)), polled.borrow().len()));
        want.push((wanted, 0));
    }
    assert_eq!(got, want);
}

fn asks_password() -> Html {
    page(PASSWORD)
}

#[tokio::test(start_paused = true)]
async fn once_zitadel_took_the_password_a_later_page_says_nothing_of_it() {
    let (_wire, polled) = zitadel(&[("/ui/login/device/consent", asks_password)], passed(), &[]);
    let e = signed_in().await.map(|_| ()).unwrap_err();
    assert_eq!(
        (format!("{:?}: {e:#}", Failure::of(&e)), polled.borrow().len()),
        ("Transient: zitadel asked for the password again without saying why".into(), 0)
    );
}

#[tokio::test(start_paused = true)]
async fn the_grant_is_polled_at_its_interval_and_five_seconds_slower_after_slow_down() {
    let polls = ["authorization_pending", "slow_down", "authorization_pending", "granted"];
    let (_wire, polled) = zitadel(&[], passed(), &polls);
    assert_eq!(signed_in().await.unwrap().access.unwrap().token, Secret::new("t"));
    assert_eq!(*polled.borrow(), [0, 5, 15, 25]);
}

#[tokio::test(start_paused = true)]
async fn the_polls_end_before_the_device_code_expires_or_once_the_grant_is_denied() {
    let (wire, polled) = zitadel(&[], passed(), &[]);
    let expired = shown(signed_in().await.map(|_| ""));
    assert_eq!(
        expired,
        "error: device flow: the device code expires before its next poll; last page /ui/login/device/allowed"
    );
    assert_eq!((polled.borrow().len(), polled.borrow().last().copied()), (60, Some(295)));
    drop(wire);
    let (_wire, _) = zitadel(&[], passed(), &["authorization_pending", "access_denied"]);
    let denied = shown(signed_in().await.map(|_| ""));
    assert_eq!(
        denied,
        "error: device flow: the issuer refused it: access_denied; last page /ui/login/device/allowed"
    );
}

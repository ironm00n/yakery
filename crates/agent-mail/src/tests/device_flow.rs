use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use private::Secret;
use reqwest::Request;
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

pub(super) fn page(html: impl Into<String>) -> Html {
    let page = http::Response::builder().header("content-type", "text/html");
    page.body(html.into().into()).unwrap()
}

/// A path ZITADEL answers otherwise than on the recorded walk, and how.
type Detour = (&'static str, fn() -> Html);

/// ZITADEL's device flow over its recorded login pages but for `detours`, the password's one post
/// answered with `after_password`, and the token endpoint answering `polls` in turn, then pending;
/// each poll is logged at its second since the start.
pub(super) fn login_pages(
    detours: &[Detour],
    after_password: Html,
    polls: &[&'static str],
) -> (impl FnMut(&Request) -> Html + 'static, Rc<RefCell<Vec<u64>>>) {
    let (start, polled) = (Instant::now(), Rc::new(RefCell::new(vec![])));
    let (log, mut polls) = (polled.clone(), polls.iter().copied().collect::<VecDeque<_>>());
    let (mut after_password, detours) = (Some(after_password), detours.to_vec());
    let device = json!({ "device_code": "dc", "user_code": "ABCD-EFGH", "expires_in": 300,
        "verification_uri": "https://auth.example.org/ui/login/device", "interval": 5 });
    let pages = move |r: &Request| match (r.method().as_str(), r.url().path()) {
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
    };
    (pages, polled)
}

fn zitadel(
    detours: &[Detour],
    after_password: Html,
    polls: &[&'static str],
) -> (Wired, Rc<RefCell<Vec<u64>>>) {
    let (pages, polled) = login_pages(detours, after_password, polls);
    (wired(pages), polled)
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

/// ZITADEL 4.19.4's templates for the steps it renders past the password, filled in by hand: what
/// each renders between the page's chrome, which has no form.
const CHANGE_PASSWORD: &str = include_str!("zitadel/rendered/change-password.html");
pub(super) const VERIFY_TOTP: &str = include_str!("zitadel/rendered/verify-totp.html");
const VERIFY_MAIL: &str = include_str!("zitadel/rendered/verify-mail.html");
const GRANT_REQUIRED: &str = include_str!("zitadel/rendered/grant-required.html");

/// The recorded MFA prompt as ZITADEL renders it when the login policy requires a second factor:
/// without its skip button.
fn mfa_required() -> Html {
    let prompt = include_str!("zitadel/4-mfa-prompt.html");
    let skip = prompt.find(r#"name="skip""#).unwrap();
    let start = prompt[..skip].rfind("<button").unwrap();
    let end = skip + prompt[skip..].find("</button>").unwrap() + "</button>".len();
    page(format!("{}{}", &prompt[..start], &prompt[end..]))
}

#[tokio::test(start_paused = true)]
async fn the_page_answering_the_password_says_by_its_handler_whether_zitadel_refused_it() {
    let loginname = include_str!("zitadel/2-loginname.html");
    let path = "/ui/login/password";
    let failed =
        |id: &str| format!("PasswordRefused: zitadel failed the password: error {id:?} at {path}");
    let unchecked = |id: &str| {
        format!(
            "Transient: zitadel answered the password with error {id:?} at {path}, which no failed check gives"
        )
    };
    let no_id = format!(
        "PasswordRefused: zitadel answered the password at {path} with no error ID; taken as a refusal"
    );
    let unfinished = |step: &str| {
        format!(
            "Unfinished: zitadel {step}, past the password; a person has to finish that step in a browser"
        )
    };
    let grant = "Login not possible. The user is required to have at least one grant on the application. \
        Please contact your administrator. (APP-asb43)";
    let (mut got, mut want) = (vec![], vec![]);
    for (after_password, wanted) in [
        (erred(PASSWORD, "COMMAND-3M0fs"), failed("COMMAND-3M0fs")),
        (erred(PASSWORD, "COMMAND-CahN2"), failed("COMMAND-CahN2")),
        (erred(loginname, "EVENT-SDe2f"), failed("EVENT-SDe2f")),
        (erred(PASSWORD, ""), no_id.clone()),
        (page(PASSWORD), no_id),
        (erred(PASSWORD, "COMMAND-JLK35"), unchecked("COMMAND-JLK35")),
        (erred(PASSWORD, "COMMAND-Edf3g"), unchecked("COMMAND-Edf3g")),
        (erred(loginname, "COMMAND-Edf3g"), unchecked("COMMAND-Edf3g")),
        (
            page(CHANGE_PASSWORD),
            unfinished(
                "asks for change-old-password, change-new-password, \
            change-password-confirmation at /ui/login/password/change",
            ),
        ),
        (page(VERIFY_TOTP), unfinished("asks for code at /ui/login/mfa/verify")),
        (page(VERIFY_MAIL), unfinished("asks for code at /ui/login/mail/verification")),
        (mfa_required(), unfinished("asks for provider at /ui/login/mfa/prompt")),
        (page(GRANT_REQUIRED), unfinished(&format!("shows {grant:?} at {path}"))),
        (reply(502, "bad gateway"), format!("Transient: {path}: HTTP 502 Bad Gateway")),
        (reply(429, "{}"), format!("Transient: {path}: HTTP 429 Too Many Requests")),
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

fn loginname() -> Html {
    page(include_str!("zitadel/2-loginname.html"))
}

fn unknown() -> Html {
    page("<p>Something happened.</p>")
}

fn restarted() -> Html {
    moved(302, "/ui/login/device")
}

#[tokio::test(start_paused = true)]
async fn the_grant_is_polled_for_only_once_the_login_pages_approve_the_device() {
    let ended =
        |path: &str| format!("the login pages ended at {path} without approving the device");
    let transient = |path| format!("Transient: {}", ended(path));
    let refused = format!("PasswordRefused: {}; taken as a refusal", ended("/ui/login/password"));
    let rows: [(&[Detour], Html, String); 4] = [
        (&[("/ui/login/loginname", loginname)], passed(), transient("/ui/login/loginname")),
        (&[], unknown(), refused),
        (&[("/ui/login/device/consent", unknown)], passed(), transient("/ui/login/device/consent")),
        (&[("/ui/login/device/allowed", restarted)], passed(), transient("/ui/login/device")),
    ];
    let (mut got, mut want) = (vec![], vec![]);
    for (detours, after_password, wanted) in rows {
        let (_wire, polled) = zitadel(detours, after_password, &[]);
        let e = signed_in().await.map(|_| ()).unwrap_err();
        got.push((format!("{:?}: {e:#}", Failure::of(&e)), polled.borrow().len()));
        want.push((wanted, 0));
    }
    assert_eq!(got, want);
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
    assert_eq!(expired, "error: device flow: the device code expires before its next poll");
    assert_eq!((polled.borrow().len(), polled.borrow().last().copied()), (60, Some(295)));
    drop(wire);
    let (_wire, _) = zitadel(&[], passed(), &["authorization_pending", "access_denied"]);
    let denied = shown(signed_in().await.map(|_| ""));
    assert_eq!(denied, "error: device flow: the issuer refused it: access_denied");
}

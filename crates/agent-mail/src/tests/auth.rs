use std::cell::RefCell;

use super::*;
use crate::{
    auth::{ADMIN_SCOPE, Auth, Form, SCOPE, Zitadel},
    private::{Secret, now},
    store::{Access, PasswordFile, Store, Tokens},
    web::Origin,
};

/// Where the first form of a page at `path` posts, with which fields, and whether it consents or asks the password.
fn post(path: &str, html: &str) -> String {
    let answers =
        [("user_code", "ABCD-EFGH"), ("loginName", "agent@example.org"), ("password", "pw")];
    let posted = Form::read(html).and_then(|f| f.post(&answers, &Origin::Auth.url(path)));
    shown(posted.map(|post| match post {
        None => "nothing".into(),
        Some(p) => {
            let short = |v: &str| {
                v.get(..8).filter(|_| v.len() > 24).map_or(v.into(), |v| format!("{v}.."))
            };
            let fields: Vec<String> =
                p.fields.iter().map(|(n, v)| format!("{n}={}", short(v))).collect();
            let flags = [(p.consents, " consents"), (p.asks_password, " asks the password")];
            let flags: String = flags.iter().filter(|f| f.0).map(|f| f.1).collect();
            format!("{} {}{flags}", p.url.path(), fields.join("&"))
        }
    }))
}

#[test]
fn a_form_posts_only_to_its_own_origin_and_never_guesses_an_answer() {
    let change =
        r#"<form action="/x"><input name="change-old-password"><input name="change-new-password">"#;
    let off = r#"<form action="https://evil.com/collect"><input name="password"></form>"#;
    let consent_off = r#"<form><button formaction="https://evil.com/allowed"></button></form>"#;
    rows!(|(path, html)| post(path, html);
        ("/ui/login/password/change", change) => "error: zitadel asks for change-old-password, \
            change-new-password at /ui/login/password/change";
        ("/ui/login/password", off) =>
            "error: refusing https://evil.com/collect: it is not on https://auth.example.org";
        ("/ui/login/password", consent_off) =>
            "error: refusing https://evil.com/allowed: it is not on https://auth.example.org";
    );
}

/// Pages recorded from ZITADEL 4.19.4's Login V1 in the VM test, at the paths they were served from.
#[test]
fn the_walk_answers_zitadels_own_login_pages() {
    let request = "authRequestID=396425044525318236";
    let at = |path: &str| path.replace("{request}", request);
    rows!(|(path, html): (&str, &str)| post(&at(path), html);
        ("/ui/login/device", include_str!("zitadel/1-user-code.html")) =>
            "/ui/login/device gorilla.csrf.Token=a0MQ1DIn..&user_code=ABCD-EFGH";
        ("/ui/login/login?{request}", include_str!("zitadel/2-loginname.html")) => at(
            "/ui/login/loginname gorilla.csrf.Token=48ZMlbnE..&{request}&loginName=agent@example.org");
        ("/ui/login/loginname", include_str!("zitadel/3-password.html")) => at("/ui/login/password \
            gorilla.csrf.Token=vOuBQMy5..&{request}&loginName=agent@example.org&password=pw asks the password");
        ("/ui/login/password", include_str!("zitadel/4-mfa-prompt.html")) =>
            at("/ui/login/mfa/prompt gorilla.csrf.Token=l278u2kA..&{request}&skip=true");
        ("/ui/login/device/{action}?{request}", include_str!("zitadel/5-consent.html")) =>
            at("/ui/login/device/allowed gorilla.csrf.Token=TI5G8lk3..&{request} consents");
        ("/ui/login/device/allowed", include_str!("zitadel/6-done.html")) => "nothing";
    );
}

#[derive(Clone, Copy, Default, PartialEq)]
enum Change {
    #[default]
    Applies,
    Refused,
    AppliedButReplyLost,
    ChangedElsewhere,
}

/// ZITADEL with one account, whose password is `password`.
#[derive(Default)]
struct Fake {
    password: RefCell<String>,
    change: Change,
    refresh_valid: Option<&'static str>,
    calls: RefCell<Vec<String>>,
}

fn granted(token: &str, refresh: Option<&str>) -> Tokens {
    let access = Access { token: Secret::new(token), expires_at: now() + 3600 };
    Tokens { access: Some(access), refresh: refresh.map(Secret::new) }
}

impl Zitadel for Fake {
    async fn device_flow(&self, _: &Login, password: &Secret, scope: &str) -> Result<Tokens> {
        self.calls.borrow_mut().push(format!("sign in {} {scope}", password.expose()));
        ensure!(
            password.expose() == *self.password.borrow(),
            Failure::PasswordRefused.with("refused")
        );
        Ok(granted(&format!("access-{}", password.expose()), Some("refresh-1")))
    }

    async fn refresh(&self, refresh: &Secret) -> Result<Option<Tokens>> {
        self.calls.borrow_mut().push(format!("refresh {}", refresh.expose()));
        Ok((Some(refresh.expose()) == self.refresh_valid)
            .then(|| granted("access-refreshed", None)))
    }

    async fn change_password(&self, token: &Secret, old: &Secret, new: &Secret) -> Result<()> {
        self.calls.borrow_mut().push(format!("change {} {}", token.expose(), old.expose()));
        let (now, outcome) = match self.change {
            Change::Applies => (new.expose(), Ok(())),
            Change::Refused => (old.expose(), Err(anyhow!("HTTP 400"))),
            Change::AppliedButReplyLost => (new.expose(), Err(anyhow!("connection reset"))),
            Change::ChangedElsewhere => ("someone else's", Err(anyhow!("connection reset"))),
        };
        *self.password.borrow_mut() = now.into();
        outcome
    }
}

fn auth(dir: &Scratch, zitadel: Fake) -> Auth<Fake> {
    let login = login("ada@agent.example.org");
    let store =
        Store { state: dir.0.join("state"), locks: dir.0.join("locks"), login: login.clone() };
    Auth { zitadel, store, login, password: PasswordFile(dir.0.join("password")) }
}

fn fixture(name: &str, file: &str, zitadel: Fake) -> (Scratch, Auth<Fake>) {
    let dir = Scratch::new(name);
    fs::write(dir.0.join("password"), file).unwrap();
    let auth = auth(&dir, zitadel);
    (dir, auth)
}

fn fake(password: &str) -> Fake {
    Fake { password: RefCell::new(password.into()), ..Fake::default() }
}

impl Auth<Fake> {
    fn calls(&self) -> Vec<String> {
        self.zitadel.calls.borrow().clone()
    }

    async fn stored(&self) -> Tokens {
        self.store.tokens(&self.store.held().await.unwrap()).unwrap()
    }

    async fn token(&self) -> Option<String> {
        self.stored().await.access.map(|a| a.token.expose().to_owned())
    }
}

#[tokio::test]
async fn access_signs_in_once_then_reuses_the_cached_token() {
    let (_dir, auth) = fixture("cache", "old\n", fake("old"));
    assert_eq!(auth.access().await.unwrap().token, Secret::new("access-old"));
    assert_eq!(auth.access().await.unwrap().token, Secret::new("access-old"));
    assert_eq!(auth.calls(), [format!("sign in old {SCOPE}")]);
}

#[tokio::test]
async fn a_rejected_token_is_refreshed_and_the_refresh_token_kept() {
    let (_dir, auth) =
        fixture("refresh", "old", Fake { refresh_valid: Some("refresh-1"), ..fake("old") });
    auth.reject(&auth.access().await.unwrap().token).await.unwrap();
    assert_eq!(auth.access().await.unwrap().token, Secret::new("access-refreshed"));
    assert_eq!(auth.stored().await.refresh, Some(Secret::new("refresh-1")));
    assert_eq!(auth.calls()[1..], ["refresh refresh-1".to_string()]);
}

#[tokio::test]
async fn a_refused_refresh_token_falls_back_to_signing_in() {
    let (_dir, auth) = fixture("refused", "old", fake("old"));
    auth.reject(&auth.access().await.unwrap().token).await.unwrap();
    auth.access().await.unwrap();
    assert_eq!(
        auth.calls()[1..],
        ["refresh refresh-1".to_string(), format!("sign in old {SCOPE}")]
    );
}

#[tokio::test]
async fn reject_leaves_a_token_another_session_replaced() {
    let (_dir, auth) = fixture("replaced", "old", fake("old"));
    auth.access().await.unwrap();
    auth.reject(&Secret::new("someone-elses")).await.unwrap();
    assert!(auth.stored().await.access.is_some());
}

#[tokio::test]
async fn rotate_changes_the_password_and_stores_tokens_for_the_new_one() {
    let (dir, auth) = fixture("rotate", "old\n", fake("old"));
    auth.rotate_password().await.unwrap();
    let new = dir.file("password").unwrap();
    let server = auth.zitadel.password.borrow().clone();
    assert_eq!((new.len(), server, dir.file("password.new")), (40, new.clone(), None));
    let calls = [
        format!("sign in old {ADMIN_SCOPE}"),
        "change access-old old".into(),
        format!("sign in {new} {SCOPE}"),
    ];
    assert_eq!(auth.calls(), calls);
    assert_eq!(auth.token().await, Some(format!("access-{new}")));
}

#[tokio::test]
async fn a_refused_change_keeps_the_old_password() {
    let (dir, auth) = fixture("refuse", "old\n", Fake { change: Change::Refused, ..fake("old") });
    assert_eq!(
        shown(auth.rotate_password().await.map(|()| "")),
        "error: the password is unchanged: HTTP 400"
    );
    assert_eq!((dir.file("password").as_deref(), dir.file("password.new")), (Some("old\n"), None));
    assert_eq!(auth.token().await.as_deref(), Some("access-old"));
}

#[tokio::test]
async fn a_change_whose_reply_was_lost_is_kept() {
    let (dir, auth) =
        fixture("lost", "old\n", Fake { change: Change::AppliedButReplyLost, ..fake("old") });
    auth.rotate_password().await.unwrap();
    let new = dir.file("password").unwrap();
    assert_eq!(
        (auth.zitadel.password.borrow().clone(), dir.file("password.new")),
        (new.clone(), None)
    );
    assert_eq!(auth.token().await, Some(format!("access-{new}")));
}

#[tokio::test]
async fn when_neither_password_works_both_files_stay() {
    let (dir, auth) =
        fixture("neither", "old\n", Fake { change: Change::ChangedElsewhere, ..fake("old") });
    let e = shown(auth.rotate_password().await.map(|()| ""));
    assert!(e.contains("neither password signs in") && e.contains("password.new"), "{e}");
    assert_eq!(
        (dir.file("password"), dir.file("password.new").map(|p| p.len())),
        (Some("old\n".into()), Some(40))
    );
}

#[tokio::test]
async fn a_refused_password_retries_with_a_staged_one_and_promotes_it() {
    let (dir, auth) = fixture("crash", "old", fake("staged"));
    fs::write(dir.0.join("password.new"), "staged").unwrap();
    assert_eq!(auth.access().await.unwrap().token, Secret::new("access-staged"));
    assert_eq!((dir.file("password").as_deref(), dir.file("password.new")), (Some("staged"), None));
}

#[tokio::test]
async fn a_refused_password_is_not_posted_again_by_any_process_until_the_file_changes() {
    let (dir, first) = fixture("wrong", "old", fake("other"));
    assert!(Failure::of(&first.access().await.err().unwrap()).permanent());
    let next_process = auth(&dir, fake("other"));
    let memo = next_process.access().await.err().unwrap();
    assert!(Failure::of(&memo).permanent());
    let said = format!("{memo:#}");
    assert!(said.ends_with("until the file changes: zitadel refused the password"), "{said}");
    assert_eq!((first.calls().len(), next_process.calls().len()), (1, 0));
    fs::write(dir.0.join("password"), "other").unwrap();
    assert!(next_process.access().await.is_ok());
}

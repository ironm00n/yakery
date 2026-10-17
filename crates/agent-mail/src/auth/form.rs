//! The first form on a ZITADEL login page, and what to post back.

use std::collections::HashMap;

use crate::origin::Pinned;
use anyhow::{Result, bail};
use lol_html::{RewriteStrSettings, element, rewrite_str};

pub struct Form {
    action: Pinned,
    hidden: Fields,
    inputs: Vec<String>,
    buttons: Vec<Button>,
}

struct Button {
    name: String,
    value: String,
    formaction: String,
}

/// Form fields in order, with `URLSearchParams.set` semantics.
#[derive(Clone, Default)]
#[cfg_attr(test, derive(PartialEq, Eq, Debug))]
pub struct Fields(Vec<(String, String)>);

impl Fields {
    fn set(&mut self, name: &str, value: &str) {
        let mut kept = false;
        self.0.retain_mut(|(n, v)| {
            if n != name {
                return true;
            }
            if !kept {
                *v = value.into();
                kept = true;
                return true;
            }
            false
        });
        if !kept {
            self.0.push((name.into(), value.into()));
        }
    }

    pub fn pairs(&self) -> &[(String, String)] {
        &self.0
    }
}

pub struct Post {
    pub url: Pinned,
    pub fields: Fields,
    /// The consent button: after this post the device code is authorized.
    pub consents: bool,
}

impl Form {
    /// Mirrors mail.ts: elements are collected from the first `<form>` until a second one opens.
    pub fn first(html: &str, page: &Pinned) -> Result<Option<Form>> {
        let mut form: Option<Form> = None;
        let mut done = false;
        rewrite_str(
            html,
            RewriteStrSettings::new().append_element_content_handler(element!(
                "form, input, button",
                |el| {
                    let attr = |name| el.get_attribute(name).unwrap_or_default();
                    match (el.tag_name().as_str(), form.as_mut()) {
                        ("form", Some(_)) => done = true,
                        ("form", None) => {
                            let action = el.get_attribute("action").filter(|a| !a.is_empty());
                            form = Some(Form {
                                action: page.join(action.as_deref().unwrap_or(page.as_str()))?,
                                hidden: Fields::default(),
                                inputs: vec![],
                                buttons: vec![],
                            });
                        }
                        (_, Some(_)) if done => {}
                        ("input", Some(form)) => match el.get_attribute("name") {
                            Some(name) if el.get_attribute("type").as_deref() == Some("hidden") => {
                                form.hidden.set(&name, &attr("value"))
                            }
                            Some(name) => form.inputs.push(name),
                            None => {}
                        },
                        ("button", Some(form)) => form.buttons.push(Button {
                            name: attr("name"),
                            value: attr("value"),
                            formaction: attr("formaction"),
                        }),
                        _ => {}
                    }
                    Ok(())
                }
            )),
        )?;
        Ok(form)
    }

    pub fn asks(&self, input: &str) -> bool {
        self.inputs.iter().any(|i| i == input)
    }

    pub fn post(&self, answers: &HashMap<&str, &str>, page: &Pinned) -> Result<Post> {
        let mut fields = self.hidden.clone();
        for name in &self.inputs {
            if let Some(answer) = answers.get(name.as_str()) {
                fields.set(name, answer);
            }
        }
        if let Some(allow) = self
            .buttons
            .iter()
            .find(|b| b.formaction.ends_with("allowed"))
        {
            return Ok(Post {
                url: page.join(&allow.formaction)?,
                fields,
                consents: true,
            });
        }
        let unanswered: Vec<&str> = self
            .inputs
            .iter()
            .map(String::as_str)
            .filter(|name| !answers.contains_key(name))
            .collect();
        if !unanswered.is_empty() {
            let Some(skip) = self.buttons.iter().find(|b| b.name == "skip") else {
                bail!(
                    "zitadel asks for {} at {}",
                    unanswered.join(", "),
                    page.path()
                );
            };
            fields.set(&skip.name, &skip.value);
        }
        Ok(Post {
            url: self.action.clone(),
            fields,
            consents: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::origin::Origin;

    fn page(path: &str) -> Pinned {
        Origin::Auth.url(path)
    }

    fn answers() -> HashMap<&'static str, &'static str> {
        HashMap::from([
            ("user_code", "ABCD-EFGH"),
            ("loginName", "agent@ironmoon.dev"),
            ("password", "pw"),
        ])
    }

    fn pairs(post: &Post) -> Vec<(&str, &str)> {
        post.fields
            .pairs()
            .iter()
            .map(|(n, v)| (n.as_str(), v.as_str()))
            .collect()
    }

    #[test]
    fn hidden_fields_and_answers_post_to_the_action() {
        let html = r#"<html><body>
            <form action="/ui/login/loginname" method="POST">
              <input type="hidden" name="gorilla.csrf.Token" value="tok+/=">
              <input type="hidden" name="authRequestID" value="">
              <input type="text" name="loginName" value="">
              <button type="submit" name="" value="">next</button>
            </form></body></html>"#;
        let at = page("/ui/login/login?authRequestID=1");
        let form = Form::first(html, &at).unwrap().unwrap();
        assert!(form.asks("loginName"));
        let post = form.post(&answers(), &at).unwrap();
        assert_eq!(
            post.url.as_str(),
            "https://auth.ironmoon.dev/ui/login/loginname"
        );
        assert!(!post.consents);
        assert_eq!(
            pairs(&post),
            [
                ("gorilla.csrf.Token", "tok+/="),
                ("authRequestID", ""),
                ("loginName", "agent@ironmoon.dev")
            ]
        );
    }

    #[test]
    fn missing_action_posts_back_to_the_page() {
        let at = page("/ui/login/device");
        let form = Form::first(r#"<form><input name="user_code"></form>"#, &at)
            .unwrap()
            .unwrap();
        let post = form.post(&answers(), &at).unwrap();
        assert_eq!(post.url.as_str(), at.as_str());
        assert_eq!(pairs(&post), [("user_code", "ABCD-EFGH")]);
    }

    #[test]
    fn unanswered_inputs_press_skip() {
        let html = r#"<form action="mfa/prompt">
              <input type="hidden" name="authRequestID" value="9">
              <input type="radio" name="provider" value="0">
              <button name="skip" value="true" formnovalidate>skip</button>
              <button name="" value="">next</button>
            </form>"#;
        let at = page("/ui/login/password");
        let post = Form::first(html, &at)
            .unwrap()
            .unwrap()
            .post(&answers(), &at)
            .unwrap();
        assert_eq!(
            post.url.as_str(),
            "https://auth.ironmoon.dev/ui/login/mfa/prompt"
        );
        assert_eq!(pairs(&post), [("authRequestID", "9"), ("skip", "true")]);
    }

    #[test]
    fn unanswered_inputs_without_skip_are_an_error() {
        let html = r#"<form action="/x"><input name="change-old-password"><input name="change-new-password"></form>"#;
        let at = page("/ui/login/password/change");
        let err = Form::first(html, &at)
            .unwrap()
            .unwrap()
            .post(&answers(), &at)
            .err()
            .unwrap();
        assert_eq!(
            err.to_string(),
            "zitadel asks for change-old-password, change-new-password at /ui/login/password/change"
        );
    }

    #[test]
    fn consent_posts_to_the_allowed_formaction() {
        let html = r#"<form action="/ui/login/device">
              <input type="hidden" name="authRequestID" value="7">
              <button formaction="/ui/login/device/denied" name="deny">deny</button>
              <button formaction="/ui/login/device/allowed" name="allow">allow</button>
            </form>"#;
        let at = page("/ui/login/device/consent");
        let post = Form::first(html, &at)
            .unwrap()
            .unwrap()
            .post(&answers(), &at)
            .unwrap();
        assert!(post.consents);
        assert_eq!(
            post.url.as_str(),
            "https://auth.ironmoon.dev/ui/login/device/allowed"
        );
        assert_eq!(pairs(&post), [("authRequestID", "7")]);
    }

    #[test]
    fn only_the_first_form_counts() {
        let html = r#"<form action="/a"><input type="hidden" name="h" value="1"></form>
            <input name="between">
            <form action="/b"><input name="password"><button name="skip" value="x"></button></form>"#;
        let at = page("/p");
        let form = Form::first(html, &at).unwrap().unwrap();
        assert!(!form.asks("password"));
        assert!(form.asks("between"));
    }

    #[test]
    fn off_origin_actions_are_refused() {
        let at = page("/ui/login/password");
        let action = r#"<form action="https://evil.com/collect"><input name="password"></form>"#;
        assert!(Form::first(action, &at).is_err());
        let consent = r#"<form action="/ui/login/device"><button formaction="https://evil.com/allowed"></button></form>"#;
        let form = Form::first(consent, &at).unwrap().unwrap();
        assert!(form.post(&answers(), &at).is_err());
    }

    #[test]
    fn no_form_is_none() {
        assert!(Form::first("<p>done</p>", &page("/")).unwrap().is_none());
    }

    #[test]
    fn set_replaces_the_first_and_drops_the_rest() {
        let mut f = Fields(vec![
            ("a".into(), "1".into()),
            ("b".into(), "2".into()),
            ("a".into(), "3".into()),
        ]);
        f.set("a", "x");
        assert_eq!(
            f,
            Fields(vec![("a".into(), "x".into()), ("b".into(), "2".into())])
        );
    }
}

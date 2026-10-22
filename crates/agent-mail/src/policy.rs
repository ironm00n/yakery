//! What is compiled in: where agent-mail signs in, where it reads mail, and whom each login may mail. The values
//! come from the build's environment, which package.nix sets from yakery's `lib/ironmoon.nix`.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};

use crate::{address::Address, identity::Login};

pub const SCOPE: &str = "openid email profile offline_access";
pub const ZITADEL_API_SCOPE: &str = "urn:zitadel:iam:org:project:id:zitadel:aud";

macro_rules! deployment {
    ($($(#[$doc:meta])* $vis:vis $name:ident = $var:literal, test $fixture:literal;)*) => {$(
        $(#[$doc])*
        #[cfg(not(test))]
        $vis const $name: &str = env!($var);
        $(#[$doc])*
        #[cfg(test)]
        $vis const $name: &str = $fixture;
        const _: () = assert!(!$name.is_empty(), concat!($var, " is empty"));
    )*};
}

deployment! {
    pub AUTH_HOST = "AGENT_MAIL_AUTH_HOST", test "auth.example.org";
    pub CLIENT_ID = "AGENT_MAIL_CLIENT_ID", test "agent-mail-test";
    /// jmap-client discovers the session at `/.well-known/jmap`, which Stalwart redirects to `/jmap/session`.
    pub JMAP_HOST = "AGENT_MAIL_JMAP_HOST", test "mail.example.org";
    DOMAIN = "AGENT_MAIL_DOMAIN", test "example.org";
    AGENT_DOMAIN = "AGENT_MAIL_AGENT_DOMAIN", test "agent.example.org";
    SHARED_LOCAL_PART = "AGENT_MAIL_SHARED_LOCAL_PART", test "agent";
}

/// Comma-separated addresses outside the deployment's domains that the shared login may still mail: the secrets
/// flake's `data.mail.agentCorrespondents`, the list Stalwart lets the agents reach.
#[cfg(not(test))]
const CORRESPONDENTS: &str = env!("AGENT_MAIL_CORRESPONDENTS");
#[cfg(test)]
const CORRESPONDENTS: &str = "";

/// More of them, each for mail from one topic address alone: the secrets flake's `data.mail.agentCorrespondentsFor`,
/// a JSON object from that address to its list. Stalwart can't tell the shared login's topics apart; this does.
#[cfg(not(test))]
const CORRESPONDENTS_FOR: &str = env!("AGENT_MAIL_CORRESPONDENTS_FOR");
#[cfg(test)]
const CORRESPONDENTS_FOR: &str = r#"{"agent+unboca@example.org": ["colleague@example.com"]}"#;

/// `<shared local part>@<domain>`, the login every Claude session shares.
fn is_shared(login: &Login) -> bool {
    let address = login.address();
    address.local().eq_ignore_ascii_case(SHARED_LOCAL_PART)
        && address.domain().eq_ignore_ascii_case(DOMAIN)
}

fn restriction(login: &Login) -> Option<[&'static str; 2]> {
    is_shared(login).then_some([DOMAIN, AGENT_DOMAIN])
}

/// The login's own address as a sender. The shared login has none: its mail goes out only from a topic
/// subaddress, so a reply reaches the session waiting on that topic.
pub fn bare_sender(login: &Login) -> Result<Address> {
    if is_shared(login) {
        bail!("{login} is shared by every session, so it sends only from a topic subaddress");
    }
    Ok(login.address().clone())
}

fn compiled(a: &str) -> Result<Address> {
    a.parse()
        .with_context(|| format!("compiled-in address \"{a}\""))
}

/// Whom mail from `from` may go to outside the deployment's domains: everyone on `list`, and whoever `by_sender`
/// lists for `from`. One malformed entry, for any sender, refuses all mail.
fn correspondents(from: &Address, list: &str, by_sender: &str) -> Result<Vec<Address>> {
    let mut all = list
        .split(',')
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(compiled)
        .collect::<Result<Vec<_>>>()?;
    let by_sender: BTreeMap<String, Vec<String>> =
        serde_json::from_str(by_sender).context("compiled-in correspondents by sender")?;
    for (sender, list) in &by_sender {
        let list = list
            .iter()
            .map(|a| compiled(a))
            .collect::<Result<Vec<_>>>()?;
        if compiled(sender)? == *from {
            all.extend(list);
        }
    }
    Ok(all)
}

/// The `To` and `Cc` of a mail; the only way to build one checks them against the login's restriction on mail from
/// `from`.
pub struct Recipients {
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
}

impl Recipients {
    pub fn check(
        login: &Login,
        from: &Address,
        to: Vec<Address>,
        cc: Vec<Address>,
    ) -> Result<Recipients> {
        let correspondents = correspondents(from, CORRESPONDENTS, CORRESPONDENTS_FOR)?;
        Self::check_against(login, from, to, cc, &correspondents)
    }

    fn check_against(
        login: &Login,
        from: &Address,
        to: Vec<Address>,
        cc: Vec<Address>,
        correspondents: &[Address],
    ) -> Result<Recipients> {
        if let Some(domains) = restriction(login) {
            let outside: Vec<String> = to
                .iter()
                .chain(&cc)
                .filter(|a| {
                    !domains.iter().any(|d| d.eq_ignore_ascii_case(a.domain()))
                        && !correspondents.contains(a)
                })
                .map(Address::to_string)
                .collect();
            if !outside.is_empty() {
                let allowed = if correspondents.is_empty() {
                    domains.join(" and ")
                } else {
                    format!("{} and its correspondents", domains.join(", "))
                };
                bail!("{from} may mail only {allowed}, not {}", outside.join(", "));
            }
        }
        Ok(Recipients { to, cc })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tooling() -> Address {
        "agent+tooling@example.org".parse().unwrap()
    }

    fn check(login: &str, to: &str) -> Result<Recipients> {
        Recipients::check_against(
            &login.parse().unwrap(),
            &tooling(),
            vec![to.parse().unwrap()],
            vec![],
            &[],
        )
    }

    #[test]
    fn the_host_login_mails_only_allowlisted_domains() {
        for ok in [
            "owner@example.org",
            "x@AGENT.example.org",
            "agent+t@Example.Org",
        ] {
            assert!(check("agent@example.org", ok).is_ok(), "{ok}");
            assert!(check("Agent@Example.org", ok).is_ok(), "{ok}");
        }
        for bad in [
            "someone@example.com",
            "x@evil-example.org",
            "x@example.org.example.com",
            "x@sub.agent.example.org",
        ] {
            assert!(check("agent@example.org", bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn vm_agent_logins_mail_anyone() {
        assert!(check("ada@agent.example.org", "someone@example.com").is_ok());
    }

    #[test]
    fn only_the_shared_local_part_at_the_domain_is_shared() {
        for other in [
            "agent@agent.example.org",
            "agents@example.org",
            "agent@example.com",
        ] {
            let login: Login = other.parse().unwrap();
            assert!(!is_shared(&login) && bare_sender(&login).is_ok(), "{other}");
        }
        assert!(bare_sender(&"AGENT@example.ORG".parse().unwrap()).is_err());
    }

    #[test]
    fn cc_is_checked_too() {
        let login = "agent@example.org".parse().unwrap();
        let err = Recipients::check_against(
            &login,
            &tooling(),
            vec!["owner@example.org".parse().unwrap()],
            vec!["someone@example.com".parse().unwrap()],
            &[],
        )
        .err()
        .unwrap();
        assert_eq!(
            err.to_string(),
            "agent+tooling@example.org may mail only example.org and agent.example.org, not someone@example.com"
        );
    }

    #[test]
    fn a_correspondent_is_allowed_and_nobody_else_at_its_domain() {
        let login = "agent@example.org".parse().unwrap();
        let listed =
            correspondents(&tooling(), " pen.pal@example.com , other@example.net", "{}").unwrap();
        let check = |a: &str| {
            Recipients::check_against(
                &login,
                &tooling(),
                vec![a.parse().unwrap()],
                vec![],
                &listed,
            )
        };
        assert!(check("Pen.Pal@Example.com").is_ok());
        assert_eq!(
            check("stranger@example.com").err().unwrap().to_string(),
            "agent+tooling@example.org may mail only example.org, agent.example.org and its correspondents, not stranger@example.com"
        );
    }

    #[test]
    fn a_topic_mails_the_correspondents_granted_to_it_and_no_other_topic_does() {
        let login = "agent@example.org".parse().unwrap();
        let check = |from: &str| {
            Recipients::check(
                &login,
                &from.parse().unwrap(),
                vec!["Colleague@Example.com".parse().unwrap()],
                vec![],
            )
        };
        assert!(check("agent+unboca@example.org").is_ok());
        assert!(check("Agent+Unboca@Example.org").is_ok());
        assert_eq!(
            check("agent+tooling@example.org")
                .err()
                .unwrap()
                .to_string(),
            "agent+tooling@example.org may mail only example.org and agent.example.org, not Colleague@Example.com"
        );
    }

    #[test]
    fn a_malformed_correspondent_refuses_everything() {
        let from = tooling();
        assert!(correspondents(&from, "ok@example.com,not an address", "{}").is_err());
        assert!(correspondents(&from, "", "{}").unwrap().is_empty());
        for by_sender in [
            r#"{"agent+x@example.org": ["not an address"]}"#,
            r#"{"not an address": ["ok@example.com"]}"#,
            r#"["ok@example.com"]"#,
        ] {
            assert!(correspondents(&from, "", by_sender).is_err(), "{by_sender}");
        }
    }
}

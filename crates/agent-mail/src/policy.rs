//! What is compiled in: where agent-mail signs in, where it reads mail, and whom each login may mail.

use anyhow::{Context, Result, bail};

use crate::{address::Address, identity::Login};

pub const AUTH_HOST: &str = "auth.ironmoon.dev";
pub const CLIENT_ID: &str = "395450175197396579";
pub const SCOPE: &str = "openid email profile offline_access";
pub const ZITADEL_API_SCOPE: &str = "urn:zitadel:iam:org:project:id:zitadel:aud";
/// jmap-client discovers the session at `/.well-known/jmap`, which Stalwart redirects to `/jmap/session`.
pub const JMAP_HOST: &str = "mail.ironmoon.dev";

/// Logins that may mail only these domains and the correspondents; every other login mails anyone.
const RESTRICTED: &[(&str, &[&str])] = &[(
    "agent@ironmoon.dev",
    &["ironmoon.dev", "agent.ironmoon.dev"],
)];

/// Comma-separated addresses outside those domains that a restricted login may still mail. package.nix sets it
/// from the secrets flake's `data.mail.agentCorrespondents`, the list Stalwart lets the agents reach.
const CORRESPONDENTS: &str = match option_env!("AGENT_MAIL_CORRESPONDENTS") {
    Some(list) => list,
    None => "",
};

fn correspondents(list: &str) -> Result<Vec<Address>> {
    list.split(',')
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(|a| {
            a.parse()
                .with_context(|| format!("compiled-in correspondent \"{a}\""))
        })
        .collect()
}

/// The `To` and `Cc` of a mail; the only way to build one checks them against the login's restriction.
pub struct Recipients {
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
}

impl Recipients {
    pub fn check(login: &Login, to: Vec<Address>, cc: Vec<Address>) -> Result<Recipients> {
        Self::check_against(login, to, cc, &correspondents(CORRESPONDENTS)?)
    }

    fn check_against(
        login: &Login,
        to: Vec<Address>,
        cc: Vec<Address>,
        correspondents: &[Address],
    ) -> Result<Recipients> {
        let login_text = login.to_string();
        if let Some((_, domains)) = RESTRICTED
            .iter()
            .find(|(l, _)| l.eq_ignore_ascii_case(&login_text))
        {
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
                bail!(
                    "{login} may mail only {allowed}, not {}",
                    outside.join(", ")
                );
            }
        }
        Ok(Recipients { to, cc })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(login: &str, to: &str) -> Result<Recipients> {
        Recipients::check_against(
            &login.parse().unwrap(),
            vec![to.parse().unwrap()],
            vec![],
            &[],
        )
    }

    #[test]
    fn the_host_login_mails_only_allowlisted_domains() {
        for ok in [
            "ironmoon@ironmoon.dev",
            "x@AGENT.ironmoon.dev",
            "agent+t@IronMoon.Dev",
        ] {
            assert!(check("agent@ironmoon.dev", ok).is_ok(), "{ok}");
            assert!(check("Agent@IronMoon.dev", ok).is_ok(), "{ok}");
        }
        for bad in [
            "someone@example.com",
            "x@evil-ironmoon.dev",
            "x@ironmoon.dev.example.com",
            "x@sub.agent.ironmoon.dev",
        ] {
            assert!(check("agent@ironmoon.dev", bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn vm_agent_logins_mail_anyone() {
        assert!(check("ada@agent.ironmoon.dev", "someone@example.com").is_ok());
    }

    #[test]
    fn cc_is_checked_too() {
        let login = "agent@ironmoon.dev".parse().unwrap();
        let err = Recipients::check_against(
            &login,
            vec!["ironmoon@ironmoon.dev".parse().unwrap()],
            vec!["someone@example.com".parse().unwrap()],
            &[],
        )
        .err()
        .unwrap();
        assert_eq!(
            err.to_string(),
            "agent@ironmoon.dev may mail only ironmoon.dev and agent.ironmoon.dev, not someone@example.com"
        );
    }

    #[test]
    fn a_correspondent_is_allowed_and_nobody_else_at_its_domain() {
        let login = "agent@ironmoon.dev".parse().unwrap();
        let listed = correspondents(" pen.pal@example.com , other@example.org").unwrap();
        let to = |a: &str| vec![a.parse().unwrap()];
        assert!(
            Recipients::check_against(&login, to("Pen.Pal@Example.com"), vec![], &listed).is_ok()
        );
        let err = Recipients::check_against(&login, to("stranger@example.com"), vec![], &listed)
            .err()
            .unwrap();
        assert_eq!(
            err.to_string(),
            "agent@ironmoon.dev may mail only ironmoon.dev, agent.ironmoon.dev and its correspondents, not stranger@example.com"
        );
    }

    #[test]
    fn a_malformed_correspondent_refuses_everything() {
        assert!(correspondents("ok@example.com,not an address").is_err());
        assert!(correspondents("").unwrap().is_empty());
    }
}

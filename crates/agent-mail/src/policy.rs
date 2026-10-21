//! What is compiled in: where agent-mail signs in, where it reads mail, and whom each login may mail. The values
//! come from the build's environment, which package.nix sets from yakery's `lib/ironmoon.nix`.

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
            vec!["owner@example.org".parse().unwrap()],
            vec!["someone@example.com".parse().unwrap()],
            &[],
        )
        .err()
        .unwrap();
        assert_eq!(
            err.to_string(),
            "agent@example.org may mail only example.org and agent.example.org, not someone@example.com"
        );
    }

    #[test]
    fn a_correspondent_is_allowed_and_nobody_else_at_its_domain() {
        let login = "agent@example.org".parse().unwrap();
        let listed = correspondents(" pen.pal@example.com , other@example.net").unwrap();
        let to = |a: &str| vec![a.parse().unwrap()];
        assert!(
            Recipients::check_against(&login, to("Pen.Pal@Example.com"), vec![], &listed).is_ok()
        );
        let err = Recipients::check_against(&login, to("stranger@example.com"), vec![], &listed)
            .err()
            .unwrap();
        assert_eq!(
            err.to_string(),
            "agent@example.org may mail only example.org, agent.example.org and its correspondents, not stranger@example.com"
        );
    }

    #[test]
    fn a_malformed_correspondent_refuses_everything() {
        assert!(correspondents("ok@example.com,not an address").is_err());
        assert!(correspondents("").unwrap().is_empty());
    }
}

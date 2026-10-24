use text::{Address, text};

use crate::{
    AGENT_DOMAIN, CORRESPONDENTS, CORRESPONDENTS_FOR, DOMAIN, SHARED_LOCAL_PART, address::Topic,
    prelude::*,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Login(Address);

text! {
    Login(|s| {
        let address: Address = s.parse()?;
        let plain = !address.local().contains('+');
        ensure!(plain, "login {s} must be a plain address, not a subaddress");
        Ok(Login(address))
    }, |l| l.0.to_string());
}

impl Login {
    pub fn address(&self) -> &Address {
        &self.0
    }

    fn detail(&self, email: &str) -> Option<Option<Topic>> {
        let address: Address = email.parse().ok()?;
        let (base, detail) = match address.local().split_once('+') {
            Some((base, detail)) => (base, Some(detail.parse().ok()?)),
            None => (address.local(), None),
        };
        let ours = address.domain().eq_ignore_ascii_case(self.0.domain())
            && base.eq_ignore_ascii_case(self.0.local());
        ours.then_some(detail)
    }

    pub fn owns(&self, email: &str) -> bool {
        self.detail(email).is_some()
    }

    pub fn first_topic<'a>(&self, emails: impl IntoIterator<Item = &'a str>) -> Option<Address> {
        emails.into_iter().find(|e| matches!(self.detail(e), Some(Some(_))))?.parse().ok()
    }

    fn shared(&self) -> bool {
        self.0.local().eq_ignore_ascii_case(SHARED_LOCAL_PART)
            && self.0.domain().eq_ignore_ascii_case(DOMAIN)
    }

    pub fn bare_sender(&self) -> Result<Address> {
        ensure!(
            !self.shared(),
            "{self} is shared by every session, so it sends only from a topic subaddress"
        );
        Ok(self.0.clone())
    }
}

pub fn correspondents(from: &Address, list: &str, by_sender: &str) -> Result<Vec<Address>> {
    let parse =
        |a: &str| a.parse::<Address>().with_context(|| format!("compiled-in address \"{a}\""));
    let mut all: Vec<Address> = list
        .split(',')
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(parse)
        .collect::<Result<_>>()?;
    let by_sender: BTreeMap<String, Vec<String>> =
        serde_json::from_str(by_sender).context("compiled-in correspondents by sender")?;
    for (sender, list) in by_sender {
        let list = list.iter().map(|a| parse(a)).collect::<Result<Vec<_>>>()?;
        if parse(&sender)? == *from {
            all.extend(list);
        }
    }
    Ok(all)
}

pub struct Recipients(Envelope);

pub struct Envelope {
    pub from: Address,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
}

impl std::ops::Deref for Recipients {
    type Target = Envelope;
    fn deref(&self) -> &Envelope {
        &self.0
    }
}

impl Recipients {
    pub fn check(
        login: &Login,
        from: Address,
        to: Vec<Address>,
        cc: Vec<Address>,
    ) -> Result<Recipients> {
        let correspondents = correspondents(&from, CORRESPONDENTS, CORRESPONDENTS_FOR)?;
        let domains = [DOMAIN, AGENT_DOMAIN];
        let outside: Vec<String> = (to.iter().chain(&cc))
            .filter(|a| {
                login.shared()
                    && !domains.iter().any(|d| d.eq_ignore_ascii_case(a.domain()))
                    && !correspondents.contains(a)
            })
            .map(Address::to_string)
            .collect();
        if !outside.is_empty() {
            let allowed = match correspondents.is_empty() {
                true => domains.join(" and "),
                false => format!("{} and its correspondents", domains.join(", ")),
            };
            bail!("{from} may mail only {allowed}, not {}", outside.join(", "));
        }
        Ok(Recipients(Envelope { from, to, cc }))
    }

    pub fn everyone(&self) -> impl Iterator<Item = &Address> {
        self.to.iter().chain(&self.cc)
    }
}

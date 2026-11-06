//! The issuer sending the browser back (RFC 6749 4.1.2): with a code, or with why not.

use private::Secret;

use crate::Pkce;

/// What the redirect's query says; its code is had only for the sign-in whose state it carries.
pub struct Returned {
    state: Option<String>,
    code: Result<String, Denied>,
}

/// A code from a return carrying its sign-in's state, the only code a [`crate::Grant`] takes.
pub struct Code<'a> {
    pub(crate) code: String,
    pub(crate) pkce: &'a Pkce,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Denied {
    /// It came back with another state than the sign-in's, or none.
    Mismatched,
    /// An error (RFC 6749 4.1.2.1), as the issuer named it; its text is the issuer's.
    Error {
        error: String,
        description: Option<String>,
    },
    Codeless,
}

impl Returned {
    /// An error wins over a code, as the issuer sends one or the other.
    pub fn read(query: &str) -> Returned {
        let asked = |key: &str| {
            form_urlencoded::parse(query.as_bytes())
                .find_map(|(k, v)| (k == key).then(|| v.into_owned()))
        };
        let code = match (asked("error"), asked("code")) {
            (Some(error), _) => {
                Err(Denied::Error { error, description: asked("error_description") })
            }
            (None, code) => code.ok_or(Denied::Codeless),
        };
        Returned { state: asked("state"), code }
    }

    pub fn state(&self) -> Option<&str> {
        self.state.as_deref()
    }

    /// Whether it carries `pkce`'s state, compared in constant time.
    pub fn returns_from(&self, pkce: &Pkce) -> bool {
        self.state.as_deref().map(Secret::new).is_some_and(|state| state == *pkce.state())
    }

    /// The code, if it returns from `pkce`'s sign-in.
    pub fn code(self, pkce: &Pkce) -> Result<Code<'_>, Denied> {
        if !self.returns_from(pkce) {
            return Err(Denied::Mismatched);
        }
        self.code.map(|code| Code { code, pkce })
    }
}

#[cfg(test)]
#[path = "../tests/callback.rs"]
mod tests;

use crate::prelude::*;

pub fn nullable<'de, D: serde::Deserializer<'de>, T: Default + Deserialize<'de>>(
    d: D,
) -> Result<T, D::Error> {
    Option::<T>::deserialize(d).map(Option::unwrap_or_default)
}

/// A view is fetched with exactly the properties it declares, so it reads none it did not fetch, and decoding a
/// reply that lacks one fails; null, JMAP's empty, reads as the default.
pub trait View: DeserializeOwned {
    const GET: &str;
    const CAP: Cap = Cap::Mail;
    const PROPERTIES: &[&str];
    const VALUES: Option<(&str, usize)> = None;
}

/// What a method belongs to beyond JMAP's core: mail, with submission, or calendars. Each names the capabilities a
/// request declares, and the capability whose primary account the method acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cap {
    Mail,
    Calendars,
}

const MAIL: &str = "urn:ietf:params:jmap:mail";
const SUBMISSION: &str = "urn:ietf:params:jmap:submission";
const CALENDARS: &str = "urn:ietf:params:jmap:calendars";

impl Cap {
    pub const fn using(self) -> &'static [&'static str] {
        match self {
            Cap::Mail => &[MAIL, SUBMISSION],
            Cap::Calendars => &[CALENDARS],
        }
    }

    /// The capability the session names the method's account under, among its primary accounts.
    pub const fn account(self) -> &'static str {
        match self {
            Cap::Mail => MAIL,
            Cap::Calendars => CALENDARS,
        }
    }
}

macro_rules! wire {
    (views { $($view:ident in $get:literal $(under $cap:ident)? $(with ($bodies:literal, $bytes:expr))? {
        $($vf:ident: $vt:ty = $vp:literal),* $(,)?
    })* }
     objects { $($object:ident { $($of:ident: $ot:ty = $op:literal),* $(,)? })* }) => {
        $(
            wire!(@struct [] $view { $($vf: $vt = $vp),* });
            impl $crate::jmap::View for $view {
                const GET: &str = $get;
                $(const CAP: $crate::jmap::Cap = $crate::jmap::Cap::$cap;)?
                const PROPERTIES: &[&str] = &[$($vp),*];
                $(const VALUES: Option<(&str, usize)> = Some(($bodies, $bytes));)?
            }
        )*
        $(wire!(@struct [#[serde(default)]] $object { $($of: $ot = $op),* });)*
    };
    (@struct [$($meta:tt)*] $name:ident { $($field:ident: $ty:ty = $prop:literal),* }) => {
        #[derive(Clone, Debug, Default, Deserialize)]
        $($meta)*
        pub struct $name {
            $(
                #[serde(rename = $prop, deserialize_with = "crate::jmap::nullable")]
                pub $field: $ty,
            )*
        }
    };
}
pub(crate) use wire;

/// A method, the error type that answers it without failing (as None), and what it belongs to.
pub struct Call(pub &'static str, pub Option<&'static str>, pub Cap);

impl Call {
    pub const fn mail(name: &'static str, tolerated: Option<&'static str>) -> Call {
        Call(name, tolerated, Cap::Mail)
    }

    pub const fn calendars(name: &'static str, tolerated: Option<&'static str>) -> Call {
        Call(name, tolerated, Cap::Calendars)
    }
}

macro_rules! keywords {
    ($($keyword:ident = $name:literal),* $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Keyword { $($keyword),* }
        impl Keyword {
            pub const fn name(self) -> &'static str {
                match self { $(Keyword::$keyword => $name),* }
            }
        }
    };
}
pub(crate) use keywords;

pub fn jmap_id(id: &str) -> Result<&str> {
    let valid = (1..=255).contains(&id.len())
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    ensure!(valid, "{} is not a JMAP id", id.escape_debug());
    Ok(id)
}

use serde::{Deserialize, Deserializer, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::Cap;

/// Null, JMAP's empty, read as the default.
#[doc(hidden)]
pub fn nullable<'de, D: Deserializer<'de>, T: Default + Deserialize<'de>>(
    d: D,
) -> Result<T, D::Error> {
    Option::<T>::deserialize(d).map(Option::unwrap_or_default)
}

/// A view is fetched with exactly the properties it declares, so it reads none it did not fetch,
/// and decoding a reply that lacks one fails; null reads as the default. A `(default)` view reads a
/// missing one as the default too, as JSCalendar leaves a property at its default out (RFC 8984).
/// Declared by [`view!`].
pub trait View: DeserializeOwned {
    const GET: &str;
    const CAP: Cap;
    const PROPERTIES: &[&str];
    /// The `fetch*BodyValues` argument an Email view sets, with the most bytes of each value.
    const VALUES: Option<(&str, usize)> = None;

    /// The arguments of a /get of `ids` (null for every object) with this view's properties.
    fn args(ids: Value) -> Value {
        let mut args = json!({ "ids": ids, "properties": Self::PROPERTIES });
        if let Some((bodies, bytes)) = Self::VALUES {
            (args[bodies], args["maxBodyValueBytes"]) = (json!(true), json!(bytes));
        }
        args
    }
}

/// `view! { under MAIL { Mailbox in "Mailbox/get" { id: String = "id", role: String = "role" } } }`:
/// [`View`]s called under the [`Cap`] `MAIL`, whose fields are the properties they fetch.
/// `with ("fetchTextBodyValues", bytes)` after the method sets [`View::VALUES`], and `(default)`
/// before the fields makes a `(default)` view.
#[macro_export]
macro_rules! view {
    (under $cap:path { $($view:ident in $get:literal $(with ($bodies:literal, $bytes:expr))?
        $(($default:ident))? { $($field:ident: $ty:ty = $prop:literal),* $(,)? }
    )* }) => {$(
        $crate::object!(@struct $(#[serde($default)])? $view { $(pub $field: $ty = $prop),* });
        impl $crate::View for $view {
            const GET: &str = $get;
            const CAP: $crate::Cap = $cap;
            const PROPERTIES: &[&str] = &[$($prop),*];
            $(const VALUES: Option<(&str, usize)> = Some(($bodies, $bytes));)?
        }
    )*};
}

/// `object! { Addr { email: String = "email" } }`: an object inside an answer, whose properties
/// read as their default when absent or null.
#[macro_export]
macro_rules! object {
    ($($(#[$meta:meta])* $object:ident { $($field:ident: $ty:ty = $prop:literal),* $(,)? })*) => {
        $($crate::object!(@struct #[serde(default)] $(#[$meta])* $object {
            $(pub $field: $ty = $prop),*
        });)*
    };
    (@struct $(#[$meta:meta])* $name:ident {
        $($vis:vis $field:ident: $ty:ty = $prop:literal),* $(,)?
    }) => {
        #[derive(Clone, Debug, Default, ::serde::Deserialize)]
        $(#[$meta])*
        pub struct $name {
            $(
                #[serde(rename = $prop, deserialize_with = "::jmap::nullable")]
                $vis $field: $ty,
            )*
        }
    };
}

#[cfg(test)]
#[path = "../tests/view.rs"]
mod tests;

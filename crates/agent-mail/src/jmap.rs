use std::marker::PhantomData;

pub use ::jmap::{Cap, id as jmap_id};

pub const MAIL: Cap = Cap { using: &[::jmap::MAIL, ::jmap::SUBMISSION], account: ::jmap::MAIL };
pub const CALENDARS: Cap = Cap { using: &[::jmap::CALENDARS], account: ::jmap::CALENDARS };

/// A method, the answer it reads as, the method error it takes for no answer, and what it is called under.
pub struct Call<T>(pub &'static str, pub Option<&'static str>, pub Cap, PhantomData<fn() -> T>);

impl<T> Call<T> {
    pub const fn new(name: &'static str, tolerated: Option<&'static str>, cap: Cap) -> Call<T> {
        Call(name, tolerated, cap, PhantomData)
    }

    pub const fn mail(name: &'static str, tolerated: Option<&'static str>) -> Call<T> {
        Call::new(name, tolerated, MAIL)
    }

    pub const fn calendars(name: &'static str, tolerated: Option<&'static str>) -> Call<T> {
        Call::new(name, tolerated, CALENDARS)
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

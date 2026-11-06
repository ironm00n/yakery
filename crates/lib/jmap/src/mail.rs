//! The parts of JMAP for Mail (RFC 8621) its callers read.

use std::collections::HashMap;

crate::object! {
    /// How the SMTP server took a submission for one recipient (RFC 8621 7).
    Status { reply: String = "smtpReply", delivered: String = "delivered" }
}

/// What a submission's delivery status says of it, with each refused recipient and its reply.
#[derive(Debug, PartialEq, Eq)]
pub enum Delivery {
    Accepted(Vec<(String, String)>),
    Refused(Vec<(String, String)>),
    Unknown,
}

impl Delivery {
    /// Refused once every recipient is; `None`, a status not yet known, is [`Delivery::Unknown`].
    pub fn of(statuses: Option<&HashMap<String, Status>>) -> Delivery {
        let Some(statuses) = statuses else { return Delivery::Unknown };
        let mut refused: Vec<_> = statuses
            .iter()
            .filter(|(_, s)| s.delivered == "no")
            .map(|(a, s)| (a.clone(), s.reply.trim().to_owned()))
            .collect();
        refused.sort();
        if refused.len() == statuses.len() && !statuses.is_empty() {
            Delivery::Refused(refused)
        } else {
            Delivery::Accepted(refused)
        }
    }
}

#[cfg(test)]
#[path = "../tests/mail.rs"]
mod tests;

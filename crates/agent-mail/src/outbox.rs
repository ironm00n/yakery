//! Submitting an email as the login's identity, always with an explicit envelope: the recipients are exactly the
//! checked list, whatever the headers parse to.
//!
//! A created submission says only that Stalwart ran the SMTP transaction; whether each recipient was accepted is
//! in its `deliveryStatus`, which is read back in the same request.

use anyhow::{Context, Result, bail};
use jmap_client::{
    core::request::Request,
    email_submission::{self, Delivered, EmailSubmission},
    identity::Identity,
    mailbox::Role,
};

use crate::{
    address::Address,
    mail::{Mail, Responses},
};

pub struct Outbox {
    identity: String,
    /// Stalwart refuses any other envelope sender for this identity.
    identity_email: String,
    pub drafts: String,
    pub sent: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Refusal {
    pub address: String,
    pub reply: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Delivery {
    /// At least one recipient was accepted; `refused` lists the rest.
    Accepted { refused: Vec<Refusal> },
    /// Every recipient was refused: nothing was queued.
    Refused(Vec<Refusal>),
    /// The submission exists, but its status did not come back.
    Unknown,
}

#[derive(Debug)]
enum Submitted {
    Created {
        id: String,
        delivery: Delivery,
    },
    /// No submission was created, so nothing was sent.
    NotCreated(anyhow::Error),
}

#[derive(Debug)]
pub enum Outcome {
    Submitted {
        id: String,
        delivery: Delivery,
    },
    /// No submission was created, so nothing was sent.
    Unsent(anyhow::Error),
    /// The reply was lost and no submission could be found or ruled out.
    Unknown(anyhow::Error),
}

/// Whether a failed call is known to have changed nothing: RFC 8620 request-level errors mean no method ran.
/// Anything else, a lost or timed-out response included, leaves the outcome unknown.
fn not_processed(e: &anyhow::Error) -> bool {
    matches!(
        e.downcast_ref::<jmap_client::Error>(),
        Some(jmap_client::Error::Problem(_))
    )
}

fn delivery(submission: &EmailSubmission) -> Delivery {
    let Some(statuses) = submission.delivery_status() else {
        return Delivery::Unknown;
    };
    let mut refused: Vec<Refusal> = statuses
        .iter()
        .filter(|(_, s)| matches!(s.delivered(), Delivered::No))
        .map(|(address, s)| Refusal {
            address: address.clone(),
            reply: s.smtp_reply().to_owned(),
        })
        .collect();
    refused.sort_by(|a, b| a.address.cmp(&b.address));
    match refused.len() == statuses.len() && !statuses.is_empty() {
        true => Delivery::Refused(refused),
        false => Delivery::Accepted { refused },
    }
}

impl Outbox {
    pub async fn open(mail: &Mail) -> Result<Outbox> {
        let mut responses = mail
            .call(|r| {
                r.get_identity();
            })
            .await?;
        let identities: Vec<Identity> = responses.next()?.unwrap_get_identity()?.take_list();
        let login = mail.auth.login.to_string();
        let identity = identities
            .iter()
            .find(|i| i.email().is_some_and(|e| e.eq_ignore_ascii_case(&login)))
            .with_context(|| format!("no identity for {login}"))?;
        let session = mail.session();
        Ok(Outbox {
            identity: identity.id().unwrap_or_default().to_owned(),
            identity_email: identity.email().unwrap_or_default().to_owned(),
            drafts: session.mailbox(Role::Drafts)?.to_owned(),
            sent: session.mailbox(Role::Sent)?.to_owned(),
        })
    }

    fn submit(&self, r: &mut Request<'_>, email_id: &str, rcpt_to: &[Address]) {
        r.set_email_submission()
            .create_with_id("send")
            .email_id(email_id)
            .identity_id(&self.identity)
            .envelope(
                self.identity_email.as_str(),
                rcpt_to.iter().map(|a| a.to_string()),
            );
        r.get_email_submission().ids(["#send"]).properties([
            email_submission::Property::DeliveryStatus,
            email_submission::Property::UndoStatus,
        ]);
    }

    /// Reads what `submit` added. An `Err` means the responses were malformed, so the outcome is unknown.
    fn read(mut responses: Responses) -> Result<Submitted> {
        let created = match responses
            .next()?
            .unwrap_set_email_submission()?
            .created("send")
        {
            Ok(created) => created,
            Err(e) => return Ok(Submitted::NotCreated(e.into())),
        };
        let id = created.id().unwrap_or_default().to_owned();
        let delivery = match responses
            .next()
            .and_then(|r| Ok(r.unwrap_get_email_submission()?))
        {
            Ok(mut got) => got.take_list().first().map_or(Delivery::Unknown, delivery),
            Err(_) => Delivery::Unknown,
        };
        Ok(Submitted::Created { id, delivery })
    }

    /// Submits `email_id` to exactly `rcpt_to`; when the reply is lost, the submission is looked for.
    pub async fn send(&self, mail: &Mail, email_id: &str, rcpt_to: &[Address]) -> Outcome {
        let submitted = mail
            .call(|r| self.submit(r, email_id, rcpt_to))
            .await
            .and_then(Outbox::read);
        match submitted {
            Ok(Submitted::Created { id, delivery }) => Outcome::Submitted { id, delivery },
            Ok(Submitted::NotCreated(e)) => Outcome::Unsent(e),
            Err(e) if not_processed(&e) => Outcome::Unsent(e),
            Err(e) => match self.find(mail, email_id).await {
                Ok(Some((id, delivery))) => Outcome::Submitted { id, delivery },
                Ok(None) => Outcome::Unknown(e),
                Err(f) => {
                    Outcome::Unknown(e.context(format!("looking for its submission failed: {f:#}")))
                }
            },
        }
    }

    async fn find(&self, mail: &Mail, email_id: &str) -> Result<Option<(String, Delivery)>> {
        let mut responses = mail
            .call(|r| {
                let ids = r
                    .query_email_submission()
                    .filter(email_submission::query::Filter::email_ids([email_id]))
                    .result_reference();
                r.get_email_submission().ids_ref(ids).properties([
                    email_submission::Property::DeliveryStatus,
                    email_submission::Property::UndoStatus,
                ]);
            })
            .await?;
        responses.next()?.unwrap_query_email_submission()?;
        let mut got = responses.next()?.unwrap_get_email_submission()?;
        Ok(got
            .take_list()
            .first()
            .map(|s| (s.id().unwrap_or_default().to_owned(), delivery(s))))
    }

    /// Moves a sent draft into Sent.
    pub async fn file(&self, mail: &Mail, email_id: &str) -> Result<()> {
        let mut responses = mail
            .call(|r| {
                r.set_email()
                    .update(email_id)
                    .mailbox_id(&self.drafts, false)
                    .mailbox_id(&self.sent, true)
                    .keyword("$draft", false);
            })
            .await?;
        responses
            .next()?
            .unwrap_set_email()?
            .unwrap_update_errors()?;
        Ok(())
    }
}

pub fn describe(refused: &[Refusal]) -> String {
    let lines: Vec<String> = refused
        .iter()
        .map(|r| format!("  {}: {}", r.address, r.reply.trim()))
        .collect();
    lines.join("\n")
}

pub fn require_all(id: &str, delivery: &Delivery) -> Result<()> {
    if let Delivery::Refused(refused) = delivery {
        bail!(
            "submission {id}: every recipient was refused, nothing was sent\n{}",
            describe(refused)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use jmap_client::core::response::{Response, TaggedMethodResponse};

    use super::*;

    fn read(json: &str) -> Submitted {
        let r: Response<TaggedMethodResponse> = serde_json::from_str(json).unwrap();
        Outbox::read(Responses::from_vec(r.unwrap_method_responses())).unwrap()
    }

    fn created(statuses: &str) -> String {
        format!(
            r#"{{"sessionState":"s","methodResponses":[
              ["EmailSubmission/set", {{"accountId":"a","newState":"2","created":{{"send":{{"id":"S1","undoStatus":"final"}}}}}}, "s1"],
              ["EmailSubmission/get", {{"accountId":"a","state":"2","notFound":[],"list":[{{"id":"S1","undoStatus":"final","deliveryStatus":{statuses}}}]}}, "s2"]
            ]}}"#
        )
    }

    #[test]
    fn a_submission_every_recipient_refused_is_refused() {
        let got = read(&created(
            r#"{"nosuch@example.org":{"smtpReply":"550 5.1.2 Mailbox does not exist.","delivered":"no","displayed":"unknown"}}"#,
        ));
        let Submitted::Created { id, delivery } = got else {
            panic!("{got:?}")
        };
        assert_eq!(id, "S1");
        assert_eq!(
            delivery,
            Delivery::Refused(vec![Refusal {
                address: "nosuch@example.org".into(),
                reply: "550 5.1.2 Mailbox does not exist.".into()
            }])
        );
        assert!(require_all("S1", &delivery).is_err());
    }

    #[test]
    fn a_partly_refused_submission_lists_the_refused() {
        let got = read(&created(
            r#"{"a@example.org":{"smtpReply":"250 2.1.5 Queued","delivered":"unknown","displayed":"unknown"},
                "b@example.org":{"smtpReply":"550 no","delivered":"no","displayed":"unknown"}}"#,
        ));
        let Submitted::Created {
            delivery: Delivery::Accepted { refused },
            ..
        } = got
        else {
            panic!("{got:?}")
        };
        assert_eq!(
            refused
                .iter()
                .map(|r| r.address.as_str())
                .collect::<Vec<_>>(),
            ["b@example.org"]
        );
    }

    #[test]
    fn a_created_submission_without_its_status_is_unknown_not_failed() {
        let got = read(
            r#"{"sessionState":"s","methodResponses":[
              ["EmailSubmission/set", {"accountId":"a","newState":"2","created":{"send":{"id":"S1"}}}, "s1"]
            ]}"#,
        );
        assert!(
            matches!(
                got,
                Submitted::Created {
                    delivery: Delivery::Unknown,
                    ..
                }
            ),
            "{got:?}"
        );
    }

    #[test]
    fn a_submission_not_created_says_so() {
        let got = read(
            r##"{"sessionState":"s","methodResponses":[
              ["EmailSubmission/set", {"accountId":"a","newState":"2","notCreated":{"send":{"type":"forbiddenFrom"}}}, "s1"],
              ["EmailSubmission/get", {"accountId":"a","state":"2","notFound":["#send"],"list":[]}, "s2"]
            ]}"##,
        );
        assert!(matches!(got, Submitted::NotCreated(_)), "{got:?}");
    }
}

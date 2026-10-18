use anyhow::Result;
use jmap_client::{
    core::query::Filter,
    email::{self, query},
    mailbox::Role,
};

use crate::{
    address::Topic,
    auth::Auth,
    config::Config,
    mail::Mail,
    show::{inbox_line, out},
};

#[derive(clap::Args)]
pub struct Args {
    /// Keep only mail addressed to the subaddress <login>+<topic>.
    #[arg(long)]
    topic: Vec<Topic>,
    #[arg(long, default_value_t = 10)]
    limit: usize,
}

pub async fn run(args: Args) -> Result<()> {
    let mail = Mail::connect(Auth::new(Config::load()?)?).await?;
    let inbox = mail.session().mailbox(Role::Inbox)?.to_owned();
    let addressed: Vec<query::Filter> = args
        .topic
        .iter()
        .map(|t| mail.auth.login.topic_address(t))
        .flat_map(|a| [query::Filter::to(a.clone()), query::Filter::cc(a)])
        .collect();
    let mut responses = mail
        .call(|r| {
            let in_inbox = query::Filter::in_mailbox(&inbox);
            let filter = match addressed.is_empty() {
                true => Filter::from(in_inbox),
                false => Filter::and([Filter::from(in_inbox), Filter::or(addressed.clone())]),
            };
            let ids = r
                .query_email()
                .filter(filter)
                .sort([query::Comparator::received_at().descending()])
                .limit(args.limit)
                .result_reference();
            use email::Property as P;
            r.get_email().ids_ref(ids).properties([
                P::ReceivedAt,
                P::From,
                P::To,
                P::Cc,
                P::Subject,
                P::Keywords,
                P::Attachments,
                P::Preview,
            ]);
        })
        .await?;
    responses.next()?.unwrap_query_email()?;
    for m in responses.emails()? {
        out(&inbox_line(&m))?;
    }
    Ok(())
}

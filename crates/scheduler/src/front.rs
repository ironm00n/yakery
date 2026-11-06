//! The front: every byte from the internet is parsed here, by a process with no credentials, no
//! state and no IP socket of its own, which asks the core for what each page shows.

use std::convert::Infallible;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, HeaderName, REFERRER_POLICY,
    X_CONTENT_TYPE_OPTIONS,
};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use jiff::Timestamp;
use maud::Markup;
use text::Address;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;

use crate::Sockets;
use crate::ask::{
    self, Answer, BookAsk, Booked, FrontAsk, LinkAsk, Question, Refusal, Week, WeekAsk,
};
use crate::checked::Checked;
use crate::link::{Form, MOST_NAME, MOST_NOTE, Token};
use crate::pages::{self, Problem, Viewer};
use crate::params::{Posted, Query};
use crate::policy::Policy;

const ANSWERING: usize = 8;
/// The longest form the link page's fields allow: each character as up to 4 UTF-8 bytes, each
/// percent-encoded as 3, and room for the slot, the zone and the field names.
const MOST_FORM: usize = 12 * (MOST_NAME + MOST_NOTE) + 3 * Address::MOST + 256;
const WITHIN: Duration = Duration::from_secs(10);

const HEADERS: [(HeaderName, &str); 5] = [
    (CONTENT_TYPE, "text/html; charset=utf-8"),
    (
        CONTENT_SECURITY_POLICY,
        "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'",
    ),
    (X_CONTENT_TYPE_OPTIONS, "nosniff"),
    (REFERRER_POLICY, "no-referrer"),
    (CACHE_CONTROL, "no-store"),
];

fn reply(status: StatusCode, markup: Markup, noindex: bool) -> Response<Full<Bytes>> {
    let mut reply = Response::builder().status(status);
    for (name, value) in HEADERS {
        reply = reply.header(name, value);
    }
    if noindex {
        reply = reply.header("x-robots-tag", "noindex");
    }
    reply.body(Full::new(Bytes::from(markup.into_string()))).expect("static headers")
}

struct Front<'a> {
    core: &'a Path,
}

impl Front<'_> {
    /// Past [`ask::ANSWERED_WITHIN`] the core has claimed nothing; a booking that got no answer
    /// otherwise may have been claimed, and its page will say.
    async fn ask<Q: Question>(&self, question: Q) -> Answer<Q::Answer> {
        let question: FrontAsk = question.into();
        match tokio::time::timeout(ask::ANSWERED_WITHIN, ask::ask(self.core, &question)).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(e)) => {
                yakery_log::warn!("asking the core: {e:#}");
                Err(Q::LOST)
            }
            Err(_) => {
                yakery_log::warn!("the core gave no answer within {:?}", ask::ANSWERED_WITHIN);
                Err(Refusal::Unavailable)
            }
        }
    }

    async fn answer(&self, request: Request<Incoming>) -> Response<Full<Bytes>> {
        let query = Query::parse(request.uri().query().unwrap_or_default().as_bytes());
        let path = request.uri().path().to_string();
        let token = path.strip_prefix("/b/").map(Token::parse);
        match (request.method().clone(), token) {
            (Method::GET, None) if path == "/" => self.week(&query).await,
            (Method::GET, Some(Ok(token))) => self.link(token, &query, None).await,
            (Method::POST, Some(Ok(token))) => match tokio::time::timeout(
                WITHIN,
                Limited::new(request.into_body(), MOST_FORM).collect(),
            )
            .await
            {
                Ok(Ok(body)) => self.book(token, Posted::parse(&body.to_bytes())).await,
                _ => self.refused(&Viewer::new(None), Refusal::Unavailable),
            },
            _ => reply(
                StatusCode::NOT_FOUND,
                pages::message(&Viewer::new(None), "Not found", "No page here."),
                true,
            ),
        }
    }

    async fn week(&self, query: &Query) -> Response<Full<Bytes>> {
        let (viewer, n) = (Viewer::new(query.tz.as_deref()), query.week);
        match self.ask(WeekAsk(n)).await {
            Ok(Week { monday, blocks, open, later }) => {
                reply(StatusCode::OK, pages::week(&viewer, n, monday, &blocks, &open, later), false)
            }
            Err(r) => self.refused(&viewer, r),
        }
    }

    async fn link(
        &self,
        token: Token,
        query: &Query,
        retry: Option<(Problem, Form)>,
    ) -> Response<Full<Bytes>> {
        let (viewer, chosen) = (Viewer::new(query.tz.as_deref()), query.slot);
        let status = retry.as_ref().map_or(StatusCode::OK, |(problem, _)| match problem {
            Problem::Field(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Problem::Taken => StatusCode::CONFLICT,
        });
        let (problem, form) = retry.unzip();
        match self.ask(LinkAsk(token)).await {
            Ok(view) => {
                let form = form.unwrap_or_default();
                reply(status, pages::link(&viewer, &view, chosen, &form, problem), true)
            }
            Err(r) => self.refused(&viewer, r),
        }
    }

    async fn book(&self, token: Token, posted: Posted) -> Response<Full<Bytes>> {
        let Posted { tz, slot, form } = posted;
        let viewer = Viewer::new(tz.as_deref());
        let Some(start) = slot else { return self.refused(&viewer, Refusal::SlotGone) };
        let booker = match Checked::new(form.clone()) {
            Ok(booker) => booker,
            Err(field) => {
                let again = Query { tz, week: 0, slot: Some(start) };
                return self.link(token, &again, Some((Problem::Field(field), form))).await;
            }
        };
        let by = Timestamp::now() + ask::CLAIMED_WITHIN;
        match self.ask(BookAsk { token: token.clone(), start, booker, by }).await {
            Ok(Booked { start, duration, booker, owner }) => {
                let page = pages::booked(&viewer, start, duration, &booker, &owner);
                reply(StatusCode::OK, page, true)
            }
            Err(Refusal::SlotGone) => {
                let again = Query { tz, ..Query::default() };
                self.link(token, &again, Some((Problem::Taken, form))).await
            }
            Err(r) => self.refused(&viewer, r),
        }
    }

    fn refused(&self, viewer: &Viewer, refusal: Refusal) -> Response<Full<Bytes>> {
        let (status, title, text) = match refusal {
            Refusal::NoSuchLink => {
                (StatusCode::NOT_FOUND, "No such link", "This link doesn't exist, or has expired.")
            }
            Refusal::NoSuchWeek => (
                StatusCode::NOT_FOUND,
                "Too far ahead",
                "That week is further out than this page shows.",
            ),
            Refusal::Pending => (
                StatusCode::ACCEPTED,
                "Being booked",
                "This link is being booked. Check back in a few minutes.",
            ),
            Refusal::Used => (StatusCode::CONFLICT, "Already booked", "This link has been used."),
            Refusal::SlotGone => {
                (StatusCode::CONFLICT, "Time taken", "That time is no longer free.")
            }
            Refusal::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "Try again later",
                "The calendar can't be reached right now.",
            ),
            Refusal::Unanswered => (
                StatusCode::BAD_GATEWAY,
                "No answer",
                "The calendar gave no answer. Open this link again in a few minutes to see whether \
                 this time was booked.",
            ),
        };
        reply(status, pages::message(viewer, title, text), true)
    }
}

pub async fn run(policy: &'static Policy, mut sockets: Sockets) -> Result<()> {
    let core = policy.layout.front.path.as_path();
    let listener = std::net::TcpListener::from(sockets.take(&policy.layout.http.fd)?);
    listener.set_nonblocking(true)?;
    let listener = TcpListener::from_std(listener)?;
    let answering = Arc::new(Semaphore::new(ANSWERING));
    loop {
        let (stream, _) = yakery_serve::accept(async || listener.accept().await).await;
        let permit = answering.clone().acquire_owned().await?;
        tokio::task::spawn_local(async move {
            let front = Front { core };
            let service =
                service_fn(|request| async { Ok::<_, Infallible>(front.answer(request).await) });
            let connection = http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(WITHIN)
                .serve_connection(TokioIo::new(stream), service);
            let _ = connection.await;
            drop(permit);
        });
    }
}

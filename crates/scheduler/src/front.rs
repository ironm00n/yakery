//! The front: every byte from the internet is parsed here, by a process with no credentials, no
//! state and no IP socket of its own, which asks the core for what each page shows.

use std::collections::BTreeMap;
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
use crate::ask::{self, FrontAnswer, FrontAsk, Refusal};
use crate::checked::Checked;
use crate::link::{Form, MOST_NAME, MOST_NOTE, Token};
use crate::pages::{self, Viewer};

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

fn pairs(bytes: &[u8]) -> BTreeMap<String, String> {
    form_urlencoded::parse(bytes).into_owned().collect()
}

struct Front<'a> {
    core: &'a Path,
}

impl Front<'_> {
    /// Past [`ask::ANSWERED_WITHIN`] the core has claimed nothing; a booking that got no answer
    /// otherwise may have been claimed, and its page will say.
    async fn ask(&self, question: FrontAsk) -> FrontAnswer {
        let books = matches!(question, FrontAsk::Book { .. });
        match tokio::time::timeout(ask::ANSWERED_WITHIN, ask::ask(self.core, &question)).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(e)) => {
                yakery_log::warn!("asking the core: {e:#}");
                FrontAnswer::Refused(if books { Refusal::Unanswered } else { Refusal::Unavailable })
            }
            Err(_) => {
                yakery_log::warn!("the core gave no answer within {:?}", ask::ANSWERED_WITHIN);
                FrontAnswer::Refused(Refusal::Unavailable)
            }
        }
    }

    async fn answer(&self, request: Request<Incoming>) -> Response<Full<Bytes>> {
        let query = pairs(request.uri().query().unwrap_or_default().as_bytes());
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
                Ok(Ok(body)) => self.book(token, pairs(&body.to_bytes())).await,
                _ => self.refused(&Viewer::new(None), Refusal::Unavailable),
            },
            _ => reply(
                StatusCode::NOT_FOUND,
                pages::message(&Viewer::new(None), "Not found", "No page here."),
                true,
            ),
        }
    }

    async fn week(&self, query: &BTreeMap<String, String>) -> Response<Full<Bytes>> {
        let viewer = Viewer::new(query.get("tz").map(String::as_str));
        let n = query.get("week").and_then(|w| w.parse().ok()).unwrap_or(0);
        match self.ask(FrontAsk::Week(n)).await {
            FrontAnswer::Week { monday, blocks, open } => {
                reply(StatusCode::OK, pages::week(&viewer, n, monday, &blocks, &open), false)
            }
            FrontAnswer::Refused(r) => self.refused(&viewer, r),
            _ => self.refused(&viewer, Refusal::Unavailable),
        }
    }

    /// A link's page; `retry` is a form sent back with what was wrong with it.
    async fn link(
        &self,
        token: Token,
        query: &BTreeMap<String, String>,
        retry: Option<(StatusCode, Form, String)>,
    ) -> Response<Full<Bytes>> {
        let viewer = Viewer::new(query.get("tz").map(String::as_str));
        let chosen = query.get("slot").and_then(|s| s.parse().ok());
        let chosen = chosen.and_then(|s| Timestamp::from_second(s).ok());
        let (status, form, problem) =
            retry.unwrap_or((StatusCode::OK, Form::default(), String::new()));
        match self.ask(FrontAsk::Link(token)).await {
            FrontAnswer::Link(view) => {
                reply(status, pages::link(&viewer, &view, chosen, &form, &problem), true)
            }
            FrontAnswer::Refused(r) => self.refused(&viewer, r),
            _ => self.refused(&viewer, Refusal::Unavailable),
        }
    }

    async fn book(&self, token: Token, fields: BTreeMap<String, String>) -> Response<Full<Bytes>> {
        let field = |name: &str| fields.get(name).cloned().unwrap_or_default();
        let form = Form { name: field("name"), address: field("address"), note: field("note") };
        let viewer = Viewer::new(fields.get("tz").map(String::as_str));
        let Some(start) = field("slot").parse().ok().and_then(|s| Timestamp::from_second(s).ok())
        else {
            return self.refused(&viewer, Refusal::SlotGone);
        };
        let booker = match Checked::new(form.clone()) {
            Ok(booker) => booker,
            Err(problem) => {
                let retry = (StatusCode::UNPROCESSABLE_ENTITY, form, problem.to_string());
                return self.link(token, &fields, Some(retry)).await;
            }
        };
        let by = Timestamp::now() + ask::CLAIMED_WITHIN;
        match self.ask(FrontAsk::Book { token: token.clone(), start, booker, by }).await {
            FrontAnswer::Booked { start, duration, booker } => {
                reply(StatusCode::OK, pages::booked(&viewer, start, duration, &booker), true)
            }
            FrontAnswer::Refused(Refusal::SlotGone) => {
                let zone =
                    fields.iter().filter(|(k, _)| *k == "tz").map(|(k, v)| (k.clone(), v.clone()));
                let taken = "That time was just taken. Choose another.".to_string();
                self.link(token, &zone.collect(), Some((StatusCode::CONFLICT, form, taken))).await
            }
            FrontAnswer::Refused(r) => self.refused(&viewer, r),
            _ => self.refused(&viewer, Refusal::Unavailable),
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

pub async fn run(core: &'static Path, mut sockets: Sockets) -> Result<()> {
    let listener = std::net::TcpListener::from(sockets.take("http")?);
    listener.set_nonblocking(true)?;
    let listener = TcpListener::from_std(listener)?;
    let answering = Arc::new(Semaphore::new(ANSWERING));
    loop {
        let (stream, _) = listener.accept().await?;
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

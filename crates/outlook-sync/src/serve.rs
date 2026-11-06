use std::convert::Infallible;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use jiff::Timestamp;
use oidc::{Denied, Returned};
use serde::Deserialize;
use tokio::net::TcpListener;

use crate::policy::{Feed, Route};
use crate::state::{self, Login, Untaken};
use crate::sync::{Sync, Wake};

/// Graph's notifications carry ids and metadata only; this is well past a batch of them.
const MOST_BODY: usize = 1024 * 1024;
const WITHIN: Duration = Duration::from_secs(5);

fn reply(status: StatusCode, kind: &str, body: impl Into<Bytes>) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("content-type", kind)
        .header("cache-control", "no-store")
        .body(Full::new(body.into()))
        .expect("static headers")
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Notice {
    subscription_id: String,
    #[serde(default)]
    client_state: String,
    #[serde(default)]
    lifecycle_event: Option<String>,
}

#[derive(Deserialize)]
struct Notices {
    value: Vec<Notice>,
}

impl Sync {
    async fn answer(&self, request: Request<Incoming>) -> Response<Full<Bytes>> {
        let route =
            request.uri().path().strip_prefix(self.policy.public.prefix()).and_then(Route::of);
        let query: Vec<(String, String)> =
            form_urlencoded::parse(request.uri().query().unwrap_or_default().as_bytes())
                .into_owned()
                .collect();
        let field = |name: &str| query.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
        match (request.method().clone(), route) {
            (Method::POST, Some(Route::Notices(feed))) => match field("validationToken") {
                Some(token) if self.shared.validating.get() => {
                    reply(StatusCode::OK, "text/plain", token)
                }
                Some(_) => {
                    reply(StatusCode::FORBIDDEN, "text/plain", "no subscription is being made")
                }
                None => {
                    let body = Limited::new(request.into_body(), MOST_BODY).collect();
                    match tokio::time::timeout(WITHIN, body).await {
                        Ok(Ok(body)) => match serde_json::from_slice(&body.to_bytes()) {
                            Ok(notices) => self.noticed(notices, feed),
                            Err(e) => yakery_log::warn!("an unreadable notice: {e}"),
                        },
                        Ok(Err(e)) => yakery_log::warn!("a notice's body: {e}"),
                        Err(_) => yakery_log::warn!("a notice's body took past {WITHIN:?}"),
                    }
                    reply(StatusCode::ACCEPTED, "text/plain", "")
                }
            },
            (Method::GET, Some(Route::Callback)) => {
                self.callback(Returned::read(request.uri().query().unwrap_or_default())).await
            }
            _ => reply(StatusCode::NOT_FOUND, "text/plain", "not found"),
        }
    }

    fn noticed(&self, notices: Notices, feed: Feed) {
        for notice in
            notices.value.iter().filter(|n| self.shared.ours(&n.subscription_id, &n.client_state))
        {
            let wake = match (feed, notice.lifecycle_event.as_deref()) {
                (Feed::Changes, _) => Some(Wake::Changed),
                (Feed::Lifecycle, Some("reauthorizationRequired")) => Some(Wake::Reauthorize),
                (Feed::Lifecycle, Some("subscriptionRemoved")) => Some(Wake::Removed),
                (Feed::Lifecycle, Some("missed")) => Some(Wake::Missed),
                (Feed::Lifecycle, other) => {
                    yakery_log::warn!("a lifecycle event this sync doesn't know: {other:?}");
                    None
                }
            };
            if let Some(wake) = wake {
                let _ = self.shared.wake.send(wake);
            }
        }
    }

    async fn callback(&self, returned: Returned) -> Response<Full<Bytes>> {
        let page = |status, text: &str| {
            let html = format!(
                "<!doctype html><meta charset=utf-8><title>outlook-sync</title><p>{text}</p>"
            );
            reply(status, "text/html; charset=utf-8", html)
        };
        let pkce = match state::take(&self.state, &returned, Timestamp::now()) {
            Ok(Ok(pkce)) => pkce,
            Ok(Err(untaken)) => {
                let (status, text) = match untaken {
                    Untaken::Nothing => (
                        StatusCode::NOT_FOUND,
                        "No sign-in is waiting. Run outlook-sync login first.",
                    ),
                    Untaken::Stale => (
                        StatusCode::FORBIDDEN,
                        "This sign-in link is not the one waiting, or it has expired.",
                    ),
                    Untaken::Taken => {
                        (StatusCode::CONFLICT, "This sign-in is being finished already.")
                    }
                };
                return page(status, text);
            }
            Err(e) => {
                yakery_log::warn!("recording the sign-in: {e:#}");
                return page(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "The sign-in couldn't be recorded.",
                );
            }
        };
        let outcome = match returned.code(&pkce) {
            Ok(code) => self.graph.redeem(&code).await.map_err(|e| format!("{e:#}")),
            Err(denied) => {
                let error = match denied {
                    Denied::Error { error, .. } => error,
                    Denied::Mismatched | Denied::Codeless => String::new(),
                };
                Err(format!("Microsoft answered without a code: {error}"))
            }
        };
        let done = Login::Done(outcome.clone());
        if let Err(e) = self.state.save(state::LOGIN, &Some(done)) {
            yakery_log::warn!("recording the sign-in: {e:#}");
        }
        match outcome {
            Ok(()) => {
                let _ = self.shared.wake.send(Wake::SignedIn);
                page(
                    StatusCode::OK,
                    "Signed in. The calendar sync starts now, and this page can be closed.",
                )
            }
            Err(e) => {
                yakery_log::warn!("sign-in: {e}");
                page(StatusCode::BAD_GATEWAY, "Signing in failed. The server's log says why.")
            }
        }
    }
}

pub async fn serve(sync: &'static Sync, listener: TcpListener) -> anyhow::Result<()> {
    loop {
        let (stream, _) = yakery_serve::accept("the http socket", || listener.accept()).await?;
        tokio::task::spawn_local(async move {
            let service = service_fn(move |request| async move {
                Ok::<_, Infallible>(sync.answer(request).await)
            });
            let _ = http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(WITHIN)
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
    }
}

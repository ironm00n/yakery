//! The public paths nginx forwards: Graph's notifications and lifecycle events, each answered at
//! once and handed to the loop only if it names our subscription and its secret, and the sign-in
//! callback, which redeems a code only for the sign-in `outlook-sync login` started.

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
use serde::Deserialize;
use tokio::net::TcpListener;

use crate::state::{self, Login};
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
        let prefix = self.policy.public.split_once("://").map_or("", |(_, r)| r);
        let prefix = prefix.find('/').map_or("", |at| &prefix[at..]);
        let path = request.uri().path().strip_prefix(prefix).unwrap_or_default().to_string();
        let query: Vec<(String, String)> =
            form_urlencoded::parse(request.uri().query().unwrap_or_default().as_bytes())
                .into_owned()
                .collect();
        let field = |name: &str| query.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
        match (request.method().clone(), path.as_str()) {
            (Method::POST, "/notify" | "/lifecycle") => match field("validationToken") {
                Some(token) if self.shared.validating.get() => {
                    reply(StatusCode::OK, "text/plain", token)
                }
                Some(_) => {
                    reply(StatusCode::FORBIDDEN, "text/plain", "no subscription is being made")
                }
                None => {
                    let lifecycle = path == "/lifecycle";
                    if let Ok(Ok(body)) = tokio::time::timeout(
                        WITHIN,
                        Limited::new(request.into_body(), MOST_BODY).collect(),
                    )
                    .await
                        && let Ok(notices) = serde_json::from_slice::<Notices>(&body.to_bytes())
                    {
                        self.noticed(notices, lifecycle);
                    }
                    reply(StatusCode::ACCEPTED, "text/plain", "")
                }
            },
            (Method::GET, "/callback") => {
                self.callback(field("state"), field("code"), field("error")).await
            }
            _ => reply(StatusCode::NOT_FOUND, "text/plain", "not found"),
        }
    }

    fn noticed(&self, notices: Notices, lifecycle: bool) {
        for notice in
            notices.value.iter().filter(|n| self.shared.ours(&n.subscription_id, &n.client_state))
        {
            let wake = match (lifecycle, notice.lifecycle_event.as_deref()) {
                (false, _) => Some(Wake::Changed),
                (true, Some("reauthorizationRequired")) => Some(Wake::Reauthorize),
                (true, Some("subscriptionRemoved")) => Some(Wake::Removed),
                (true, Some("missed")) => Some(Wake::Missed),
                (true, other) => {
                    yakery_log::warn!("a lifecycle event this sync doesn't know: {other:?}");
                    None
                }
            };
            if let Some(wake) = wake {
                let _ = self.shared.wake.send(wake);
            }
        }
    }

    async fn callback(
        &self,
        given: Option<String>,
        code: Option<String>,
        error: Option<String>,
    ) -> Response<Full<Bytes>> {
        let page = |status, text: &str| {
            let html = format!(
                "<!doctype html><meta charset=utf-8><title>outlook-sync</title><p>{text}</p>"
            );
            reply(status, "text/html; charset=utf-8", html)
        };
        let Ok(Some(Login::Pending { state, verifier, until })) =
            self.state.load::<Option<Login>>(state::LOGIN)
        else {
            return page(
                StatusCode::NOT_FOUND,
                "No sign-in is waiting. Run outlook-sync login first.",
            );
        };
        if given.as_deref().map(private::Secret::new) != Some(state) || Timestamp::now() > until {
            return page(
                StatusCode::FORBIDDEN,
                "This sign-in link is not the one waiting, or it has expired.",
            );
        }
        let outcome = match (code, error) {
            (Some(code), _) => {
                self.graph.redeem(&code, &verifier).await.map_err(|e| format!("{e:#}"))
            }
            (None, error) => {
                Err(format!("Microsoft answered without a code: {}", error.unwrap_or_default()))
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

/// Serves each connection on its own local task.
pub async fn serve(sync: &'static Sync, listener: TcpListener) {
    loop {
        let Ok((stream, _)) = listener.accept().await else { continue };
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

use std::{
    convert::Infallible,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context as Waker, Poll},
};

use bytes::Bytes;
use http_body::Frame;
use private::Secret;
use reqwest::{Client, Method, RequestBuilder, Response, StatusCode, Url, header, redirect};
use tokio::time::Instant;

use crate::{AUTH_HOST, JMAP_HOST, prelude::*};

pub const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Auth,
    Mail,
}

#[derive(Clone, Debug)]
pub struct Pinned(Origin, Url);

impl Origin {
    fn host(self) -> &'static str {
        if self == Origin::Auth { AUTH_HOST } else { JMAP_HOST }
    }

    pub fn url(self, path: &str) -> Pinned {
        self.check(&format!("https://{}{path}", self.host()))
            .expect("compiled-in paths stay on their origin")
    }

    pub fn check(self, url: &str) -> Result<Pinned> {
        let host = self.host();
        let parsed =
            Url::parse(url).with_context(|| format!("{} is not a URL", url.escape_debug()))?;
        let on = (parsed.scheme(), parsed.host_str(), parsed.port(), parsed.username())
            == ("https", Some(host), None, "")
            && parsed.password().is_none();
        ensure!(on, "refusing {}: it is not on https://{host}", url.escape_debug());
        Ok(Pinned(self, parsed))
    }
}

impl Pinned {
    pub fn join(&self, link: &str) -> Result<Pinned> {
        let joined =
            self.1.join(link).with_context(|| format!("{} is not a link", link.escape_debug()))?;
        self.0.check(joined.as_str())
    }

    pub fn path(&self) -> &str {
        self.1.path()
    }

    pub fn as_str(&self) -> &str {
        self.1.as_str()
    }
}

pub enum Body<'a> {
    Empty,
    Form(&'a [(String, String)]),
    Json(&'a Value),
}

pub struct Page {
    pub url: Pinned,
    pub status: StatusCode,
    pub problem: bool,
    pub text: String,
}

impl Page {
    pub async fn read(url: Pinned, response: Response) -> Result<Page> {
        let kind = response.headers().get(header::CONTENT_TYPE);
        let problem = kind.is_some_and(|t| t == "application/problem+json");
        let status = response.status();
        Ok(Page { url, status, problem, text: response.text().await.context("reading the reply")? })
    }

    pub fn jmap<T: DeserializeOwned>(self) -> Result<T> {
        if !self.status.is_success() {
            return Err(self.failure());
        }
        serde_json::from_str(&self.text)
            .with_context(|| format!("stalwart: {}: unreadable reply", self.url.path()))
    }

    pub fn failure(self) -> anyhow::Error {
        let what = format!("stalwart: {}: HTTP {}", self.url.path(), self.status);
        let problem: Value = serde_json::from_str(&self.text).unwrap_or_default();
        let kind = match (self.status.as_u16(), self.problem) {
            (401, _) => Failure::TokenRefused,
            (_, true) if problem["limit"] == "maxConcurrentRequests" => Failure::Busy,
            (_, true) => Failure::Unprocessed,
            _ => Failure::Transient,
        };
        kind.with(if self.problem { format!("{what}: {}", self.text) } else { what })
    }
}

pub fn redirect(
    from: &Pinned,
    status: StatusCode,
    location: Option<&str>,
) -> Result<(Pinned, bool)> {
    let location = location
        .with_context(|| format!("{}: HTTP {status} without a usable Location", from.path()))?;
    let keeps = matches!(status, StatusCode::TEMPORARY_REDIRECT | StatusCode::PERMANENT_REDIRECT);
    Ok((from.join(location)?, keeps))
}

pub struct Web(Client);

impl Web {
    pub fn new(cookies: bool) -> Result<Web> {
        let client = Client::builder().redirect(redirect::Policy::none()).https_only(true);
        Ok(Web(client.cookie_store(cookies).connect_timeout(TIMEOUT).build()?))
    }

    async fn go(&self, request: RequestBuilder) -> reqwest::Result<Response> {
        let request = request.build()?;
        #[cfg(test)]
        {
            if let Some(response) = crate::tests::wire(&request) {
                return Ok(response);
            }
        }
        self.0.execute(request).await
    }

    pub async fn send(
        &self,
        method: Method,
        url: Pinned,
        body: Body<'_>,
        bearer: Option<&Secret>,
    ) -> Result<Page> {
        let (mut method, mut url, mut body) = (method, url, body);
        for _ in 0..10 {
            let mut request = self.0.request(method.clone(), url.1.clone()).timeout(TIMEOUT);
            if let Some(token) = bearer {
                request = request.bearer_auth(token.expose());
            }
            request = match body {
                Body::Empty => request,
                Body::Form(fields) => request.form(fields),
                Body::Json(json) => request.json(json),
            };
            let response = self.go(request).await.with_context(|| url.path().to_owned())?;
            if !response.status().is_redirection() {
                return Page::read(url, response).await;
            }
            let location = response.headers().get(header::LOCATION).and_then(|l| l.to_str().ok());
            let (next, keeps) = redirect(&url, response.status(), location)?;
            url = next;
            if !keeps {
                (method, body) = (Method::GET, Body::Empty);
            }
        }
        bail!("{}: more than 10 redirects", url.path())
    }

    /// The response to `send(body)`, failing once [`TIMEOUT`] passes with no piece of `bytes` taken, or no answer
    /// after the last, so an upload is bounded by its progress rather than its size.
    pub async fn paced<T>(
        bytes: &[u8],
        send: impl AsyncFnOnce(reqwest::Body) -> Result<T>,
    ) -> Result<T> {
        let taken = Arc::new(Mutex::new(Instant::now()));
        let body = reqwest::Body::wrap(Paced(Bytes::copy_from_slice(bytes), taken.clone()));
        let stalled = async {
            loop {
                let due = *taken.lock().unwrap() + TIMEOUT;
                if Instant::now() >= due {
                    break;
                }
                tokio::time::sleep_until(due).await;
            }
        };
        tokio::select! {
            sent = send(body) => sent,
            () = stalled => bail!("the upload made no progress for {}s", TIMEOUT.as_secs()),
        }
    }

    pub async fn fetch(
        &self,
        method: Method,
        url: &Pinned,
        bearer: &Secret,
        build: impl FnOnce(RequestBuilder) -> RequestBuilder,
    ) -> Result<Response> {
        let request = self.0.request(method, url.1.clone()).bearer_auth(bearer.expose());
        let response = self.go(build(request)).await.context("stalwart")?;
        match response.status().is_success() {
            true => Ok(response),
            false => Err(Page::read(url.clone(), response).await?.failure()),
        }
    }
}

struct Paced(Bytes, Arc<Mutex<Instant>>);

impl http_body::Body for Paced {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _: &mut Waker<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        *self.1.lock().unwrap() = Instant::now();
        let piece = self.0.len().min(64 * 1024);
        Poll::Ready((piece > 0).then(|| Ok(Frame::data(self.0.split_to(piece)))))
    }
}

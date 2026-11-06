use std::fmt;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

/// A request (RFC 8620 3.3): the capabilities it uses and its method calls, in order.
#[derive(Clone, Debug, Serialize)]
pub struct Request {
    #[serde(rename = "methodCalls")]
    calls: Vec<(&'static str, Value, String)>,
    using: Vec<&'static str>,
}

/// One call in a [`Request`], to find its answer by in the [`Response`].
#[derive(Clone, Copy, Debug)]
pub struct Call {
    id: usize,
    method: &'static str,
}

impl Request {
    pub fn new(using: &[&'static str]) -> Request {
        Request { calls: vec![], using: using.to_vec() }
    }

    pub fn call(&mut self, method: &'static str, args: Value) -> Call {
        let call = Call { id: self.calls.len(), method };
        self.calls.push((method, args, call.id.to_string()));
        call
    }
}

crate::object!(@struct
    /// A response (RFC 8620 3.4): the answers to a [`Request`]'s calls.
    #[serde(default)]
    Response {
        replies: Vec<(String, Value, String)> = "methodResponses",
    }
);

crate::object! {
    /// A method error (RFC 8620 3.6.2).
    MethodError { kind: String = "type", description: Option<String> = "description" }
}

/// Why a call has no answer to read.
#[derive(Debug)]
pub enum Failed {
    /// Neither its method's answer nor a method error is in the response under its id.
    Unanswered,
    Refused(MethodError),
    /// An answer that doesn't read as the one asked for.
    Unreadable(serde_json::Error),
}

impl Response {
    /// The call's answer: the first in the response under its id and its method's name, or a
    /// method error under its id.
    pub fn answer<T: DeserializeOwned>(&self, call: Call) -> Result<T, Failed> {
        let id = call.id.to_string();
        let ours = |(name, _, of): &&(String, Value, String)| {
            *of == id && (name == call.method || name == "error")
        };
        match self.replies.iter().find(ours).ok_or(Failed::Unanswered)? {
            (name, error, _) if name == "error" => {
                Err(Failed::Refused(MethodError::deserialize(error).unwrap_or_default()))
            }
            (_, answer, _) => T::deserialize(answer).map_err(Failed::Unreadable),
        }
    }
}

impl fmt::Display for MethodError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.description {
            Some(description) => write!(f, "{}: {description}", self.kind),
            None => f.write_str(&self.kind),
        }
    }
}

impl fmt::Display for Failed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failed::Unanswered => f.write_str("the response holds no answer to it"),
            Failed::Refused(error) => write!(f, "refused: {error}"),
            Failed::Unreadable(e) => write!(f, "its answer doesn't read: {e}"),
        }
    }
}

impl std::error::Error for Failed {}

#[cfg(test)]
#[path = "../tests/request.rs"]
mod tests;

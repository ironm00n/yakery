//! One agent session's Slack. `agent-slackd` holds Slack's Socket Mode connection and keeps every message that
//! passes the checks in an inbox on disk; `agent-slack` waits for the next one, posts, and reads threads through it.

pub mod api;
pub mod daemon;
mod inbox;
pub mod slack;

pub use inbox::*;

#[cfg(test)]
mod tests;

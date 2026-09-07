//! Wizard state machine, guards and on-disk persistence.
//!
//! The session is the source of truth; the UI renders [`SessionSnapshot`].
//! Every step has a list of guards; `next()` only advances when each guard
//! passes or has been explicitly overridden with a reason.

pub mod engine;
pub mod guards;
pub mod model;
pub mod store;

pub use engine::SessionEngine;
pub use guards::{FcStatus, GuardOutcome, GuardResult};
pub use model::*;
pub use store::SessionStore;

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("session not found: {0}")]
    NotFound(String),
    #[error("guard '{0}' did not pass: {1}")]
    GuardFailed(String, String),
    #[error("guard '{0}' cannot be overridden")]
    NotOverridable(String),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, SessionError>;

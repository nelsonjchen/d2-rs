// SPDX-License-Identifier: MPL-2.0
//! Cooperative export context: the caller's first cause and the renderer timeout.
use d2_rs_core::{CancellationToken, LayoutError};
use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

#[derive(Clone, Debug, Default)]
pub struct RenderContext {
    token: CancellationToken,
    deadline: Option<Instant>,
}
impl RenderContext {
    pub(crate) fn new(token: Option<&CancellationToken>, timeout_ns: i64) -> Self {
        Self {
            token: token.map(CancellationToken::child).unwrap_or_default(),
            deadline: Some(Instant::now() + Duration::from_nanos(timeout_ns as u64)),
        }
    }
    /// A child context; cancellation does not propagate back to the caller.
    pub fn from_token(token: &CancellationToken) -> Self {
        Self {
            token: token.child(),
            deadline: None,
        }
    }
    pub fn check(&self) -> Result<(), RenderError> {
        if let Some(error) = self.token.error() {
            return Err(RenderError::Context(error));
        }
        if self.deadline.is_some_and(|d| Instant::now() >= d) {
            self.token.expire();
            return Err(RenderError::Context(self.token.error().unwrap()));
        }
        Ok(())
    }
    pub(crate) fn check_string(&self) -> Result<(), String> {
        self.check().map_err(|e| e.to_string())
    }
    /// Native stages latch their first error as text. Keep the typed context
    /// source only when that error is the current context cause, with its exact
    /// source-defined stage prefixes intact.
    pub(crate) fn error(&self, message: String) -> RenderError {
        if let Err(RenderError::Context(cause)) = self.check() {
            let suffix = cause.to_string();
            if message == suffix || message.ends_with(&format!(": {suffix}")) {
                return RenderError::StageContext { message, cause };
            }
        }
        RenderError::Message(message)
    }
}

#[derive(Debug)]
pub enum RenderError {
    Context(LayoutError),
    StageContext { message: String, cause: LayoutError },
    Message(String),
}
impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Context(cause) => cause.fmt(f),
            Self::StageContext { message, .. } | Self::Message(message) => message.fmt(f),
        }
    }
}
impl std::error::Error for RenderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Context(cause) | Self::StageContext { cause, .. } => Some(cause),
            Self::Message(_) => None,
        }
    }
}
impl From<String> for RenderError {
    fn from(message: String) -> Self {
        Self::Message(message)
    }
}

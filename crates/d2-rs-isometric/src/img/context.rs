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
    #[cfg(not(target_arch = "wasm32"))]
    timer: Option<std::sync::Arc<DeadlineTimer>>,
}

// Go's context.WithTimeout publishes expiration once; ctx.Err only reads the
// cause. Visibility polls millions of times, so reading the clock at each poll
// adds work that the Go renderer never performs.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
struct DeadlineTimer {
    stop: Option<std::sync::mpsc::Sender<()>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for DeadlineTimer {
    fn drop(&mut self) {
        self.stop.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl RenderContext {
    pub(crate) fn new(token: Option<&CancellationToken>, timeout_ns: i64) -> Self {
        let token = token.map(CancellationToken::child).unwrap_or_default();
        let deadline = Instant::now() + Duration::from_nanos(timeout_ns as u64);
        #[cfg(not(target_arch = "wasm32"))]
        let timer = {
            let (stop, receive) = std::sync::mpsc::channel();
            let child = token.clone();
            std::thread::Builder::new()
                .name("d2-isometric-deadline".into())
                .spawn(move || {
                    if matches!(
                        receive.recv_timeout(deadline.saturating_duration_since(Instant::now())),
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                    ) {
                        child.expire();
                    }
                })
                .ok()
                .map(|worker| {
                    std::sync::Arc::new(DeadlineTimer {
                        stop: Some(stop),
                        worker: Some(worker),
                    })
                })
        };
        if Instant::now() >= deadline {
            token.expire();
        }
        Self {
            token,
            deadline: Some(deadline),
            #[cfg(not(target_arch = "wasm32"))]
            timer,
        }
    }
    /// A child context; cancellation does not propagate back to the caller.
    pub fn from_token(token: &CancellationToken) -> Self {
        Self {
            token: token.child(),
            deadline: None,
            #[cfg(not(target_arch = "wasm32"))]
            timer: None,
        }
    }
    pub fn check(&self) -> Result<(), RenderError> {
        if let Some(error) = self.token.error() {
            return Err(RenderError::Context(error));
        }
        // WASM has no timer thread. Retain cooperative deadline checking there,
        // and use it as a fallback if the native host cannot start a timer.
        #[cfg(target_arch = "wasm32")]
        let check_clock = true;
        #[cfg(not(target_arch = "wasm32"))]
        let check_clock = self.timer.is_none();
        if check_clock && self.deadline.is_some_and(|d| Instant::now() >= d) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_deadline_preserves_the_first_cause_and_does_not_cancel_parent() {
        let parent = CancellationToken::new();
        let context = RenderContext::new(Some(&parent), 1);
        let clone = context.clone();
        drop(context);
        assert!(matches!(
            clone.check(),
            Err(RenderError::Context(LayoutError::DeadlineExceeded))
        ));
        assert!(parent.error().is_none());
        parent.cancel();
        assert!(matches!(
            clone.check(),
            Err(RenderError::Context(LayoutError::DeadlineExceeded))
        ));
        let context = RenderContext::new(Some(&parent), 1);
        assert!(matches!(
            context.check(),
            Err(RenderError::Context(LayoutError::Cancelled))
        ));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn deadline_timer_survives_context_cloning_and_expires_without_clock_polls() {
        let context = RenderContext::new(None, 1_000_000);
        let token = context.token.clone();
        let clone = context.clone();
        drop(context);
        for _ in 0..1000 {
            if token.error().is_some() {
                assert!(matches!(
                    clone.check(),
                    Err(RenderError::Context(LayoutError::DeadlineExceeded))
                ));
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("deadline timer did not publish expiration");
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

// SPDX-License-Identifier: MPL-2.0
//! D2 lib/time.WithTimeout and one context for a complete document invocation.
use d2_rs_core::CancellationToken;
use std::cell::Cell;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

thread_local! {
    static ACTIVE_DOCUMENT: Cell<usize> = const { Cell::new(0) };
}

/// Go multiplies a signed duration before testing whether it is positive.
pub fn duration(seconds: i64) -> Option<Duration> {
    let nanos = seconds.wrapping_mul(1_000_000_000);
    (nanos > 0).then(|| Duration::from_nanos(nanos as u64))
}

pub struct Scope {
    cancellation: CancellationToken,
    stop: Option<mpsc::Sender<()>>,
    timer: Option<JoinHandle<()>>,
    _cancellation_scope: super::watch::CancellationGuard,
}

impl Scope {
    pub fn document(timeout: Option<Duration>) -> Self {
        let cancellation = super::watch::active_cancellation_token()
            .map_or_else(CancellationToken::new, |parent| parent.child());
        let cancellation_scope = super::watch::track_cancellation(cancellation.clone());
        ACTIVE_DOCUMENT.with(|depth| depth.set(depth.get() + 1));
        let (stop, timer) = if let Some(timeout) = timeout {
            let (send, receive) = mpsc::channel();
            let token = cancellation.clone();
            let timer = thread::spawn(move || {
                if matches!(
                    receive.recv_timeout(timeout),
                    Err(mpsc::RecvTimeoutError::Timeout)
                ) {
                    token.expire();
                }
            });
            (Some(send), Some(timer))
        } else {
            (None, None)
        };
        Self {
            cancellation,
            stop,
            timer,
            _cancellation_scope: cancellation_scope,
        }
    }

    pub fn layout(timeout: Option<Duration>) -> Option<Self> {
        if ACTIVE_DOCUMENT.with(|depth| depth.get() != 0) {
            return None;
        }
        timeout.map(|timeout| Self::document(Some(timeout)))
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(timer) = self.timer.take() {
            let _ = timer.join();
        }
        self.cancellation.cancel();
        ACTIVE_DOCUMENT.with(|depth| depth.set(depth.get() - 1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_seconds_match_go_duration_multiplication() {
        for seconds in [0, -1, i64::MIN, i64::MAX] {
            assert_eq!(duration(seconds), None, "{seconds}");
        }
        assert_eq!(duration(120), Some(Duration::from_secs(120)));
        // A negative integer may wrap to a positive duration, just as in Go.
        assert_eq!(
            duration(-9_223_372_037),
            Some(Duration::from_nanos(9_223_372_036_709_551_616))
        );
    }

    #[test]
    fn document_boards_share_deadline_and_watch_parent_survives() {
        let parent = CancellationToken::new();
        let _watch = super::super::watch::track_cancellation(parent.clone());
        {
            let document = Scope::document(None);
            document.cancellation.expire();
            for _ in 0..3 {
                assert!(Scope::layout(Some(Duration::from_secs(120))).is_none());
                assert_eq!(
                    super::super::watch::active_cancellation_token()
                        .unwrap()
                        .error()
                        .unwrap()
                        .to_string(),
                    "context deadline exceeded"
                );
            }
        }
        assert!(!parent.is_cancelled());
        assert!(
            !super::super::watch::active_cancellation_token()
                .unwrap()
                .is_cancelled()
        );
        let next_document = Scope::document(None);
        assert!(!next_document.cancellation.is_cancelled());
        parent.cancel();
        assert_eq!(
            next_document.cancellation.error().unwrap().to_string(),
            "context canceled"
        );
    }

    #[test]
    fn completed_scope_stops_timer_and_restores_outer_context() {
        let token = {
            let scope = Scope::document(Some(Duration::from_secs(120)));
            scope.cancellation.clone()
        };
        assert_eq!(token.error().unwrap().to_string(), "context canceled");
        assert_eq!(ACTIVE_DOCUMENT.with(Cell::get), 0);
        assert!(super::super::watch::active_cancellation_token().is_none());
    }
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! coordinator.go: recover each worker, cancel siblings, and join before return.

use crate::LayoutError;
use d2_rs_core::CancellationToken;
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;

pub(super) fn generate<T: Send, R: Send>(
    parent: Option<&CancellationToken>,
    routers: Vec<(&'static str, T)>,
    worker: impl Fn(T) -> R + Send + Sync,
) -> Result<Vec<(&'static str, R)>, LayoutError> {
    if routers.len() > 3 {
        return Err(LayoutError::BadRouteState(format!(
            "route-search work limit exceeded: edge routing declared {} flavors; limit 3",
            routers.len()
        )));
    }
    let child = parent.map_or_else(CancellationToken::new, CancellationToken::child);
    let run = |(flavor, router)| {
        let response = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::internal::with_cancellation(Some(&child), || worker(router))
        }));
        if response.is_err() {
            // Do this before waiting for the other workers: they may be
            // waiting for cancellation. Never inspect the panic payload.
            child.cancel();
        }
        (flavor, response)
    };
    // Indexed collection retains declared order and joins every job, including
    // jobs not started when a sibling panics. Ordinary errors are worker data.
    #[cfg(not(target_arch = "wasm32"))]
    let results: Vec<_> = routers.into_par_iter().map(run).collect();
    #[cfg(target_arch = "wasm32")]
    let results: Vec<_> = routers.into_iter().map(run).collect();
    child.cancel();
    let mut responses = Vec::with_capacity(results.len());
    for (flavor, result) in results {
        match result {
            Ok(response) => responses.push((flavor, response)),
            Err(_) => {
                return Err(LayoutError::InvariantViolation(format!(
                    "edge routing flavor {flavor} violated an invariant"
                )));
            }
        }
    }
    // A recovered panic takes precedence over caller cancellation, even when
    // the first declared panic completed after another worker's cancellation.
    if let Some(error) = parent.and_then(CancellationToken::error) {
        return Err(error);
    }
    Ok(responses)
}

pub(super) fn successful<T>(
    responses: impl IntoIterator<Item = Result<T, LayoutError>>,
    aggregate_error: Option<LayoutError>,
) -> Result<Vec<T>, LayoutError> {
    if let Some(error) = aggregate_error {
        return Err(error);
    }
    let mut successes = Vec::new();
    let mut first_error = None;
    for response in responses {
        match response {
            Ok(value) => successes.push(value),
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    if successes.is_empty() {
        return Err(first_error.unwrap_or_else(|| {
            LayoutError::InvariantViolation("edge routing produced no flavor responses".into())
        }));
    }
    Ok(successes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    const FLAVORS: [&str; 3] = ["ShortestToLongest", "LongestToShortest", "Default"];
    fn routers() -> Vec<(&'static str, usize)> {
        FLAVORS
            .into_iter()
            .enumerate()
            .map(|(i, name)| (name, i))
            .collect()
    }

    #[test]
    fn panic_is_fatal_despite_success_and_every_worker_finishes() {
        let completed = AtomicUsize::new(0);
        let parent = CancellationToken::new();
        let result = generate(Some(&parent), routers(), |index| {
            completed.fetch_add(1, Ordering::SeqCst);
            if index == 0 {
                std::panic::panic_any(Arc::new(AtomicUsize::new(0)));
            }
            index
        });
        assert!(
            matches!(result, Err(LayoutError::InvariantViolation(ref message))
            if message == "edge routing flavor ShortestToLongest violated an invariant")
        );
        assert_eq!(completed.load(Ordering::SeqCst), 3);
        assert!(parent.error().is_none());
    }

    #[test]
    fn declared_panic_precedes_caller_cancellation() {
        for _ in 0..25 {
            let parent = CancellationToken::new();
            let result = generate(Some(&parent), routers(), |_| -> () {
                parent.cancel();
                panic!("sensitive worker payload");
            });
            assert!(
                matches!(result, Err(LayoutError::InvariantViolation(ref message))
                if message == "edge routing flavor ShortestToLongest violated an invariant")
            );
        }
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn panic_cancels_active_siblings_and_drains_them() {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(3)
            .build()
            .unwrap();
        let ready = std::sync::Barrier::new(3);
        let canceled = AtomicUsize::new(0);
        let completed = AtomicUsize::new(0);
        let result = pool.install(|| {
            generate(None, routers(), |index| {
                ready.wait();
                if index == 0 {
                    panic!("stop siblings");
                }
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                while !crate::internal::cancellation_requested() {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "sibling never canceled"
                    );
                    std::thread::yield_now();
                }
                canceled.fetch_add(1, Ordering::SeqCst);
                completed.fetch_add(1, Ordering::SeqCst);
            })
        });
        assert!(matches!(result, Err(LayoutError::InvariantViolation(_))));
        assert_eq!(canceled.load(Ordering::SeqCst), 2);
        assert_eq!(completed.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn ordinary_failure_allows_fallback_without_canceling_siblings() {
        let responses = generate(None, routers(), |index| {
            assert!(!crate::internal::cancellation_requested());
            if index == 0 {
                Err(LayoutError::BadRouteState("shortest failed".into()))
            } else {
                Ok(index)
            }
        })
        .unwrap();
        assert_eq!(responses.iter().map(|r| r.0).collect::<Vec<_>>(), FLAVORS);
        assert_eq!(
            successful(responses.into_iter().map(|r| r.1), None).unwrap(),
            vec![1, 2]
        );
    }

    #[test]
    fn all_failures_use_declared_order_and_empty_is_invariant() {
        let responses = generate(None, routers(), |index| -> Result<(), LayoutError> {
            Err(LayoutError::BadRouteState(FLAVORS[index].into()))
        })
        .unwrap();
        assert!(
            matches!(successful(responses.into_iter().map(|r| r.1), None),
            Err(LayoutError::BadRouteState(message)) if message == FLAVORS[0])
        );
        assert!(matches!(
            successful::<()>([], None),
            Err(LayoutError::InvariantViolation(_))
        ));
    }

    #[test]
    fn aggregate_failure_rejects_successful_fallback() {
        let error = LayoutError::WorkLimit {
            seed: 0,
            location: "EdgeRouting",
            limit: 1,
        };
        assert!(matches!(
            successful([Ok(42)], Some(error)),
            Err(LayoutError::WorkLimit { .. })
        ));
    }

    #[test]
    fn deadline_is_inherited_and_checked_after_join() {
        let parent = CancellationToken::new();
        let completed = AtomicUsize::new(0);
        let result = generate(Some(&parent), routers(), |_| {
            parent.expire();
            assert!(matches!(
                crate::internal::cancellation_layout_error(),
                Some(LayoutError::DeadlineExceeded)
            ));
            completed.fetch_add(1, Ordering::SeqCst);
        });
        assert!(matches!(result, Err(LayoutError::DeadlineExceeded)));
        assert_eq!(completed.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn excessive_flavors_do_not_start_workers() {
        let mut jobs = routers();
        jobs.push(("extra", 3));
        let result = generate(None, jobs, |_| panic!("must not run"));
        assert!(matches!(result, Err(LayoutError::BadRouteState(_))));
    }
}

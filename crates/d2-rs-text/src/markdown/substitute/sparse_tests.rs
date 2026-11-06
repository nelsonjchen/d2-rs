// SPDX-License-Identifier: MPL-2.0
//! Original master's matcher storage/allocation contract, using real allocations.
use super::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct CountAllocations;
thread_local! {
    static COUNT: Cell<Option<usize>> = const { Cell::new(None) };
}
fn allocation() {
    let _ = COUNT.try_with(|count| {
        if let Some(value) = count.get() {
            count.set(Some(value + 1));
        }
    });
}
// Only the calling test thread is counted. Bookkeeping uses a const-initialized
// TLS Cell and performs no allocation, including inside allocator callbacks.
unsafe impl GlobalAlloc for CountAllocations {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        allocation();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        allocation();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        allocation();
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: CountAllocations = CountAllocations;

fn evaluate(request: &serde_json::Value) -> serde_json::Value {
    let variables = serde_json::from_value(request["variables"].clone()).unwrap();
    let matcher = Matcher::new(&variables, &mut || None).unwrap();
    // An empty Rust BTreeMap has no heap allocation, unlike Go's nonnil map.
    let sparse = matcher
        .states
        .iter()
        .all(|s| s.terminal.is_none() || s.next.is_empty());
    let input = request["input"].as_str().unwrap();
    let limit = request["limit"].as_u64().unwrap();
    let run = || {
        let result = matcher.replace(input, limit, &mut || None).unwrap();
        assert_eq!(result.0, input);
        assert_eq!(result.1, 0);
    };
    run(); // testing.AllocsPerRun's warm-up, outside measurement.
    let runs = request["runs"].as_u64().unwrap();
    assert!(runs > 0);
    let mut allocations = 0;
    for _ in 0..runs {
        COUNT.with(|c| c.set(Some(0)));
        run();
        allocations += COUNT.with(|c| c.replace(None).unwrap());
    }
    let (result, used) = matcher.replace(input, limit, &mut || None).unwrap();
    serde_json::json!({"got":{"result":result,"used":used,"error":"","terminal_storage_sparse":sparse,"allocations_within_limit":allocations as u64 <= runs},"observations":{"allocations":allocations,"runs":runs}})
}

#[test]
fn master_sparse_scoreboard() {
    if let Ok(path) = std::env::var("D2RS_REMAINING_REQUEST") {
        let request = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        std::fs::write(
            std::env::var("D2RS_REMAINING_RESULT").unwrap(),
            serde_json::to_vec(&evaluate(&request)).unwrap(),
        )
        .unwrap();
        return;
    }
    let request = serde_json::json!({"variables":{"one":"1","two":"2","three":"3"},"input":format!("${{unknown}} {}", "plain text without matches ".repeat(1<<15)),"limit":32,"runs":5});
    let result = evaluate(&request);
    assert_eq!(result["got"]["terminal_storage_sparse"], true);
    assert_eq!(result["got"]["allocations_within_limit"], true, "{result}");
}

#[test]
fn counter_observes_allocation_and_reallocation_on_this_thread() {
    COUNT.with(|c| c.set(Some(0)));
    let mut data = Vec::<u8>::with_capacity(std::hint::black_box(8));
    data.reserve(std::hint::black_box(128));
    std::hint::black_box(&data);
    let count = COUNT.with(|c| c.replace(None).unwrap());
    assert!(count >= 2, "allocator counter missed the negative control");
}

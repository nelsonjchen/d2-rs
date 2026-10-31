// SPDX-License-Identifier: MPL-2.0
//! Public cancellation-tree contract probe, compared with pinned Go context.
use d2_rs_core::CancellationToken;
use serde::{Deserialize, Serialize};
use std::io;

#[derive(Deserialize, Serialize)]
struct Input {
    name: String,
    parents: Vec<usize>,
    cause: String,
    pre_cancel: String,
    #[serde(default)]
    drop_intermediates: bool,
}
#[derive(Serialize)]
struct Output {
    input: Input,
    observed_nodes: Vec<usize>,
    after_repeat: Vec<String>,
    after_first_completion: Vec<String>,
}
fn causes(nodes: &[CancellationToken]) -> Vec<String> {
    nodes
        .iter()
        .map(|node| {
            node.error()
                .map_or_else(String::new, |error| error.to_string())
        })
        .collect()
}
fn main() {
    let inputs: Vec<Input> = serde_json::from_reader(io::stdin().lock()).unwrap();
    let mut outputs = Vec::new();
    for input in inputs {
        let root = CancellationToken::new();
        let mut nodes = vec![root.clone()];
        for (index, parent) in input.parents.iter().copied().enumerate() {
            let node = nodes[parent].child();
            if index == 0 {
                match input.pre_cancel.as_str() {
                    "deadline" => node.expire(),
                    "cancel" => node.cancel(),
                    "" => (),
                    other => panic!("unknown child cause: {other}"),
                }
            }
            nodes.push(node);
        }
        let mut has_child = vec![false; nodes.len()];
        for &parent in &input.parents {
            has_child[parent] = true;
        }
        let observed_nodes: Vec<usize> = (0..nodes.len())
            .filter(|&index| !input.drop_intermediates || index == 0 || !has_child[index])
            .collect();
        let retained: Vec<_> = observed_nodes
            .iter()
            .map(|&index| nodes[index].clone())
            .collect();
        if input.drop_intermediates {
            drop(nodes);
        }
        let first = root.clone();
        let deadline = input.cause == "deadline";
        let worker = std::thread::spawn(move || {
            if deadline {
                first.expire();
            } else {
                first.cancel();
            }
        });
        while !root.is_cancelled() {
            std::hint::spin_loop();
        }
        root.cancel();
        let after_repeat = causes(&retained);
        worker.join().unwrap();
        outputs.push(Output {
            input,
            observed_nodes,
            after_repeat,
            after_first_completion: causes(&retained),
        });
    }
    serde_json::to_writer_pretty(io::stdout().lock(), &outputs).unwrap();
}

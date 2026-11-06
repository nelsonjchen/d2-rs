// SPDX-License-Identifier: MPL-2.0
//! Original compound-cancellation input replay. Go calibrates Err-call counts;
//! Rust calibrates its existing compound stage/publication boundaries instead.
//! The schedule is deliberately NOT evidence of identical context poll counts.
use super::super::compound_routes::interior_api_tests::{decode, view};
use super::*;
use serde_json::{Value, json};

thread_local! {
    static SCHEDULE: RefCell<Option<(usize, usize)>> = const { RefCell::new(None) };
}

pub(super) fn checkpoint(token: Option<&CancellationToken>) {
    SCHEDULE.with(|schedule| {
        if let Some((checks, limit)) = schedule.borrow_mut().as_mut() {
            *checks += 1;
            if *limit > 0 && *checks >= *limit {
                token.unwrap().cancel();
            }
        }
    });
}

fn winner(graph: ArenaGraph) -> SeedCandidate<SeedLayout> {
    // CompoundCandidate is observed before upstream's optional score gate.
    // An infinite incumbent admits every finite proposal in this test adapter.
    SeedCandidate {
        index: 0,
        seed: 0,
        score: evaluation::RaceSelectionScore {
            penalty: f64::INFINITY,
            area: f64::INFINITY,
        },
        value: SeedLayout {
            graph,
            result: LayoutResult {
                removed_edges: Default::default(),
                source_edge_storage: Default::default(),
                synthetic_root: None,
                route_curves: Default::default(),
                sequence_lifelines: Vec::new(),
                boxes: Default::default(),
                routes: Default::default(),
                edge_labels: Default::default(),
                edge_label_percentage_commits: Default::default(),
                node_labels: Default::default(),
                report: LayoutReport {
                    selected_seed: 0,
                    selected_pass: 0,
                    score: Default::default(),
                    improvement_passes: 1,
                    candidates: Vec::new(),
                    warnings: Vec::new(),
                },
            },
        },
    }
}

fn run(input: &Value, limit: usize, pre_cancel: bool) -> (Value, usize) {
    let (graph, arena) = decode(input);
    assert_eq!(view(&arena, &input["view"]), input["view"]);
    let winner = winner(arena);
    let token = CancellationToken::new();
    if pre_cancel {
        token.cancel();
    }
    SCHEDULE.with(|schedule| assert!(schedule.replace(Some((0, limit))).is_none()));
    let result = build_compound_candidate(&graph, &winner, Some(&token));
    let checks = SCHEDULE.with(|schedule| schedule.replace(None).unwrap().0);
    let (candidate, error) = match result {
        Ok(Some(candidate)) => {
            let refined = compound_routes::preserve_compound_routes(
                &graph,
                &winner.value.graph,
                &candidate.value.graph,
                Some(&token),
            )
            .unwrap();
            (view(&refined, &input["view"]), String::new())
        }
        Ok(None) => panic!("original fixture must enter compound work"),
        Err(error) => (Value::Null, error.to_string()),
    };
    (
        json!({"graph":candidate,"cause":error,"input_after":view(&winner.value.graph,&input["view"])}),
        checks,
    )
}

#[test]
#[ignore = "explicit original-call replay requires input/output paths"]
fn replay() {
    let request = std::env::var_os("D2RS_TALA_CANCELLATION_REQUEST").unwrap();
    let response = std::env::var_os("D2RS_TALA_CANCELLATION_RESPONSE").unwrap();
    let requests: Vec<Value> = serde_json::from_slice(&std::fs::read(request).unwrap()).unwrap();
    let mut results = Vec::new();
    for q in &requests {
        let pre = q["cancel"]["kind"] == "pre";
        let probe_only = q["cancel"]["kind"] == "probe";
        let (probe_checks, limit) = if pre || probe_only {
            (0, 0)
        } else {
            let (probe, count) = run(&q["graph"], 0, false);
            assert_eq!(probe["cause"], "");
            assert_eq!(probe["input_after"], q["graph"]["view"]);
            let index = q["cancel"]["fraction_index"].as_u64().unwrap();
            let limit = match index {
                0 => count / 3,
                1 => 2 * count / 3,
                2 => count - 1,
                _ => unreachable!(),
            };
            assert!(limit >= 2 && limit < count);
            (count, limit)
        };
        let (result, checks) = run(&q["graph"], limit, pre);
        assert!(
            pre || probe_only || checks == limit,
            "cancellation must stop at the armed boundary"
        );
        let diagnostic = result["cause"].clone();
        results.push(json!({"result":result,"diagnostic":diagnostic,"native_schedule":{"probe_checks":probe_checks,"limit":limit,"checks":checks}}));
    }
    std::fs::write(response, serde_json::to_vec(&results).unwrap()).unwrap();
}

use super::*;

#[test]
fn requested_target_trace_survives_unused_leaf_hashes() {
    for requested in [false, true] {
        let calls = std::cell::Cell::new(0);
        let own = diagnostic_hash_input(
            || {
                calls.set(calls.get() + 1);
                vec![42]
            },
            requested,
        );
        assert_eq!(calls.get(), usize::from(requested));
        assert_eq!(own(), vec![42]);
        assert_eq!(calls.get(), 1);
    }
    let calls = std::cell::Cell::new(0);
    let _unused_leaf = diagnostic_hash_input(
        || {
            calls.set(calls.get() + 1);
            vec![42]
        },
        true,
    );
    assert_eq!(calls.get(), 1);
}

#[test]
fn planned_svg_preserves_direct_render_bytes_and_target_selection() {
    for source in [
        "a -> b\n",
        "a\nlayers: {empty: {}; one: {b; layers: {two: {c}}}}\nscenarios: {other: {d}}\n",
        "layers: {\"a.b\": {x; steps: {next: {y}}}}\n",
    ] {
        let plan = source_board_plan(source.as_bytes(), "-").unwrap();
        for target in ["", "*"].into_iter().chain(
            plan.all_targets
                .iter()
                .flat_map(|target| [target.clone(), format!("{target}.*")])
                .collect::<Vec<_>>()
                .iter()
                .map(String::as_str),
        ) {
            let args = vec![
                "--engine=dagre".into(),
                "--bundle=false".into(),
                format!("--target={target}"),
            ];
            let expected = render_source_partial(source.as_bytes(), &args);
            let planned = render_source_partial_with_board_targets(
                source.as_bytes(),
                &args,
                Some(&plan.all_targets),
            );
            assert_eq!(planned, expected, "{target}: {source}");
        }
    }
}

// Manual stage profile for the workload named by the performance brief.
// It also compares the complete SVG bytes on every old/new paired render.
#[test]
#[ignore = "set D2_RS_PROFILE_SOURCE to a source file; run with --nocapture"]
fn profile_svg_source_stages() {
    let path = std::env::var("D2_RS_PROFILE_SOURCE").expect("source path");
    let source = std::fs::read(&path).unwrap();
    let args = vec![
        format!("--path={path}"),
        "--engine=dagre".into(),
        "--bundle=false".into(),
    ];
    let source_args = extract_render_options(&args).unwrap().0;
    let plan = source_board_plan(&source, &path).unwrap();
    let mut samples = BTreeMap::<&str, Vec<f64>>::new();
    for iteration in 0..21 {
        macro_rules! measure {
            ($name:literal, $body:expr) => {{
                let started = std::time::Instant::now();
                let value = $body;
                let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                if iteration > 0 {
                    samples.entry($name).or_default().push(elapsed);
                }
                value
            }};
        }
        measure!("source_config", source_config(&source, &path).unwrap());
        measure!("board_plan", source_board_plan(&source, &path).unwrap());
        let SourceRenderLayout {
            layout: (graph, result, labels, styles),
            target_metadata,
        } = measure!(
            "layout",
            source_render_layout_with_theme(&source, &source_args, None, false).unwrap()
        );
        // Metadata is now produced by layout's existing compilation.
        measure!("hash_metadata", target_metadata.unwrap());
        measure!(
            "own_hash_input",
            d2_rs_render::target_hash_input(
                &graph,
                &result,
                &labels,
                &styles,
                &RenderOptions::default(),
                false,
            )
        );
        // Alternate the pair's order to reduce drift bias.
        let fallback = || render_source_partial(&source, &args).unwrap();
        let planned = || {
            render_source_partial_with_board_targets(&source, &args, Some(&plan.all_targets))
                .unwrap()
        };
        let (before, after) = if iteration % 2 == 0 {
            (
                measure!("render_fallback", fallback()),
                measure!("render_planned", planned()),
            )
        } else {
            let after = measure!("render_planned", planned());
            (measure!("render_fallback", fallback()), after)
        };
        assert_eq!(after, before);
    }
    let medians: BTreeMap<_, _> = samples
        .iter_mut()
        .map(|(name, times)| {
            times.sort_by(f64::total_cmp);
            (*name, (times[9] + times[10]) / 2.0)
        })
        .collect();
    println!(
        "SOURCE_PROFILE {}",
        serde_json::json!({"median_ms":medians,"samples":samples})
    );
}

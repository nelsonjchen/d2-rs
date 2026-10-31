// SPDX-License-Identifier: MPL-2.0
//! d2cli/isometric_watch.go: a static board preview with full-tree navigation.
use super::*;

#[cfg(test)]
#[path = "isometric_watch_api_tests.rs"]
mod api_tests;

fn navigation_path(parts: &[String]) -> String {
    if parts.is_empty() {
        "/index.svg".into()
    } else {
        format!(
            "/{}.svg",
            parts
                .iter()
                .map(|part| url_path_escape(part))
                .collect::<Vec<_>>()
                .join("/")
        )
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render(
    input: &[u8],
    input_path: &str,
    target: &str,
    layout: &str,
    seeds: Option<&str>,
    pad: Option<f64>,
    scale: Option<f64>,
    salt: Option<&str>,
    theme: Option<i64>,
    center: Option<bool>,
    adapter_args: &[String],
    no_xml: bool,
    omit: bool,
) -> Result<Vec<u8>, String> {
    let target = target.trim_end_matches('*').trim_end_matches('.');
    let (plan, selected) = source_board_plan_for_target(input, input_path, target)?;
    let selected_is_drawable = if selected.is_empty() {
        plan.root_renderable
    } else {
        plan.renderable_targets.contains(&selected)
    };
    let board = if selected_is_drawable {
        selected.clone()
    } else {
        plan.targets
            .iter()
            .find(|child| selected.is_empty() || child.starts_with(&format!("{selected}.")))
            .cloned()
            .ok_or("isometric preview has no drawable boards")?
    };
    let mut args = source_path_args(input_path);
    args.push(format!("--engine={layout}"));
    for (name, value) in [
        ("seeds", seeds.map(str::to_owned)),
        ("pad", pad.map(|v| v.to_string())),
        ("theme", theme.map(|v| v.to_string())),
        ("center", center.map(|v| v.to_string())),
    ] {
        if let Some(value) = value {
            args.push(format!("--{name}={value}"));
        }
    }
    args.extend(adapter_args.iter().cloned());
    args.push(format!("--target={board}"));
    let frame = render_source_target_animation(input, &args)?;
    let value = source_diagram_json(&frame, board.is_empty(), true)?;
    let paths = std::iter::once(("root".to_owned(), navigation_path(&[])))
        .chain(plan.all_targets.iter().map(|target| {
            (
                format!("root.{target}"),
                navigation_path(&compiled_board_components(target)),
            )
        }))
        .collect::<BTreeMap<_, _>>();
    render_target_preview(
        &value,
        &paths,
        input_path,
        frame.render_options.theme_id,
        pad,
        scale,
        salt,
        center,
        no_xml,
        omit,
    )
}

/// renderIsometricPreview's static-board copy and painter. Animation and dark
/// theme options belong to the file export and do not enter this API.
#[allow(clippy::too_many_arguments)]
fn render_target_preview(
    value: &serde_json::Value,
    paths: &BTreeMap<String, String>,
    input_path: &str,
    theme_id: Option<i64>,
    pad: Option<f64>,
    scale: Option<f64>,
    salt: Option<&str>,
    center: Option<bool>,
    no_xml: bool,
    omit: bool,
) -> Result<Vec<u8>, String> {
    fn first_drawable(value: &serde_json::Value) -> Option<&serde_json::Value> {
        if value.is_null() {
            return None;
        }
        if value["isFolderOnly"] != true {
            return Some(value);
        }
        for field in ["layers", "scenarios", "steps"] {
            if let Some(children) = value[field].as_array() {
                for child in children {
                    if let Some(board) = first_drawable(child) {
                        return Some(board);
                    }
                }
            }
        }
        None
    }
    let mut value = first_drawable(value)
        .ok_or("isometric preview has no drawable boards")?
        .clone();
    // Go copies the board and detaches descendants before HashID/Render.
    for field in ["layers", "scenarios", "steps"] {
        value[field] = serde_json::Value::Null;
    }
    for field in ["shapes", "connections"] {
        if value[field].as_array().is_some_and(Vec::is_empty) {
            value[field] = serde_json::Value::Null;
        }
    }
    let relink = |item: &mut serde_json::Value| {
        if let Some(path) = item["link"].as_str().and_then(|link| paths.get(link)) {
            item["link"] = path.clone().into();
        }
    };
    relink(&mut value["root"]);
    for field in ["shapes", "connections"] {
        if let Some(items) = value[field].as_array_mut() {
            for item in items {
                relink(item);
            }
        }
    }
    let json = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
    let diagram: Diagram = serde_json::from_slice(&json).map_err(|e| e.to_string())?;
    let hash_data = d2_rs_render::target::TargetDiagram::decode_json(&json)
        .map_err(|e| e.to_string())?
        .ok_or("null diagram")?
        .hash_input()
        .map_err(|e| e.to_string())?;
    let hash = d2_rs_render::target_hash_id_from_input(&hash_data, salt);
    let (assets, fonts) = resources(input_path)?;
    let options = img::render::Options {
        render: d2_rs_isometric::scene::RenderOpts {
            theme_id,
            theme_overrides: diagram
                .config
                .as_ref()
                .and_then(|c| c.theme_overrides.clone()),
        },
        width: 1600,
        height: 1000,
        fit_content: true,
        fonts: Some(fonts),
        assets: Some(assets),
        ..Default::default()
    };
    let token = watch::active_cancellation_token();
    let svg = img::render::render_svg_with_context(token.as_ref(), Some(&diagram), Some(&options))
        .map_err(|e| e.to_string())?;
    wrap(
        &svg,
        &hash,
        pad.unwrap_or(100.0) as i64,
        scale,
        center.unwrap_or(false),
        no_xml,
        omit,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_preview_links_to_the_full_tree() {
        let input = b"a: {link: layers.first}\nlayers: {first: {b: {link: root}; sibling: {link: root.layers.second}}; second: {c}}";
        let svg = render(
            input,
            "input.d2",
            "layers.first",
            "dagre",
            None,
            None,
            Some(0.2),
            None,
            None,
            None,
            &[],
            false,
            false,
        )
        .unwrap();
        let svg = String::from_utf8(svg).unwrap();
        assert!(svg.contains("D2 isometric diagram"));
        assert!(svg.contains("href=\"/index.svg\""));
        assert!(svg.contains("href=\"/layers/second.svg\""));
        assert!(!svg.contains("href=\"/layers/first.svg\""));
    }

    #[test]
    fn folder_preview_selects_only_its_first_drawable_descendant() {
        let input = b"layers: {first: {b: {link: root.layers.second}}; second: {c}}";
        let svg = render(
            input,
            "input.d2",
            "",
            "dagre",
            None,
            None,
            Some(0.2),
            None,
            None,
            None,
            &[],
            false,
            false,
        )
        .unwrap();
        let svg = String::from_utf8(svg).unwrap();
        assert!(svg.contains("href=\"/layers/second.svg\""));
        assert_eq!(
            svg.matches("<title>D2 isometric diagram</title>").count(),
            1
        );
    }
}

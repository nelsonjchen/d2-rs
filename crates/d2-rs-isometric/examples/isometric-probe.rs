// SPDX-License-Identifier: MPL-2.0
//! Native side of scripts/isometric-scoreboard.py; same stdin protocol as
//! tools/isometric-probe/main.go (one JSON request per line).

use std::io::BufRead;
use std::path::Path;

use d2_rs_isometric::scene::{RenderOpts, build_scene};
use d2_rs_isometric::target::Diagram;
#[path = "support/asset_sources.rs"]
mod asset_sources;

fn boards<'a>(diagram: &'a Diagram, name: String, out: &mut Vec<(String, &'a Diagram)>) {
    if !diagram.is_folder_only {
        out.push((name.clone(), diagram));
    }
    for (kind, children) in [
        ("layers", &diagram.layers),
        ("scenarios", &diagram.scenarios),
        ("steps", &diagram.steps),
    ] {
        for (index, child) in children.iter().flatten().enumerate() {
            if let Some(child) = child {
                boards(child, format!("{name}.{kind}.{index}"), out);
            }
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for line in std::io::stdin().lock().lines() {
        let request: serde_json::Value = serde_json::from_str(&line?)?;
        let board_path = request["board"].as_str().ok_or("board")?;
        let out = Path::new(request["out"].as_str().ok_or("out")?);
        std::fs::create_dir_all(out)?;
        let diagram: Diagram = serde_json::from_slice(&std::fs::read(board_path)?)?;
        let mut theme_id = request["themeID"].as_i64();
        let mut overrides = None;
        if let Some(config) = &diagram.config {
            overrides = config.theme_overrides.clone();
            if theme_id.is_none() {
                theme_id = config.theme_id;
            }
        }
        let opts = RenderOpts {
            theme_id,
            theme_overrides: overrides,
        };
        let mut list = Vec::new();
        boards(&diagram, "isometric".into(), &mut list);
        for (name, board) in list {
            let (extension, payload) = if request["mode"] == "scene" {
                match build_scene(board, Some(&opts)) {
                    Ok(scene) => ("json", serde_json::to_vec(&scene)?),
                    Err(error) => ("err", error.into_bytes()),
                }
            } else {
                match d2_rs_isometric::img::render::render_svg(
                    board,
                    &d2_rs_isometric::img::render::Options {
                        render: opts.clone(),
                        width: 1200,
                        height: 1200,
                        fit_content: true,
                        assets: asset_sources::options(&request)?,
                        ..Default::default()
                    },
                ) {
                    Ok(svg) => ("svg", svg.into_bytes()),
                    Err(error) => ("err", error.into_bytes()),
                }
            };
            std::fs::write(out.join(format!("{name}.{extension}")), payload)?;
        }
    }
    Ok(())
}

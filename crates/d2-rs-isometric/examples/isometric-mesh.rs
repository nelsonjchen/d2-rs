// SPDX-License-Identifier: MPL-2.0
//! Per-board mesh debugger. Cargo requires explicit diagnostic-traces; normal
//! library and probe builds cannot construct these diagnostic JSON arguments.
#[path = "support/asset_sources.rs"]
mod asset_sources;
#[path = "support/mesh_dump.rs"]
mod mesh_dump;
use d2_rs_isometric::{
    img::render::{Options, build_native_scene},
    scene::{RenderOpts, build_scene},
    target::Diagram,
};
use std::{io::BufRead, path::Path};
fn boards<'a>(d: &'a Diagram, name: String, out: &mut Vec<(String, &'a Diagram)>) {
    if !d.is_folder_only {
        out.push((name.clone(), d));
    }
    for (kind, children) in [
        ("layers", &d.layers),
        ("scenarios", &d.scenarios),
        ("steps", &d.steps),
    ] {
        for (i, c) in children.iter().flatten().enumerate() {
            if let Some(c) = c {
                boards(c, format!("{name}.{kind}.{i}"), out);
            }
        }
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    for line in std::io::stdin().lock().lines() {
        let r: serde_json::Value = serde_json::from_str(&line?)?;
        let d: Diagram =
            serde_json::from_slice(&std::fs::read(r["board"].as_str().ok_or("board")?)?)?;
        let out = Path::new(r["out"].as_str().ok_or("out")?);
        std::fs::create_dir_all(out)?;
        let render = RenderOpts {
            theme_id: r["themeID"]
                .as_i64()
                .or_else(|| d.config.as_ref().and_then(|c| c.theme_id)),
            theme_overrides: d.config.as_ref().and_then(|c| c.theme_overrides.clone()),
        };
        let mut list = vec![];
        boards(&d, "isometric".into(), &mut list);
        for (name, d) in list {
            let result = build_scene(d, Some(&render)).and_then(|s| {
                build_native_scene(
                    &s,
                    1200,
                    1200,
                    &Options {
                        render: render.clone(),
                        width: 1200,
                        height: 1200,
                        fit_content: true,
                        assets: asset_sources::options(&r)?,
                        ..Default::default()
                    },
                )
            });
            let (ext, data) = match result {
                Ok(s) => ("mesh.json", serde_json::to_vec(&mesh_dump::dump(&s))?),
                Err(e) => ("err", e.into_bytes()),
            };
            std::fs::write(out.join(format!("{name}.{ext}")), data)?;
        }
    }
    Ok(())
}

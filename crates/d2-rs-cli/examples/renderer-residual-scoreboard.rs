// SPDX-License-Identifier: MPL-2.0
//! Original feature checks, icon ownership, PDF fill and E2E Render requests.
#[path = "../src/layout_features.rs"]
mod layout_features;
use base64::{Engine, engine::general_purpose::STANDARD};
use d2_rs_core::{Graph, RootStyle, ast::Shared};
use d2_rs_isometric::{
    scene::build_scene,
    target::{Diagram, Userinfo},
};
use d2_rs_language::{compiler, parse};
use serde_json::{Value, json};

fn features(request: &Value) -> Result<Value, String> {
    let input = request["input"].as_str().unwrap();
    let graph = if input.is_empty() {
        Graph::default()
    } else {
        let parsed = parse("test.d2", input.as_bytes(), None);
        if let Some(error) = parsed.error {
            return Err(error.to_string());
        }
        let (compiler, root) =
            compiler::compile_ir(Shared::new(parsed.map)).map_err(|e| e.to_string())?;
        assert!(compiler.errors.is_empty());
        compiler::lower_graph(&compiler, root).map_err(|e| e.to_string())?
    };
    layout_features::check(request["layout"].as_str().unwrap(), &graph)?;
    Ok(Value::Null)
}

fn icon(request: &Value) -> Result<Value, String> {
    let mut diagram: Diagram = serde_json::from_value(request["diagram"].clone()).unwrap();
    for (shape, user) in diagram
        .shapes
        .as_mut()
        .unwrap()
        .iter_mut()
        .zip(request["users"].as_array().unwrap())
    {
        if user.is_null() {
            continue;
        }
        shape.icon.as_mut().unwrap().user = Some(if user["hasPassword"].as_bool().unwrap() {
            Userinfo::with_password(
                user["username"].as_str().unwrap(),
                user["password"].as_str().unwrap(),
            )
        } else {
            Userinfo::new(user["username"].as_str().unwrap())
        });
    }
    let scene = build_scene(&diagram, None)?;
    let source = diagram.shapes()[0].icon.as_ref().unwrap();
    let owned = scene.nodes[0].metadata.original.icon.as_ref().unwrap();
    Ok(
        json!({"scene":scene, "urlOwned":!std::ptr::eq(source, owned),
        "userOwned":!std::ptr::eq(source.user.as_ref().unwrap(), owned.user.as_ref().unwrap())}),
    )
}

fn pdf(request: &Value) -> Result<Value, String> {
    let pages = request["pages"].as_array().unwrap();
    let mut images = Vec::new();
    let mut boards = Vec::new();
    let mut titles = Vec::new();
    let mut fills = Vec::new();
    let nav = pages[0]["nav"].as_bool().unwrap();
    let theme = pages[0]["theme"].as_i64().unwrap();
    for page in pages {
        // This adapter covers the original test's unlinked, zero-pad PNG pages.
        assert!(page["shapes"].is_null());
        assert_eq!(page["pad"].as_i64(), Some(0));
        assert_eq!(page["x"].as_f64(), Some(0.0));
        assert_eq!(page["y"].as_f64(), Some(0.0));
        assert_eq!(page["theme"].as_i64(), Some(theme));
        assert_eq!(page["nav"].as_bool(), Some(nav));
        images.push(STANDARD.decode(page["png"].as_str().unwrap()).unwrap());
        let path = page["titles"].as_array().unwrap();
        boards.push(
            path.iter()
                .map(|t| t["BoardID"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>(),
        );
        titles.push(
            path.iter()
                .map(|t| t["Name"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>(),
        );
        let graph = Graph {
            root_style: Some(RootStyle {
                fill: Some(page["fill"].as_str().unwrap().to_owned()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let options = d2_rs_render::RenderOptions {
            theme_id: Some(theme),
            ..Default::default()
        };
        fills.push(d2_rs_export::document_page_fill(&graph, &options).map_err(|e| e.to_string())?);
    }
    // Reject a page-map mismatch instead of silently projecting another mapping.
    let page_map: Value = boards
        .iter()
        .enumerate()
        .map(|(i, p)| (p.last().unwrap().clone(), json!(i)))
        .collect::<serde_json::Map<String, Value>>()
        .into();
    assert!(pages.iter().all(|p| p["pageMap"] == page_map));
    let bytes = d2_rs_export::package_pdf_pages_with_navigation_metadata_for_theme(
        &images,
        &vec![Vec::new(); pages.len()],
        &boards,
        &titles,
        nav,
        Some(theme),
        &fills,
    )
    .map_err(|e| e.to_string())?;
    Ok(json!(STANDARD.encode(bytes)))
}

fn evaluate(request: &Value) -> Value {
    let result = match request["operation"].as_str().unwrap() {
        "features" => features(request),
        "icon" => icon(request),
        "pdf" => pdf(request),
        "isometric_svg" => {
            let diagram: Diagram = serde_json::from_value(request["diagram"].clone()).unwrap();
            let options = &request["options"];
            let render = &options["Render"];
            let opts = d2_rs_isometric::img::render::Options {
                render: d2_rs_isometric::scene::RenderOpts {
                    theme_id: render["ThemeID"].as_i64(),
                    theme_overrides: if render["ThemeOverrides"].is_null() {
                        None
                    } else {
                        Some(serde_json::from_value(render["ThemeOverrides"].clone()).unwrap())
                    },
                },
                width: options["Width"].as_i64().unwrap(),
                height: options["Height"].as_i64().unwrap(),
                fit_content: options["FitContent"].as_bool().unwrap(),
                format: options["Format"].as_str().unwrap().to_owned(),
                ..Default::default()
            };
            d2_rs_isometric::img::render::render_with_context(None, Some(&diagram), Some(&opts))
                .map(|bytes| json!(STANDARD.encode(bytes)))
                .map_err(|e| e.to_string())
        }
        operation => panic!("unexpected operation {operation}"),
    };
    match result {
        Ok(output) => json!({"output":output,"error":""}),
        Err(error) => json!({"output":null,"error":error}),
    }
}

fn main() {
    let input = std::env::args().nth(1).expect("requests.json");
    let requests: Vec<Value> = serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
    println!(
        "{}",
        json!(requests.iter().map(evaluate).collect::<Vec<_>>())
    );
}

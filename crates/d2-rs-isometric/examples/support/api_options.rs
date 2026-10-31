// SPDX-License-Identifier: MPL-2.0
//! Serializable caller import limits for the public image API replay.
use d2_rs_scenebuild::{AssetOptions, SvgImportBudget, SvgImportLimits};
use serde_json::Value;

pub fn assets(options: &Value) -> Option<AssetOptions> {
    let assets = options.get("Assets").filter(|v| !v.is_null())?;
    assert!(
        assets.get("Resolver").is_none_or(Value::is_null),
        "resolver requires an explicit replay adapter"
    );
    let l = &assets["SVGImportLimits"];
    let b = &assets["SVGImportBudget"];
    let n = |v: &Value, key: &str| v[key].as_i64().unwrap_or(0);
    Some(AssetOptions {
        svg_import_limits: SvgImportLimits {
            max_bytes: n(l, "MaxBytes"),
            max_depth: n(l, "MaxDepth"),
            max_elements: n(l, "MaxElements"),
            max_attributes: n(l, "MaxAttributes"),
            max_attribute_bytes: n(l, "MaxAttributeBytes"),
            max_path_commands: n(l, "MaxPathCommands"),
            max_transform_functions: n(l, "MaxTransformFunctions"),
            max_use_depth: n(l, "MaxUseDepth"),
            max_resources: n(l, "MaxResources"),
        },
        svg_import_budget: SvgImportBudget {
            max_source_bytes: n(b, "MaxSourceBytes"),
            max_elements: n(b, "MaxElements"),
            max_attributes: n(b, "MaxAttributes"),
            max_attribute_bytes: n(b, "MaxAttributeBytes"),
            max_path_commands: n(b, "MaxPathCommands"),
            max_transform_functions: n(b, "MaxTransformFunctions"),
            max_declared_resources: n(b, "MaxDeclaredResources"),
            max_expanded_use_instances: n(b, "MaxExpandedUseInstances"),
        },
        ..Default::default()
    })
}

// SPDX-License-Identifier: MPL-2.0
// Source: D2 v0.9.0 d2renderers/d2scenebuild/assets.go SVG import branch,
// remainingSVGImportLimits/reserveSVGImport/mergeSVGImportAssets.
use super::{
    svg_budget::SvgDocumentBudget,
    svg_image::RasterAsset,
    svg_import::{Imported, Outcome},
};
use std::collections::BTreeMap;
fn context(c: Option<&d2_rs_core::CancellationToken>) -> Result<(), String> {
    match c.and_then(d2_rs_core::CancellationToken::error) {
        Some(d2_rs_core::LayoutError::DeadlineExceeded) => Err("context deadline exceeded".into()),
        Some(_) => Err("context canceled".into()),
        None => Ok(()),
    }
}
#[derive(Debug, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct DocumentImports {
    pub budget: SvgDocumentBudget,
    /// None represents a non-raster document resource.
    pub resources: BTreeMap<String, Option<RasterAsset>>,
}
impl DocumentImports {
    /// Merge immutable raster resources atomically in sorted asset-ID order.
    pub fn merge(
        &mut self,
        object: &str,
        imported: &BTreeMap<String, Option<RasterAsset>>,
        cancel: Option<&d2_rs_core::CancellationToken>,
    ) -> Result<(), String> {
        let mut pending = Vec::new();
        for (id, asset) in imported {
            context(cancel)?;
            let Some(asset) = asset else {
                return Err(format!(
                    "scene: {object} SVG asset {} has unsupported imported resource type <nil>",
                    d2_rs_core::quote_go_bytes(id.as_bytes())
                ));
            };
            if let Some(existing) = self.resources.get(id) {
                if existing.as_ref() != Some(asset) {
                    return Err(format!(
                        "scene: {object} SVG asset {} collides with a different document resource",
                        d2_rs_core::quote_go_bytes(id.as_bytes())
                    ));
                }
                continue;
            }
            pending.push((id.clone(), asset.clone()));
        }
        for (id, asset) in pending {
            self.resources.insert(id, Some(asset));
        }
        Ok(())
    }
    /// Import an already-redacted source label with cumulative work ceilings.
    /// Reservation precedes asset merge and remains charged on merge failure.
    pub fn import_labeled(
        &mut self,
        object: &str,
        source: &str,
        data: &[u8],
        mut limits: [usize; 9],
        cancel: Option<&d2_rs_core::CancellationToken>,
    ) -> Result<Imported, String> {
        let remaining = self.budget.remaining(
            object,
            [
                limits[0], limits[2], limits[3], limits[4], limits[5], limits[6], limits[8],
            ],
        )?;
        for (slot, value) in [0, 2, 3, 4, 5, 6, 8].into_iter().zip(remaining) {
            limits[slot] = value;
        }
        let outcome = super::svg_import::import_labeled(source, data, limits, None, cancel)
            .map_err(|e| format!("scene: {object} SVG asset: {e}"))?;
        let Outcome::Imported(imported) = outcome else {
            unreachable!("complete importer admits image assets")
        };
        let m = &imported.metrics;
        self.budget.reserve(
            object,
            [
                m.source_bytes,
                m.parsed_elements,
                m.parsed_attributes,
                m.parsed_attribute_bytes,
                m.parsed_path_commands,
                m.parsed_transform_funcs,
                m.declared_resources,
                m.expanded_use_instances,
            ],
            m.emitted_elements,
            m.emitted_path_commands,
        )?;
        self.merge(object, &imported.assets.entries, cancel)?;
        Ok(imported)
    }
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod tests {
    use super::*;
    #[derive(serde::Deserialize)]
    struct Call {
        error: String,
        state: DocumentImports,
    }
    #[derive(serde::Deserialize)]
    struct Row {
        input: String,
        limits: [usize; 9],
        cancel: String,
        before: DocumentImports,
        calls: Vec<Call>,
    }
    #[test]
    fn original_cumulative_import_reservation_and_asset_merge_state() {
        let rows: Vec<Row> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-document-import-v090.json"
        ))
        .unwrap();
        for (i, r) in rows.into_iter().enumerate() {
            let token = d2_rs_core::CancellationToken::new();
            let c = match r.cancel.as_str() {
                "cancelled" => {
                    token.cancel();
                    Some(&token)
                }
                "deadline" => {
                    token.expire();
                    Some(&token)
                }
                _ => None,
            };
            let mut state = r.before;
            for (n, e) in r.calls.iter().enumerate() {
                let result =
                    state.import_labeled("shape \"n\"", "math", r.input.as_bytes(), r.limits, c);
                assert_eq!(
                    result.as_ref().err().map(String::as_str).unwrap_or(""),
                    e.error,
                    "profile {i} call {n}"
                );
                assert_eq!(state, e.state, "profile {i} call {n}");
            }
        }
    }
}

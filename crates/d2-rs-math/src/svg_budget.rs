// SPDX-License-Identifier: MPL-2.0
// Source: D2 d2renderers/d2scenebuild/assets.go
// remainingSVGImportLimits and reserveSVGImport. Importer integration is separate.

const NAMES: [&str; 8] = [
    "source bytes",
    "element count",
    "attribute count",
    "attribute bytes",
    "path command count",
    "transform function count",
    "declared resource count",
    "expanded use instance count",
];

/// Source-ordered document counters, independently of per-import limits.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SvgDocumentBudget {
    pub maximum: [usize; 8],
    pub used: [usize; 8],
}

impl SvgDocumentBudget {
    /// Clamp byte/element/attribute/attribute-byte/path/transform/resource
    /// ceilings. Declared resources and expanded uses share the last ceiling.
    pub fn remaining(&self, object: &str, mut limits: [usize; 7]) -> Result<[usize; 7], String> {
        if self.maximum.contains(&0) {
            return Err(format!(
                "scene: {object} SVG asset requires positive document-wide import budgets"
            ));
        }
        for (index, name) in NAMES.iter().enumerate() {
            let remaining = self.maximum[index].saturating_sub(self.used[index]);
            if remaining == 0 {
                return Err(format!(
                    "scene: {object} SVG asset has no remaining cumulative {name} budget"
                ));
            }
            let slot = index.min(6);
            limits[slot] = limits[slot].min(remaining);
        }
        Ok(limits)
    }

    /// Reserve successful importer metrics atomically. Element/path charges
    /// use the larger parsed or emitted total, as in the original builder.
    pub fn reserve(
        &mut self,
        object: &str,
        mut parsed: [usize; 8],
        emitted_elements: usize,
        emitted_paths: usize,
    ) -> Result<(), String> {
        parsed[1] = parsed[1].max(emitted_elements);
        parsed[4] = parsed[4].max(emitted_paths);
        for (index, name) in NAMES.iter().enumerate() {
            if self.used[index] > self.maximum[index]
                || parsed[index] > self.maximum[index] - self.used[index]
            {
                return Err(format!(
                    "scene: {object} SVG asset causes cumulative {name} to exceed limit {}",
                    self.maximum[index]
                ));
            }
        }
        for (used, amount) in self.used.iter_mut().zip(parsed) {
            *used += amount;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_document_budget_controls() {
        fn array<const N: usize>(row: &serde_json::Value, key: &str) -> [usize; N] {
            row[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_u64().unwrap() as usize)
                .collect::<Vec<_>>()
                .try_into()
                .unwrap()
        }
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-document-budget-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let mut budget = SvgDocumentBudget {
                maximum: array(row, "maximum"),
                used: array(row, "used"),
            };
            let remaining = budget.remaining("shape \"n\" latex", array(row, "limits"));
            assert_eq!(
                serde_json::to_value(remaining.ok()).unwrap(),
                row["remaining"],
                "{row}"
            );
            assert_eq!(
                budget
                    .remaining("shape \"n\" latex", array(row, "limits"))
                    .err(),
                row["remaining_error"].as_str().map(str::to_owned),
                "{row}"
            );
            let result = budget.reserve(
                "shape \"n\" latex",
                array(row, "parsed"),
                row["emitted_elements"].as_u64().unwrap() as usize,
                row["emitted_paths"].as_u64().unwrap() as usize,
            );
            assert_eq!(
                result.err(),
                row["reserve_error"].as_str().map(str::to_owned),
                "{row}"
            );
            assert_eq!(
                serde_json::to_value(budget.used).unwrap(),
                row["after"],
                "{row}"
            );
        }
    }
}

// Copyright (c) 2018, 2020 Kiel University and others.
// Copyright (c) 2026 The elk-go Authors.
// SPDX-License-Identifier: EPL-2.0
// elk-go/internal/rectpacking/options.go and core/registry.go.
use crate::property::{enum_value, flag, integer};
use serde_json::Value;
pub(super) struct Profile {
    pub packing: String,
    pub whitespace: String,
    pub goal: String,
    pub shift: bool,
    pub reevaluate: bool,
    pub iterations: i32,
}
impl Profile {
    pub fn new(o: &Value) -> Self {
        Self {
            packing: enum_value(
                o,
                "rectpacking.packing.strategy",
                &["COMPACTION", "SIMPLE", "NONE"],
                "COMPACTION",
            ),
            whitespace: enum_value(
                o,
                "rectpacking.whiteSpaceElimination.strategy",
                &["EQUAL_BETWEEN_STRUCTURES", "TO_ASPECT_RATIO", "NONE"],
                "NONE",
            ),
            goal: enum_value(
                o,
                "rectpacking.widthApproximation.optimizationGoal",
                &["MAX_SCALE_DRIVEN", "ASPECT_RATIO_DRIVEN", "AREA_DRIVEN"],
                "MAX_SCALE_DRIVEN",
            ),
            shift: flag(o, "rectpacking.widthApproximation.lastPlaceShift", true),
            reevaluate: flag(
                o,
                "rectpacking.packing.compaction.rowHeightReevaluation",
                false,
            ),
            iterations: integer(o, "rectpacking.packing.compaction.iterations").unwrap_or(1),
        }
    }
}

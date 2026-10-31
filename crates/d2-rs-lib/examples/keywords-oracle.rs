// SPDX-License-Identifier: MPL-2.0
//! Development-only inventory of the native language keyword tables.
use d2_tala_rs::ast::keywords::*;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

fn main() {
    let mut result = BTreeMap::new();
    for (name, values) in [
        ("simple", SIMPLE_RESERVED_KEYWORDS),
        ("holders", RESERVED_KEYWORD_HOLDERS),
        ("composite", COMPOSITE_RESERVED_KEYWORDS),
        ("style", STYLE_KEYWORDS),
        ("boards", BOARD_KEYWORDS),
    ] {
        result.insert(name, json!(values.iter().collect::<BTreeSet<_>>()));
    }
    let reserved = SIMPLE_RESERVED_KEYWORDS
        .iter()
        .chain(STYLE_KEYWORDS)
        .chain(COMPOSITE_RESERVED_KEYWORDS)
        .filter(|value| is_reserved_keyword(value))
        .collect::<BTreeSet<_>>();
    result.insert("reserved", json!(reserved));
    for (name, values) in [
        ("near", NEAR_CONSTANTS),
        ("labels", LABEL_POSITIONS),
        ("tooltips", TOOLTIP_POSITIONS),
        ("fill_patterns", FILL_PATTERNS),
        ("text_transforms", TEXT_TRANSFORMS),
    ] {
        result.insert(name, json!(values));
    }
    let positions = LABEL_POSITIONS
        .iter()
        .map(|name| (*name, label_position(name).unwrap().d2_name()))
        .collect::<BTreeMap<_, _>>();
    result.insert("positions", json!(positions));
    println!("{}", serde_json::to_string(&result).unwrap());
}

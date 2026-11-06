// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! d2ast.Number: source spelling independent of an exact rational value.

use super::{ArrayNode, Child, Node, Range, Scalar, Shared, Value};
pub use num_bigint::BigInt;
pub use num_rational::BigRational;
use serde::{Serialize, Serializer};

#[derive(Clone, Debug, Default, Serialize)]
pub struct Number {
    pub range: Range,
    pub raw: String,
    #[serde(serialize_with = "serialize_rational")]
    pub value: Option<Shared<BigRational>>,
}

// Go math/big.Rat MarshalText emits an integer when the denominator is one,
// otherwise a reduced fraction. JSON encodes that text as a string, not f64.
fn serialize_rational<S: Serializer>(
    value: &Option<Shared<BigRational>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        None => serializer.serialize_none(),
        Some(value) => serializer.serialize_str(&value.read().to_string()),
    }
}

impl Node for Number {
    fn node_type(&self) -> std::borrow::Cow<'_, str> {
        "number".into()
    }
    fn range(&self) -> Range {
        self.range.clone()
    }
    fn children(&self) -> Vec<Child> {
        Vec::new()
    }
}
impl ArrayNode for Number {}
impl Value for Number {}
impl Scalar for Number {
    fn scalar_string(&self) -> std::borrow::Cow<'_, str> {
        self.raw.as_str().into()
    }
}

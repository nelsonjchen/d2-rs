// SPDX-License-Identifier: MPL-2.0
//! Native port of D2's renderer-neutral scene model, d2renderers/d2scene at
//! the pinned master. The JSON encoding mirrors Go's encoding/json of the
//! same structs (PascalCase fields, uint8 enums as numbers, color.NRGBA as
//! {R,G,B,A}) with interfaces tagged by their Go type name, and byte slices
//! as base64; tools/scene-probe writes the same encoding from Go.

pub mod aspect;
pub mod geometry;
pub mod paint;
pub mod path;
pub mod scene;

pub use aspect::aspect_ratio_matrix;
pub use geometry::{go_max, go_min, Bounds, Box, Matrix, Point};
pub use paint::{
    GradientStop, LineCap, LineJoin, LinearGradient, Nrgba, Paint, PaintUnits, PatternPaint,
    RadialGradient, SpreadMethod, Stroke,
};
pub use path::{endpoint_arc_to_center, CenterArc, FillRule, Path, PathCommand, PathCommandKind};
pub use scene::*;

/// A Go uint8 enumeration: a transparent u8 newtype with named constants,
/// encoded as its number like encoding/json.
#[macro_export]
macro_rules! go_enum {
    ($name:ident { $($variant:ident = $value:expr),* $(,)? }) => {
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub u8);
        #[allow(dead_code)]
        impl $name {
            $(pub const $variant: Self = Self($value);)*
        }
    };
}

pub(crate) mod bytes {
    use base64::Engine as _;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(data: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(data))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let text = Option::<String>::deserialize(deserializer)?.unwrap_or_default();
        base64::engine::general_purpose::STANDARD
            .decode(text)
            .map_err(serde::de::Error::custom)
    }
}

/// An f64 kept as its bit pattern so the containing type stays hashable
/// (Go's d2scene.Font is a comparable map key); encoded as the number.
pub(crate) mod float_bits {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bits: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_f64(f64::from_bits(*bits))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        Ok(f64::deserialize(deserializer)?.to_bits())
    }
}

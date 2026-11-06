// SPDX-License-Identifier: MPL-2.0
//! scene.go budget.check: visit native scalar values before JSON can erase
//! nonfinite floats. Struct field names do not contribute to source budgets.

use serde::{Serialize, Serializer, ser};

use super::{COORDINATE_LIMIT, MAX_BYTES, MAX_ENTRIES};

#[derive(Default)]
pub(super) struct Budget {
    bytes: usize,
    entries: usize,
}

type Error = serde_json::Error;
type Result = std::result::Result<(), Error>;

fn fail(message: impl std::fmt::Display) -> Error {
    ser::Error::custom(message)
}

// scene.go:345–390 reflects declaration order, including private Userinfo.
// Explicit visitors for icon owners preserve that order without changing JSON.
// Exhaustive destructuring makes additions to the target boundary a compile error.
macro_rules! fields {
    ($budget:expr; $($field:ident),* $(,)?) => {$( $budget.check($field)?; )*};
}

impl Budget {
    fn icon(&mut self, icon: &Option<crate::target::Url>) -> std::result::Result<(), String> {
        if let Some(icon) = icon {
            let crate::target::Url {
                scheme,
                opaque,
                user,
                host,
                path,
                raw_path,
                omit_host,
                force_query,
                raw_query,
                fragment,
                raw_fragment,
            } = icon;
            fields!(self; scheme, opaque);
            if let Some(user) = user {
                self.check(user.username())?;
                self.check(&user.password())?;
            }
            fields!(self; host, path, fragment, raw_query, raw_path, raw_fragment,
                force_query, omit_host);
        }
        Ok(())
    }

    pub(super) fn shape(
        &mut self,
        value: &crate::target::Shape,
    ) -> std::result::Result<(), String> {
        let crate::target::Shape {
            id,
            type_,
            classes,
            pos,
            width,
            height,
            opacity,
            stroke_dash,
            stroke_width,
            border_radius,
            fill,
            fill_pattern,
            stroke,
            animated,
            shadow,
            three_dee,
            multiple,
            double_border,
            tooltip,
            link,
            pretty_link,
            icon,
            icon_border_radius,
            icon_position,
            blend,
            fields,
            methods,
            columns,
            content_aspect_ratio,
            text,
            label_position,
            tooltip_position,
            z_index,
            level,
            primary_accent_color,
            secondary_accent_color,
            neutral_accent_color,
        } = value;
        fields!(self; id, type_, classes, pos, width, height, opacity, stroke_dash,
            stroke_width, border_radius, fill, fill_pattern, stroke, animated,
            shadow, three_dee, multiple, double_border, tooltip, link, pretty_link);
        self.icon(icon)?;
        fields!(self; icon_border_radius, icon_position, blend, fields, methods,
            columns, content_aspect_ratio, text, label_position, tooltip_position,
            z_index, level, primary_accent_color, secondary_accent_color,
            neutral_accent_color);
        Ok(())
    }

    pub(super) fn connection(
        &mut self,
        value: &crate::target::Connection,
    ) -> std::result::Result<(), String> {
        let crate::target::Connection {
            id,
            classes,
            src,
            src_arrow,
            src_label,
            dst,
            dst_arrow,
            dst_label,
            opacity,
            stroke_dash,
            stroke_width,
            stroke,
            fill,
            border_radius,
            text,
            label_position,
            label_percentage,
            link,
            pretty_link,
            route,
            is_curve,
            animated,
            tooltip,
            icon,
            icon_position,
            icon_border_radius,
            z_index,
        } = value;
        fields!(self; id, classes, src, src_arrow, src_label, dst, dst_arrow,
            dst_label, opacity, stroke_dash, stroke_width, stroke, fill,
            border_radius, text, label_position, label_percentage, link,
            pretty_link, route, is_curve, animated, tooltip);
        self.icon(icon)?;
        fields!(self; icon_position, icon_border_radius, z_index);
        Ok(())
    }

    pub(super) fn shapes(
        &mut self,
        values: &[crate::target::Shape],
    ) -> std::result::Result<(), String> {
        self.collection(values.len())
            .map_err(|error| error.to_string())?;
        for value in values {
            self.shape(value)?;
        }
        Ok(())
    }

    pub(super) fn connections(
        &mut self,
        values: &[crate::target::Connection],
    ) -> std::result::Result<(), String> {
        self.collection(values.len())
            .map_err(|error| error.to_string())?;
        for value in values {
            self.connection(value)?;
        }
        Ok(())
    }

    pub(super) fn legend(
        &mut self,
        value: &Option<crate::target::Legend>,
    ) -> std::result::Result<(), String> {
        if let Some(crate::target::Legend {
            label,
            shapes,
            connections,
        }) = value
        {
            self.check(label)?;
            self.shapes(shapes.as_deref().unwrap_or_default())?;
            self.connections(connections.as_deref().unwrap_or_default())?;
        }
        Ok(())
    }
}

impl Budget {
    pub(super) fn check<T: Serialize + ?Sized>(
        &mut self,
        value: &T,
    ) -> std::result::Result<(), String> {
        value.serialize(self).map_err(|error| error.to_string())
    }

    fn collection(&mut self, len: usize) -> Result {
        self.entries += len;
        if self.entries > MAX_ENTRIES {
            return Err(fail(format!(
                "isometric: collection entries exceed {MAX_ENTRIES}"
            )));
        }
        Ok(())
    }
}

macro_rules! signed {
    ($($method:ident: $ty:ty),* $(,)?) => {$ (
        fn $method(self, value: $ty) -> Result {
            self.serialize_i64(i64::from(value))
        }
    )*};
}

macro_rules! unsigned {
    ($($method:ident: $ty:ty),* $(,)?) => {$ (
        fn $method(self, _value: $ty) -> Result { Ok(()) }
    )*};
}

impl Serializer for &mut Budget {
    type Ok = ();
    type Error = Error;
    type SerializeSeq = Self;
    type SerializeTuple = Self;
    type SerializeTupleStruct = Self;
    type SerializeTupleVariant = Self;
    type SerializeMap = Self;
    type SerializeStruct = Self;
    type SerializeStructVariant = Self;

    signed!(serialize_i8: i8, serialize_i16: i16, serialize_i32: i32);
    unsigned!(serialize_u8: u8, serialize_u16: u16, serialize_u32: u32, serialize_u64: u64);

    fn serialize_i64(self, value: i64) -> Result {
        if value < -(COORDINATE_LIMIT as i64) || value > COORDINATE_LIMIT as i64 {
            return Err(fail("isometric: excessive numeric value"));
        }
        Ok(())
    }

    fn serialize_f32(self, value: f32) -> Result {
        self.serialize_f64(f64::from(value))
    }

    fn serialize_f64(self, value: f64) -> Result {
        if !value.is_finite() || value.abs() > COORDINATE_LIMIT {
            return Err(fail("isometric: nonfinite or excessive numeric value"));
        }
        Ok(())
    }

    fn serialize_str(self, value: &str) -> Result {
        self.bytes += value.len();
        if self.bytes > MAX_BYTES {
            return Err(fail(format!("isometric: text exceeds {MAX_BYTES} bytes")));
        }
        Ok(())
    }

    fn serialize_char(self, value: char) -> Result {
        self.serialize_str(value.encode_utf8(&mut [0; 4]))
    }

    fn serialize_bool(self, _value: bool) -> Result {
        Ok(())
    }
    fn serialize_none(self) -> Result {
        Ok(())
    }
    fn serialize_unit(self) -> Result {
        Ok(())
    }
    fn serialize_unit_struct(self, _name: &'static str) -> Result {
        Ok(())
    }
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        value: &'static str,
    ) -> Result {
        self.serialize_str(value)
    }
    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result {
        value.serialize(self)
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        value: &T,
    ) -> Result {
        value.serialize(self)
    }
    fn serialize_bytes(self, value: &[u8]) -> Result {
        self.collection(value.len())
    }

    fn serialize_seq(self, len: Option<usize>) -> std::result::Result<Self, Error> {
        self.collection(len.ok_or_else(|| fail("isometric: unknown collection length"))?)?;
        Ok(self)
    }
    fn serialize_tuple(self, len: usize) -> std::result::Result<Self, Error> {
        self.collection(len)?;
        Ok(self)
    }
    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> std::result::Result<Self, Error> {
        self.serialize_tuple(len)
    }
    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        len: usize,
    ) -> std::result::Result<Self, Error> {
        self.serialize_tuple(len)
    }
    fn serialize_map(self, _len: Option<usize>) -> std::result::Result<Self, Error> {
        Ok(self)
    }
    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> std::result::Result<Self, Error> {
        Ok(self)
    }
    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> std::result::Result<Self, Error> {
        Ok(self)
    }
}

macro_rules! sequence {
    ($trait:ident, $method:ident $(, $unused:ident: $ty:ty)*) => {
        impl ser::$trait for &mut Budget {
            type Ok = ();
            type Error = Error;
            fn $method<T: Serialize + ?Sized>(&mut self, $($unused: $ty,)* value: &T) -> Result {
                value.serialize(&mut **self)
            }
            fn end(self) -> Result { Ok(()) }
        }
    };
}
sequence!(SerializeSeq, serialize_element);
sequence!(SerializeTuple, serialize_element);
sequence!(SerializeTupleStruct, serialize_field);
sequence!(SerializeTupleVariant, serialize_field);
sequence!(SerializeStruct, serialize_field, _key: &'static str);
sequence!(SerializeStructVariant, serialize_field, _key: &'static str);

impl ser::SerializeMap for &mut Budget {
    type Ok = ();
    type Error = Error;
    // serde's flattened target structs use map serialization; these are field
    // names rather than authored text, and cannot consume a source text budget.
    fn serialize_key<T: Serialize + ?Sized>(&mut self, _key: &T) -> Result {
        Ok(())
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result {
        value.serialize(&mut **self)
    }
    fn end(self) -> Result {
        Ok(())
    }
}

#[cfg(test)]
mod userinfo_tests {
    use super::*;
    use crate::target::{Connection, Legend, Shape, Url, Userinfo};

    const TEXT_ERROR: &str = "isometric: text exceeds 8388608 bytes";
    const INT_ERROR: &str = "isometric: excessive numeric value";
    const FLOAT_ERROR: &str = "isometric: nonfinite or excessive numeric value";

    fn icon() -> Option<Url> {
        Some(Url {
            user: Some(Userinfo::with_password("é", "pass")),
            ..Default::default()
        })
    }

    #[test]
    fn credentials_count_utf8_bytes_and_empty_password() {
        let mut budget = Budget::default();
        budget.icon(&icon()).unwrap();
        assert_eq!(budget.bytes, 6);
        for user in [Userinfo::new("x"), Userinfo::with_password("x", "")] {
            budget
                .icon(&Some(Url {
                    user: Some(user),
                    ..Default::default()
                }))
                .unwrap();
        }
        assert_eq!(budget.bytes, 8);
        assert_eq!(budget.entries, 0);
    }

    #[test]
    fn credentials_are_cumulative_across_root_legend_shapes_and_connections() {
        let mut budget = Budget {
            bytes: MAX_BYTES - 24,
            entries: 0,
        };
        let shape = Shape {
            icon: icon(),
            ..Default::default()
        };
        let connection = Connection {
            icon: icon(),
            ..Default::default()
        };
        budget.shape(&shape).unwrap();
        budget
            .legend(&Some(Legend {
                shapes: Some(vec![shape.clone()]),
                connections: Some(vec![connection.clone()]),
                ..Default::default()
            }))
            .unwrap();
        budget.shapes(&[shape]).unwrap();
        assert_eq!(budget.bytes, MAX_BYTES);
        assert_eq!(budget.connections(&[connection]).unwrap_err(), TEXT_ERROR);
        assert_eq!(budget.entries, 4);
    }

    #[test]
    fn shape_admission_preserves_errors_on_both_sides_of_icon() {
        let shape = Shape {
            icon: icon(),
            opacity: f64::NAN,
            icon_border_radius: 1_000_000_001,
            ..Default::default()
        };
        let mut budget = Budget {
            bytes: MAX_BYTES,
            entries: 0,
        };
        assert_eq!(budget.shape(&shape).unwrap_err(), FLOAT_ERROR);
        let shape = Shape {
            opacity: 1.0,
            ..shape
        };
        let mut budget = Budget {
            bytes: MAX_BYTES,
            entries: 0,
        };
        assert_eq!(budget.shape(&shape).unwrap_err(), TEXT_ERROR);
        let shape = Shape {
            icon: None,
            ..shape
        };
        assert_eq!(Budget::default().shape(&shape).unwrap_err(), INT_ERROR);
    }

    #[test]
    fn connection_admission_preserves_errors_on_both_sides_of_icon() {
        let connection = Connection {
            icon: icon(),
            border_radius: f64::INFINITY,
            icon_border_radius: f64::NAN,
            ..Default::default()
        };
        let mut budget = Budget {
            bytes: MAX_BYTES,
            entries: 0,
        };
        assert_eq!(budget.connection(&connection).unwrap_err(), FLOAT_ERROR);
        let connection = Connection {
            border_radius: 0.0,
            ..connection
        };
        let mut budget = Budget {
            bytes: MAX_BYTES,
            entries: 0,
        };
        assert_eq!(budget.connection(&connection).unwrap_err(), TEXT_ERROR);
        let connection = Connection {
            icon: None,
            ..connection
        };
        assert_eq!(
            Budget::default().connection(&connection).unwrap_err(),
            FLOAT_ERROR
        );
    }

    #[test]
    fn username_and_password_each_enforce_the_budget() {
        for user in [Userinfo::new("x"), Userinfo::with_password("", "x")] {
            let mut budget = Budget {
                bytes: MAX_BYTES,
                entries: 0,
            };
            assert_eq!(
                budget
                    .icon(&Some(Url {
                        user: Some(user),
                        ..Default::default()
                    }))
                    .unwrap_err(),
                TEXT_ERROR
            );
        }
    }

    #[test]
    fn collection_admission_precedes_credentials() {
        let mut budget = Budget {
            bytes: MAX_BYTES,
            entries: MAX_ENTRIES,
        };
        assert_eq!(
            budget
                .shapes(&[Shape {
                    icon: icon(),
                    ..Default::default()
                }])
                .unwrap_err(),
            "isometric: collection entries exceed 100000"
        );
    }

    #[test]
    fn icon_free_visitors_preserve_serializer_accounting() {
        let shape = Shape {
            id: "a".into(),
            classes: Some(vec!["one".into()]),
            fields: Some(vec![crate::target::ClassField {
                name: "field".into(),
                ..Default::default()
            }]),
            ..Default::default()
        };
        let connection = Connection {
            id: "edge".into(),
            src_label: Some(crate::target::Text {
                label: "caption".into(),
                ..Default::default()
            }),
            route: Some(vec![Some(crate::target::GeoPoint::default())]),
            ..Default::default()
        };
        let mut original = Budget::default();
        original.check(&shape).unwrap();
        original.check(&connection).unwrap();
        let mut explicit = Budget::default();
        explicit.shape(&shape).unwrap();
        explicit.connection(&connection).unwrap();
        assert_eq!(
            (original.bytes, original.entries),
            (explicit.bytes, explicit.entries)
        );
    }
}

// SPDX-License-Identifier: MPL-2.0
//! encoding/json v1 target semantics: chronological duplicate fields, null
//! scalars, pointer allocation/reset, slice reuse and untyped map values.
//! Source: pinned Go 1.27 encoding/json/{decode,fold,encode}.go.
use super::*;
use serde::{
    Deserializer,
    de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor},
};
use std::{fmt, ops::Deref};
pub(super) trait Merge: Default {
    fn merge<'de, D: Deserializer<'de>>(&mut self, d: D) -> Result<(), D::Error>;
}
struct Into<'a, T>(&'a mut T);
impl<'de, T: Merge> DeserializeSeed<'de> for Into<'_, T> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        self.0.merge(d)
    }
}
impl Merge for String {
    fn merge<'de, D: Deserializer<'de>>(&mut self, d: D) -> Result<(), D::Error> {
        if let Some(v) = Option::<String>::deserialize(d)? {
            *self = v;
        }
        Ok(())
    }
}
impl Merge for bool {
    fn merge<'de, D: Deserializer<'de>>(&mut self, d: D) -> Result<(), D::Error> {
        if let Some(v) = Option::<bool>::deserialize(d)? {
            *self = v;
        }
        Ok(())
    }
}
impl Merge for i64 {
    fn merge<'de, D: Deserializer<'de>>(&mut self, d: D) -> Result<(), D::Error> {
        // Retain numeric spelling: Go accepts integer -0 but rejects 0.0/1e0.
        let raw = Box::<serde_json::value::RawValue>::deserialize(d)?;
        if raw.get() != "null" {
            *self = raw.get().parse().map_err(serde::de::Error::custom)?;
        }
        Ok(())
    }
}
impl Merge for GoFloat {
    fn merge<'de, D: Deserializer<'de>>(&mut self, d: D) -> Result<(), D::Error> {
        if let Some(v) = Option::<GoFloat>::deserialize(d)? {
            *self = v;
        }
        Ok(())
    }
}
impl<T: Merge> Merge for Option<T> {
    fn merge<'de, D: Deserializer<'de>>(&mut self, d: D) -> Result<(), D::Error> {
        struct Pointer<'a, T>(&'a mut Option<T>);
        impl<'de, T: Merge> Visitor<'de> for Pointer<'_, T> {
            type Value = ();
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("target pointer")
            }
            fn visit_none<E: serde::de::Error>(self) -> Result<(), E> {
                *self.0 = None;
                Ok(())
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
                self.visit_none()
            }
            fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
                self.0.get_or_insert_with(T::default).merge(d)
            }
        }
        d.deserialize_option(Pointer(self))
    }
}
/// Go retains slice backing storage when a repeated array gets shorter.
/// Decoding later elements reuses those values, even past the old logical len.
/// Empty arrays and null explicitly retire that storage. Hidden tails never
/// participate in target iteration, serialization, hashing or painting.
#[derive(Clone, Debug)]
pub(super) struct GoSlice<T> {
    storage: Vec<T>,
    len: usize,
    nil: bool,
}
impl<T> From<Vec<T>> for GoSlice<T> {
    fn from(storage: Vec<T>) -> Self {
        Self {
            len: storage.len(),
            storage,
            nil: false,
        }
    }
}
impl<T> Default for GoSlice<T> {
    fn default() -> Self {
        Self {
            storage: Vec::new(),
            len: 0,
            nil: true,
        }
    }
}
impl<T> Deref for GoSlice<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.storage[..self.len]
    }
}
impl<'a, T> IntoIterator for &'a GoSlice<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
impl<T> GoSlice<T> {
    pub(super) fn is_empty(&self) -> bool {
        self.len == 0
    }
}
impl<T: Serialize> Serialize for GoSlice<T> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if self.nil {
            s.serialize_none()
        } else {
            self.deref().serialize(s)
        }
    }
}
impl<T: Merge> Merge for GoSlice<T> {
    fn merge<'de, D: Deserializer<'de>>(&mut self, d: D) -> Result<(), D::Error> {
        struct Slice<'a, T>(&'a mut GoSlice<T>);
        impl<'de, T: Merge> Visitor<'de> for Slice<'_, T> {
            type Value = ();
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("target slice")
            }
            fn visit_none<E: serde::de::Error>(self) -> Result<(), E> {
                *self.0 = GoSlice::default();
                Ok(())
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
                self.visit_none()
            }
            fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
                d.deserialize_seq(self)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
                let mut count = 0;
                loop {
                    if self.0.storage.len() == count {
                        self.0.storage.push(T::default());
                    }
                    if seq
                        .next_element_seed(Into(&mut self.0.storage[count]))?
                        .is_none()
                    {
                        break;
                    }
                    count += 1;
                }
                if count == 0 {
                    self.0.storage.clear();
                }
                self.0.len = count;
                self.0.nil = false;
                Ok(())
            }
        }
        d.deserialize_option(Slice(self))
    }
}
impl<'de, T: Merge> Deserialize<'de> for GoSlice<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut out = Self::default();
        out.merge(d)?;
        Ok(out)
    }
}

// Dynamic Config.Data uses float64 numbers and fresh interface{} values for
// each map entry. Outer maps merge; assigning an existing entry replaces it.
struct Dynamic(Value);
impl<'de> Deserialize<'de> for Dynamic {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Any;
        impl<'de> Visitor<'de> for Any {
            type Value = Dynamic;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("Go interface JSON value")
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Dynamic, E> {
                Ok(Dynamic(Value::Null))
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Dynamic, E> {
                Ok(Dynamic(Value::Bool(v)))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Dynamic, E> {
                self.visit_f64(v as f64)
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Dynamic, E> {
                self.visit_f64(v as f64)
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Dynamic, E> {
                Ok(Dynamic(Value::Number(
                    serde_json::Number::from_f64(v)
                        .ok_or_else(|| E::custom("nonfinite interface number"))?,
                )))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Dynamic, E> {
                Ok(Dynamic(Value::String(v.into())))
            }
            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Dynamic, E> {
                Ok(Dynamic(Value::String(v)))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Dynamic, A::Error> {
                let mut values = Vec::new();
                while let Some(Dynamic(v)) = seq.next_element()? {
                    values.push(v);
                }
                Ok(Dynamic(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Dynamic, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    let Dynamic(v) = map.next_value()?;
                    values.insert(key, v);
                }
                Ok(Dynamic(Value::Object(values)))
            }
        }
        d.deserialize_any(Any)
    }
}

impl Merge for BTreeMap<String, Value> {
    fn merge<'de, D: Deserializer<'de>>(&mut self, d: D) -> Result<(), D::Error> {
        struct Map<'a>(&'a mut BTreeMap<String, Value>);
        impl<'de> Visitor<'de> for Map<'_> {
            type Value = ();
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("Config.Data map")
            }
            fn visit_none<E: serde::de::Error>(self) -> Result<(), E> {
                self.0.clear();
                Ok(())
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
                self.visit_none()
            }
            fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
                d.deserialize_map(self)
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
                while let Some(key) = map.next_key::<String>()? {
                    self.0.insert(key, map.next_value::<Dynamic>()?.0);
                }
                Ok(())
            }
        }
        d.deserialize_option(Map(self))
    }
}
struct Data<'a>(&'a Value);
impl Serialize for Data<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            Value::Number(n) => GoFloat(n.as_f64().expect("JSON number")).serialize(s),
            Value::Array(v) => {
                use serde::ser::SerializeSeq;
                let mut out = s.serialize_seq(Some(v.len()))?;
                for item in v {
                    out.serialize_element(&Data(item))?;
                }
                out.end()
            }
            Value::Object(v) => {
                use serde::ser::SerializeMap;
                let sorted: BTreeMap<_, _> = v.iter().collect();
                let mut out = s.serialize_map(Some(sorted.len()))?;
                for (k, v) in sorted {
                    out.serialize_entry(k, &Data(v))?;
                }
                out.end()
            }
            v => v.serialize(s),
        }
    }
}
pub(super) fn serialize_data<S: serde::Serializer>(
    v: &BTreeMap<String, Value>,
    s: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeMap;
    let mut map = s.serialize_map(Some(v.len()))?;
    for (k, v) in v {
        map.serialize_entry(k, &Data(v))?;
    }
    map.end()
}

fn fold(key: &str) -> String {
    key.chars()
        .map(|c| match c {
            'ſ' => 's',
            'K' => 'k',
            c => c.to_ascii_lowercase(),
        })
        .collect()
}
macro_rules! merge_field {
    (normal,$map:ident,$slot:expr) => {
        $map.next_value_seed(Into(&mut $slot))?
    };
    (scalar_option,$map:ident,$slot:expr) => {
        if let Some(value) = $map.next_value::<Option<String>>()? {
            $slot = Some(value)
        }
    };
}
macro_rules! go_struct {
 ($ty:ty {$($key:literal => $field:ident : $kind:ident),*$(,)?})=>{
  impl Merge for $ty {
   fn merge<'de,D:Deserializer<'de>>(&mut self,d:D)->Result<(),D::Error>{
    struct Object<'a>(&'a mut $ty);
    impl<'de> Visitor<'de> for Object<'_>{
     type Value=();fn expecting(&self,f:&mut fmt::Formatter)->fmt::Result{f.write_str(stringify!($ty))}
     fn visit_unit<E:serde::de::Error>(self)->Result<(),E>{Ok(())}
     fn visit_map<A:MapAccess<'de>>(self,mut map:A)->Result<(),A::Error>{
      while let Some(key)=map.next_key::<String>()?{match fold(&key).as_str(){$($key=>{merge_field!($kind,map,self.0.$field);},)*_=>{map.next_value::<IgnoredAny>()?;}}}Ok(())
     }
    }
    d.deserialize_any(Object(self))
   }
  }
  impl<'de> Deserialize<'de> for $ty{fn deserialize<D:Deserializer<'de>>(d:D)->Result<Self,D::Error>{let mut out=Self::default();out.merge(d)?;Ok(out)}}
 }
}
// net/url.Userinfo has no exported fields, including after a nonempty JSON
// object is supplied. The nonnil pointer itself still contributes {} to hashes.
#[derive(Clone, Debug, Default, Serialize)]
pub(super) struct UrlUser {}
impl Merge for UrlUser {
    fn merge<'de, D: Deserializer<'de>>(&mut self, d: D) -> Result<(), D::Error> {
        struct Empty;
        impl<'de> Visitor<'de> for Empty {
            type Value = ();
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("url.Userinfo object")
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
                Ok(())
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
                while map.next_key::<IgnoredAny>()?.is_some() {
                    map.next_value::<IgnoredAny>()?;
                }
                Ok(())
            }
        }
        d.deserialize_any(Empty)
    }
}
impl<'de> Deserialize<'de> for UrlUser {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut out = Self::default();
        out.merge(d)?;
        Ok(out)
    }
}

// GENERATED FIELD TABLE below follows pinned d2target declarations. It is
// intentionally explicit: JSON field names never come from source D2 keys.
go_struct!(HashUrl{
    "scheme" => scheme: normal,
    "opaque" => opaque: normal,
    "user" => user: normal,
    "host" => host: normal,
    "path" => path: normal,
    "rawpath" => raw_path: normal,
    "omithost" => omit_host: normal,
    "forcequery" => force_query: normal,
    "rawquery" => raw_query: normal,
    "fragment" => fragment: normal,
    "rawfragment" => raw_fragment: normal,
});

go_struct!(HashIntPoint{
    "x" => x: normal,
    "y" => y: normal,
});

go_struct!(HashPoint{
    "x" => x: normal,
    "y" => y: normal,
});

go_struct!(TargetShape{
    "id" => id: normal,
    "type" => shape_type: normal,
    "classes" => classes: normal,
    "pos" => pos: normal,
    "width" => width: normal,
    "height" => height: normal,
    "opacity" => opacity: normal,
    "strokedash" => stroke_dash: normal,
    "strokewidth" => stroke_width: normal,
    "borderradius" => border_radius: normal,
    "fill" => fill: normal,
    "fillpattern" => fill_pattern: scalar_option,
    "stroke" => stroke: normal,
    "animated" => animated: normal,
    "shadow" => shadow: normal,
    "3d" => three_dee: normal,
    "multiple" => multiple: normal,
    "double-border" => double_border: normal,
    "tooltip" => tooltip: normal,
    "link" => link: normal,
    "prettylink" => pretty_link: scalar_option,
    "icon" => icon: normal,
    "iconborderradius" => icon_border_radius: normal,
    "iconposition" => icon_position: normal,
    "blend" => blend: normal,
    "fields" => fields: normal,
    "methods" => methods: normal,
    "columns" => columns: normal,
    "contentaspectratio" => content_aspect_ratio: normal,
    "label" => label: normal,
    "fontsize" => font_size: normal,
    "fontfamily" => font_family: normal,
    "language" => language: normal,
    "color" => color: normal,
    "italic" => italic: normal,
    "bold" => bold: normal,
    "underline" => underline: normal,
    "labelwidth" => label_width: normal,
    "labelheight" => label_height: normal,
    "labelfill" => label_fill: scalar_option,
    "labelposition" => label_position: scalar_option,
    "tooltipposition" => tooltip_position: scalar_option,
    "zindex" => z_index: normal,
    "level" => level: normal,
    "primaryaccentcolor" => primary_accent_color: normal,
    "secondaryaccentcolor" => secondary_accent_color: normal,
    "neutralaccentcolor" => neutral_accent_color: normal,
});

go_struct!(TargetConnection{
    "id" => id: normal,
    "classes" => classes: normal,
    "src" => src: normal,
    "srcarrow" => src_arrow: normal,
    "srclabel" => src_label: normal,
    "dst" => dst: normal,
    "dstarrow" => dst_arrow: normal,
    "dstlabel" => dst_label: normal,
    "opacity" => opacity: normal,
    "strokedash" => stroke_dash: normal,
    "strokewidth" => stroke_width: normal,
    "stroke" => stroke: normal,
    "fill" => fill: scalar_option,
    "borderradius" => border_radius: normal,
    "label" => label: normal,
    "fontsize" => font_size: normal,
    "fontfamily" => font_family: normal,
    "language" => language: normal,
    "color" => color: normal,
    "italic" => italic: normal,
    "bold" => bold: normal,
    "underline" => underline: normal,
    "labelwidth" => label_width: normal,
    "labelheight" => label_height: normal,
    "labelfill" => label_fill: normal,
    "labelposition" => label_position: normal,
    "labelpercentage" => label_percentage: normal,
    "link" => link: normal,
    "prettylink" => pretty_link: normal,
    "route" => route: normal,
    "iscurve" => is_curve: normal,
    "animated" => animated: normal,
    "tooltip" => tooltip: normal,
    "icon" => icon: normal,
    "iconposition" => icon_position: scalar_option,
    "iconborderradius" => icon_border_radius: normal,
    "zindex" => z_index: normal,
});

go_struct!(TargetText{
    "label" => label: normal,
    "fontsize" => font_size: normal,
    "fontfamily" => font_family: normal,
    "language" => language: normal,
    "color" => color: normal,
    "italic" => italic: normal,
    "bold" => bold: normal,
    "underline" => underline: normal,
    "labelwidth" => label_width: normal,
    "labelheight" => label_height: normal,
    "labelfill" => label_fill: normal,
});

go_struct!(TargetSqlColumn{
    "name" => name: normal,
    "type" => type_text: normal,
    "constraint" => constraint: normal,
    "reference" => reference: normal,
});

go_struct!(TargetClassField{
    "name" => name: normal,
    "type" => type_text: normal,
    "visibility" => visibility: normal,
    "underline" => underline: normal,
});

go_struct!(TargetClassMethod{
    "name" => name: normal,
    "return" => return_type: normal,
    "visibility" => visibility: normal,
    "underline" => underline: normal,
});

go_struct!(TargetConfig{
    "sketch" => sketch: normal,
    "isometric" => isometric: normal,
    "themeid" => theme_id: normal,
    "darkthemeid" => dark_theme_id: normal,
    "pad" => pad: normal,
    "center" => center: normal,
    "layoutengine" => layout_engine: normal,
    "themeoverrides" => theme_overrides: normal,
    "darkthemeoverrides" => dark_theme_overrides: normal,
    "data" => data: normal,
});

go_struct!(TargetThemeOverrides {
    "n1" => n1: normal,
    "n2" => n2: normal,
    "n3" => n3: normal,
    "n4" => n4: normal,
    "n5" => n5: normal,
    "n6" => n6: normal,
    "n7" => n7: normal,
    "b1" => b1: normal,
    "b2" => b2: normal,
    "b3" => b3: normal,
    "b4" => b4: normal,
    "b5" => b5: normal,
    "b6" => b6: normal,
    "aa2" => aa2: normal,
    "aa4" => aa4: normal,
    "aa5" => aa5: normal,
    "ab4" => ab4: normal,
    "ab5" => ab5: normal,
});

go_struct!(TargetDiagram{
    "name" => name: normal,
    "config" => config: normal,
    "isfolderonly" => is_folder_only: normal,
    "description" => description: normal,
    "fontfamily" => font_family: normal,
    "monofontfamily" => mono_font_family: normal,
    "shapes" => shapes: normal,
    "connections" => connections: normal,
    "root" => root: normal,
    "legend" => legend: normal,
    "layers" => layers: normal,
    "scenarios" => scenarios: normal,
    "steps" => steps: normal,
});

go_struct!(TargetLegend{
    "label" => label: normal,
    "shapes" => shapes: normal,
    "connections" => connections: normal,
});

/// Go replaces each invalid UTF-8 byte and each unpaired UTF-16 JSON escape
/// with U+FFFD. Keep valid surrogate pairs and literal escaped backslashes.
/// Syntax validation remains the JSON parser's job; this never repairs syntax.
pub(super) fn normalize_json(input: &[u8]) -> String {
    let mut text = String::new();
    let mut rest = input;
    while !rest.is_empty() {
        match std::str::from_utf8(rest) {
            Ok(valid) => {
                text.push_str(valid);
                break;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                text.push_str(std::str::from_utf8(&rest[..valid]).expect("valid UTF-8 prefix"));
                let count = error.error_len().unwrap_or(rest.len() - valid);
                for _ in 0..count {
                    text.push('\u{fffd}');
                }
                rest = &rest[valid + count..];
            }
        }
    }
    let bytes = text.as_bytes();
    let mut out = String::new();
    let mut at = 0;
    let mut quoted = false;
    fn code(bytes: &[u8], at: usize) -> Option<u16> {
        if bytes.get(at..at + 2)? != b"\\u" {
            return None;
        }
        let value = std::str::from_utf8(bytes.get(at + 2..at + 6)?).ok()?;
        u16::from_str_radix(value, 16).ok()
    }
    while at < bytes.len() {
        if bytes[at] == b'"' {
            quoted = !quoted;
            out.push('"');
            at += 1;
            continue;
        }
        if quoted && bytes[at] == b'\\' {
            if let Some(value) = code(bytes, at) {
                if (0xd800..=0xdbff).contains(&value) {
                    if code(bytes, at + 6).is_some_and(|next| (0xdc00..=0xdfff).contains(&next)) {
                        out.push_str(&text[at..at + 12]);
                        at += 12;
                        continue;
                    }
                    out.push_str("\\ufffd");
                    at += 6;
                    continue;
                }
                if (0xdc00..=0xdfff).contains(&value) {
                    out.push_str("\\ufffd");
                    at += 6;
                    continue;
                }
            }
            if bytes.get(at + 1).is_some_and(u8::is_ascii) {
                out.push_str(&text[at..at + 2]);
                at += 2;
                continue;
            }
        }
        let ch = text[at..].chars().next().expect("UTF-8 char");
        out.push(ch);
        at += ch.len_utf8();
    }
    out
}

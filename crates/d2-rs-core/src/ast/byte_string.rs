// SPDX-License-Identifier: MPL-2.0
//! Go string storage for productions that can yield invalid UTF-8.
use std::borrow::Cow;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ByteString(Vec<u8>);
impl ByteString {
    pub fn push(&mut self, rune: char) {
        self.0
            .extend_from_slice(rune.encode_utf8(&mut [0; 4]).as_bytes());
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
    /// Go JSON string encoding replaces each invalid UTF-8 byte separately.
    /// This presentation conversion never changes the stored bytes.
    pub fn to_text(&self) -> Cow<'_, str> {
        if let Ok(text) = std::str::from_utf8(&self.0) {
            return Cow::Borrowed(text);
        }
        let mut out = String::new();
        let mut bytes = self.0.as_slice();
        while !bytes.is_empty() {
            match std::str::from_utf8(bytes) {
                Ok(text) => {
                    out.push_str(text);
                    break;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    out.push_str(std::str::from_utf8(&bytes[..valid]).unwrap());
                    out.push('\u{fffd}');
                    bytes = &bytes[valid + 1..];
                }
            }
        }
        Cow::Owned(out)
    }
}
impl From<Vec<u8>> for ByteString {
    fn from(value: Vec<u8>) -> Self {
        Self(value)
    }
}
impl From<String> for ByteString {
    fn from(value: String) -> Self {
        Self(value.into_bytes())
    }
}
impl From<&str> for ByteString {
    fn from(value: &str) -> Self {
        Self(value.as_bytes().to_vec())
    }
}
impl serde::Serialize for ByteString {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_text())
    }
}

impl AsRef<[u8]> for ByteString {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl From<&String> for ByteString {
    fn from(value: &String) -> Self {
        Self(value.as_bytes().to_vec())
    }
}
impl From<&[u8]> for ByteString {
    fn from(value: &[u8]) -> Self {
        Self(value.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::*;
    #[test]
    fn scalar_equality_preserves_bytes_hidden_by_text_presentation() {
        let key = |bytes: Vec<u8>| Key {
            value: ValueBox {
                block_string: Some(Shared::new(BlockString {
                    value: bytes.into(),
                    ..Default::default()
                })),
                ..Default::default()
            },
            ..Default::default()
        };
        let a = key(vec![0xff]);
        let b = key(vec![0xfe]);
        assert_eq!(
            a.value.scalar_box().scalar_string(),
            b.value.scalar_box().scalar_string()
        );
        assert!(!a.equals(&b));
        assert_eq!(a.value.scalar_box().scalar_bytes(), [0xff]);
        assert_eq!(
            ByteString::from(vec![0xe2, 0x82]).to_text(),
            "\u{fffd}\u{fffd}"
        );
    }
}

// SPDX-License-Identifier: MPL-2.0
//! fontface error values. Display text equals the Go error string; wrapped
//! errors keep their cause so callers can test the typed root (errors.As).

use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// `FaceCountLimitError`.
    FaceCountLimit { count: usize, limit: usize },
    /// `COLRv1UntrustedFontError`.
    COLRv1UntrustedFont,
    /// `COLRv1LimitError`.
    COLRv1Limit {
        limit: &'static str,
        value: usize,
        maximum: usize,
    },
    /// `COLRv1UnsupportedPaintError`.
    COLRv1UnsupportedPaint { type_name: String },
    /// `COLRv1UnsupportedCompositeError`.
    COLRv1UnsupportedComposite { mode: u8 },
    /// `COLRv1UnsupportedExtendError`.
    COLRv1UnsupportedExtend { extend: u8 },
    /// An sfnt package error value.
    Sfnt(crate::sfnt::Error),
    /// A cancellation error reported by the caller's context.
    Canceled(String),
    /// `fmt.Errorf("<prefix>: %w", source)`.
    Wrapped { prefix: String, source: Box<Error> },
    /// Any other `fmt.Errorf`/`errors.New` text.
    Message(String),
}

impl Error {
    pub(crate) fn msg(text: impl Into<String>) -> Error {
        Error::Message(text.into())
    }

    pub(crate) fn wrap(prefix: impl Into<String>, source: impl Into<Error>) -> Error {
        Error::Wrapped {
            prefix: prefix.into(),
            source: Box::new(source.into()),
        }
    }

    /// The innermost error of a wrap chain (`errors.Unwrap` to the end).
    pub fn root(&self) -> &Error {
        let mut current = self;
        while let Error::Wrapped { source, .. } = current {
            current = source;
        }
        current
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::FaceCountLimit { count, limit } => {
                write!(
                    f,
                    "font collection face count {count} exceeds limit {limit}"
                )
            }
            Error::COLRv1UntrustedFont => {
                f.write_str("d2fonts: COLRv1 font is not the parser-authenticated bundled resource")
            }
            Error::COLRv1Limit {
                limit,
                value,
                maximum,
            } => write!(f, "d2fonts: COLRv1 {limit} {value} exceeds limit {maximum}"),
            Error::COLRv1UnsupportedPaint { type_name } => {
                write!(f, "d2fonts: unsupported COLRv1 paint {type_name}")
            }
            Error::COLRv1UnsupportedComposite { mode } => {
                write!(f, "d2fonts: unsupported COLRv1 composite mode {mode}")
            }
            Error::COLRv1UnsupportedExtend { extend } => {
                write!(f, "d2fonts: unsupported COLRv1 gradient extend {extend}")
            }
            Error::Sfnt(error) => error.fmt(f),
            Error::Canceled(text) | Error::Message(text) => f.write_str(text),
            Error::Wrapped { prefix, source } => write!(f, "{prefix}: {source}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<crate::sfnt::Error> for Error {
    fn from(error: crate::sfnt::Error) -> Error {
        Error::Sfnt(error)
    }
}

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Source-shaped invariant errors.

use std::error::Error;
use std::fmt;

/// Marker used by callers that need to distinguish invariant failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InvariantMarker;

/// An internal layout invariant violation with a human-readable reason.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InvariantError {
    reason: String,
}

impl InvariantError {
    /// Returns the stable marker for `Error::is`-style classification.
    pub(crate) const fn marker(&self) -> InvariantMarker {
        InvariantMarker
    }
}

impl fmt::Display for InvariantError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "layout invariant violated: {}", self.reason)
    }
}

impl Error for InvariantError {}

/// Creates an invariant violation with the supplied reason.
pub(crate) fn new(reason: impl Into<String>) -> InvariantError {
    InvariantError {
        reason: reason.into(),
    }
}

/// Formats an invariant violation.
pub(crate) fn errorf(arguments: fmt::Arguments<'_>) -> InvariantError {
    new(arguments.to_string())
}

#[cfg(test)]
mod tests {
    use super::{errorf, new};

    #[test]
    fn formats_and_classifies_invariant_errors() {
        let error = new("broken ownership");
        assert_eq!(
            error.to_string(),
            "layout invariant violated: broken ownership"
        );
        assert_eq!(error.marker(), super::InvariantMarker);

        let formatted = errorf(format_args!("node {} has no owner", 42));
        assert_eq!(
            formatted.to_string(),
            "layout invariant violated: node 42 has no owner"
        );
    }
}

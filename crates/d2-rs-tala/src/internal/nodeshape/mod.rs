// This Source Code Form is subject to the terms of the Mozilla
// Public License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Shape-specific policy used by TALA routing and labeling.
//!
//! The pinned Go implementation keeps snap-point inventories and shape policy
//! in `internal/nodeshape`. The Rust engine exposes the same policy to routing
//! through a narrow re-export so source comparisons can follow the upstream
//! package boundary without duplicating port state.

pub(super) mod ports;

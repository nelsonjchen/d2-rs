// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, you can obtain one at https://mozilla.org/MPL/2.0/.

//! Proximity relations used during TALA preprocessing and placement.
//!
//! The module follows v0.9.0's `internal/proximity` boundary. The helper
//! implementations retain the arena's stable IDs, while callers can continue
//! using the existing placement-facing methods during the migration.

mod herding;
mod hubs;

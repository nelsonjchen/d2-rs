// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Classification of violations of TALA's internal layout invariants.
//!
//! The package mirrors v0.9.0's `internal/invariant` boundary.  Keeping this
//! error separate from public graph validation lets diagnostic code identify a
//! broken internal transaction without changing the plugin error protocol.

mod errors;

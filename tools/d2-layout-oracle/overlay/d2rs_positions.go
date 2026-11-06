// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// Development reference only; never shipped as part of d2-rs.

// Overlay file for package d2layouts, added only to the d2-layout-oracle
// build (scripts/layout_oracle_fixtures.py), which also renames the
// original validateObjectPositions to d2rsValidateObjectPositions.

package d2layouts

import "github.com/d2lang/d2/d2graph"

// D2RSPositionObserver, when set, sees every object-position validation and
// its result.
var D2RSPositionObserver func(objects []*d2graph.Object, err error)

func validateObjectPositions(objects []*d2graph.Object) error {
	err := d2rsValidateObjectPositions(objects)
	if D2RSPositionObserver != nil {
		D2RSPositionObserver(objects, err)
	}
	return err
}

// D2RSValidateObjectPositions runs the original validation directly.
func D2RSValidateObjectPositions(objects []*d2graph.Object) error {
	return d2rsValidateObjectPositions(objects)
}

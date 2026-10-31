// SPDX-License-Identifier: MPL-2.0
// Observe outermost native tessellation operations on actual golden boards.
// Installed only in the pinned trace checkout; original bodies are unchanged.
package d2isometricimg

var d2rsPrimitiveDepth int

func d2rsPrimitiveBegin(b *meshBuilder, op string, args map[string]any, materials ...*Material) func() {
	if !d2rsPrimitivesEnabled || d2rsPrimitiveDepth != 0 {
		return func() {}
	}
	d2rsPrimitiveDepth++
	first := len(b.triangles)
	input := map[string]any{"op": op, "args": args, "prior": first, "error": ""}
	if b.err != nil {
		input["error"] = b.err.Error()
	}
	// Preserve nils and shared pointer identity in the argument list.
	ids := map[*Material]int{}
	var source []Triangle
	indices := []int{}
	for _, m := range materials {
		index := 0
		if m != nil {
			var ok bool
			index, ok = ids[m]
			if !ok {
				index = len(ids) + 1
				ids[m] = index
				source = append(source, Triangle{Material: m})
			}
		}
		indices = append(indices, index)
	}
	input["materialArgs"] = indices
	input["materials"] = d2rsMesh(&nativeScene{triangles: source}, false).Materials
	return func() {
		dump := d2rsMesh(&nativeScene{triangles: b.triangles[first:]}, false)
		err := ""
		if b.err != nil {
			err = b.err.Error()
		}
		input["want"] = map[string]any{"triangles": dump.Triangles, "materials": dump.Materials, "error": err}
		d2rsPrimitiveCalls = append(d2rsPrimitiveCalls, input)
		d2rsPrimitiveDepth--
	}
}

func (b *meshBuilder) box(center, size Vec, m *Material, r float64) {
	done := d2rsPrimitiveBegin(b, "box", map[string]any{"center": d2rsVec(center), "size": d2rsVec(size), "radius": r}, m)
	defer done()
	b.d2rsOriginalBox(center, size, m, r)
}

func (b *meshBuilder) sphere(center, radii Vec, m *Material, segments, rings int) {
	done := d2rsPrimitiveBegin(b, "sphere", map[string]any{"center": d2rsVec(center), "radii": d2rsVec(radii), "segments": segments, "rings": rings}, m)
	defer done()
	b.d2rsOriginalSphere(center, radii, m, segments, rings)
}

func (b *meshBuilder) cylinder(center Vec, rx, rz, height, topRatio float64, body, cap *Material, segments int, horizontal bool) {
	done := d2rsPrimitiveBegin(b, "cylinder", map[string]any{"center": d2rsVec(center), "rx": rx, "rz": rz, "height": height, "topRatio": topRatio, "segments": segments, "horizontal": horizontal}, body, cap)
	defer done()
	b.d2rsOriginalCylinder(center, rx, rz, height, topRatio, body, cap, segments, horizontal)
}

func (b *meshBuilder) flat(a, c, d Vec, m *Material, shadow bool) {
	done := d2rsPrimitiveBegin(b, "flat", map[string]any{"a": d2rsVec(a), "c": d2rsVec(c), "d": d2rsVec(d), "shadow": shadow}, m)
	defer done()
	b.d2rsOriginalFlat(a, c, d, m, shadow)
}

func (b *meshBuilder) extrudedProfile(p []Vec, base float64, top, side *Material) {
	points := [][3]float64{}
	for _, v := range p {
		points = append(points, d2rsVec(v))
	}
	done := d2rsPrimitiveBegin(b, "extrudedProfile", map[string]any{"points": points, "base": base}, top, side)
	defer done()
	b.d2rsOriginalExtrudedProfile(p, base, top, side)
}

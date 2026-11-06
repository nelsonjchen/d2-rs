"""Upstream D2 inputs read from the pinned tree instead of committed copies.

Until the second fixture pass these 27 e2e sources were copied verbatim into
tests/upstream/d2 (v0.9.0 bytes, identical at the pin). They are now read from
`e2etests/testdata/files` of the patched upstream tree a recipe runs against
($D2RS_UPSTREAM), or, outside the fixture runner, from the pristine checkout
named by reference-manifest.json (`references.verify_source()`).
"""
from __future__ import annotations

import os
from pathlib import Path

# The former tests/upstream/d2 inventory, by upstream e2e file name.
E2E_INPUTS = [
    "all_shapes_link.d2",
    "arrowhead_font_color.d2",
    "arrowhead_scaling.d2",
    "centered_horizontal_connections.d2",
    "cross_arrowhead.d2",
    "disconnect_direction_right.d2",
    "ent2d2_basic.d2",
    "grid_edge_across_cell.d2",
    "grid_icon.d2",
    "grid_label_positions.d2",
    "grid_nested.d2",
    "grid_nested_simple_edges.d2",
    "grid_outside_labels.d2",
    "grid_with_latex.d2",
    "icon_positions.d2",
    "icons_on_top.d2",
    "jupyter_k8s_oidc.d2",
    "label_positions.d2",
    "multiple_constant_nears.d2",
    "multiple_person_label.d2",
    "nested_diagram_types.d2",
    "nested_layout_bug.d2",
    "nested_shape_labels.d2",
    "nesting_power.d2",
    "outside_bottom_labels.d2",
    "outside_grid_label_position.d2",
    "overlapping_child_label.d2",
]

E2E_FILES = "e2etests/testdata/files"
# Logical prefix used in requests that name an input, so captures do not
# depend on where the upstream tree lives.
E2E_LABEL = "upstream/" + E2E_FILES


def upstream_root() -> Path:
    value = os.environ.get("D2RS_UPSTREAM")
    if value:
        return Path(value)
    from references import verify_source

    return verify_source()


def pristine_e2e(name: str = "") -> Path:
    """Path in the pristine checkout, computed without running anything (safe at import)."""
    import json

    root = Path(__file__).resolve().parents[1]
    manifest = json.loads((root / "reference-manifest.json").read_text())
    base = (root / manifest["d2"]["checkout"]).resolve() / E2E_FILES
    return base / name if name else base


def e2e_dir() -> Path:
    return upstream_root() / E2E_FILES


def e2e_input(name: str) -> Path:
    path = e2e_dir() / name
    if not path.is_file():
        raise SystemExit(f"missing upstream e2e input {path}")
    return path


def e2e_inputs() -> list[Path]:
    """The inventory in the sorted order the old tests/upstream/d2 glob used."""
    return [e2e_input(name) for name in sorted(E2E_INPUTS)]

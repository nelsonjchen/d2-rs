# Assets port to upstream 25e67b4c

This area targets the native SVG catalog export mismatch cluster and the watch
preview local/unavailable-image mismatch cluster, relative to old pin 6a7b4837.
The repository pin and regenerated oracle artifacts are integration work; this
area does not move the pin or run scoreboards.

## Included

- 8558856dc: linear-gradient defaults, object-bounding-box units, deferred
  percentages in each referencing viewport, pad/reflect/repeat spreading,
  omitted/percentage/clamped/nondecreasing stop offsets, empty and degenerate
  gradients, radial gradients (including focal circles and viewport-relative radii), definitions directly in graphics containers, SVG version 1.2,
  descriptions and optional Dublin Core metadata, comma-separated simple class
  selectors, ASCII whitespace in embedded base64, separate decoded-raster
  ceilings, native export import ceilings, and image shapes with `multiple`.
- 8558856dc (continued): inert editor text/solid-color attributes on geometry,
  local path metadata hrefs, enable-background on graphics containers, isolation,
  normal blending, vector-effect=none, visible overflow on use/image, and
  unreferenced clip paths without IDs.
- b43d0c44f: data-only image resolver API classifies local paths and enforces
  denied local-file policy before rejecting external loading.
- 6012a84bd and 1c7d0204e: raster watch previews use acquired resource snapshots
  and unavailable-source markers, unescape hrefs, enforce preview bounds, and
  share the native placeholder SVG. A preview never retries image acquisition.

## Remaining from 8558856dc

This is a partial port of the catalog commit. The following output-affecting
work is still required:

- SVG mask regions/content coordinate systems, chained clipping paths, and
  single-primitive Gaussian filters, including linear-light interpolation.
- Positioned text/tspan shaping with bundled fonts and shared glyph outlines.
- Ambiguous duplicate IDs and hidden external-image sentinels.


Unsupported SVG forms retain the previous importer rejection behavior; they
are not silently approximated. Complex even-odd fills and clips now use the
upstream bounded tile algorithm and revised work accounting. The opt-in hosted catalog sweep and full PNG/GIF/PDF/PPTX
pixel coverage have not been ported. The new default-tier contracts cover
linear-gradient geometry and PNG/GIF pixels, image `multiple`, metadata/classes,
embedded-raster limits, data-only policy, and preview snapshots/placeholders.

CI/release material and JS/d2js packaging are intentionally outside this area.

// SPDX-License-Identifier: MPL-2.0
//! d2cli/isometric_paged.go: RenderPage feeds the shared PDF/PPTX writers.
use super::*;
use d2_rs_core::{Point, Rect};
use d2_rs_export::{DocumentLinkMetadata, DocumentLinkRegion};

const MAX_PIXELS: i64 = 256 * 1024 * 1024;
const MAX_ENCODED: usize = 512 * 1024 * 1024;
const MAX_LINKS: usize = 65536;
const MAX_LINK_BYTES: usize = 64 * 1024 * 1024;

#[cfg(test)]
#[path = "isometric_paged_api_tests.rs"]
mod api_tests;

// d2cli/isometric_paged.go: isometricDocumentWriter. The native packagers
// return an owned document, so admit_owned transfers that allocation instead
// of copying it into a second bytes.Buffer. Both entry points share admission.
struct DocumentWriter<W> {
    output: W,
    remaining: usize,
}

impl<W> DocumentWriter<W> {
    fn admit(&self, length: usize) -> std::io::Result<()> {
        check_context().map_err(std::io::Error::other)?;
        if length > self.remaining {
            return Err(std::io::Error::other(format!(
                "isometric paged document exceeds the {MAX_ENCODED}-byte output limit"
            )));
        }
        Ok(())
    }
}

impl DocumentWriter<Vec<u8>> {
    fn admit_owned(mut self, bytes: Vec<u8>) -> std::io::Result<Vec<u8>> {
        self.admit(bytes.len())?;
        self.remaining -= bytes.len();
        debug_assert!(self.output.is_empty());
        self.output = bytes;
        Ok(self.output)
    }
}

impl<W: std::io::Write> std::io::Write for DocumentWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.admit(bytes.len())?;
        let count = self.output.write(bytes)?;
        self.remaining -= count;
        if count != bytes.len() {
            return Err(std::io::Error::other("short write"));
        }
        Ok(count)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.output.flush()
    }
}

struct Budget {
    pixels: i64,
    encoded: usize,
    links: usize,
    link_bytes: usize,
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            pixels: MAX_PIXELS,
            encoded: MAX_ENCODED,
            links: MAX_LINKS,
            link_bytes: MAX_LINK_BYTES,
        }
    }
}
impl Budget {
    fn admit(&mut self, page: &img::page::Page, width: i64, height: i64) -> Result<(), String> {
        if page.width <= 0 || page.height <= 0 || page.width > width || page.height > height {
            return Err("isometric page has invalid image bounds".into());
        }
        let pixels = page.width * page.height;
        if pixels > self.pixels {
            return Err("isometric paged export exhausted its pixel budget".into());
        }
        if page.png.is_empty() || page.png.len() > self.encoded {
            return Err("isometric paged export exhausted its encoded image budget".into());
        }
        if page.links.len() > self.links {
            return Err("isometric paged export exhausted its link-region budget".into());
        }
        let mut bytes = 0;
        for (i, link) in page.links.iter().enumerate() {
            if i & 255 == 0 {
                check_context()?;
            }
            let b = link.r#box;
            if [b.x, b.y, b.width, b.height].iter().any(|v| !v.is_finite()) {
                return Err("isometric page link has non-finite coordinates".into());
            }
            if b.x < 0.0
                || b.y < 0.0
                || b.width <= 0.0
                || b.height <= 0.0
                || b.x + b.width > page.width as f64 + 1e-6
                || b.y + b.height > page.height as f64 + 1e-6
            {
                return Err("isometric page link lies outside the rendered image".into());
            }
            for s in [&link.url, &link.target, &link.tooltip] {
                if s.len() > self.link_bytes - bytes {
                    return Err("isometric paged export exhausted its link-string budget".into());
                }
                bytes += s.len();
            }
        }
        self.pixels -= pixels;
        self.encoded -= page.png.len();
        self.links -= page.links.len();
        self.link_bytes -= bytes;
        Ok(())
    }
}

fn links(
    regions: Vec<d2_rs_scene::LinkRegion>,
    selected: &str,
    pages: &BTreeSet<String>,
    format: ExportFormat,
) -> Result<Vec<DocumentLinkMetadata>, String> {
    let source_root = if selected.is_empty() {
        "root".to_owned()
    } else {
        format!("root.{selected}")
    };
    let mut result = Vec::new();
    for (i, mut r) in regions.into_iter().enumerate() {
        if i & 255 == 0 {
            check_context()?;
        }
        if !selected.is_empty() {
            if !r.target.is_empty() {
                if r.target != source_root && !r.target.starts_with(&format!("{source_root}.")) {
                    return Err(format!(
                        "isometric page link target {:?} lies outside selected board {:?}",
                        r.target, source_root
                    ));
                }
                r.target = format!("root{}", &r.target[source_root.len()..]);
            } else if r.url == source_root || r.url.starts_with(&format!("{source_root}.")) {
                let local = format!("root{}", &r.url[source_root.len()..]);
                if pages.contains(&local) {
                    r.target = local;
                    r.url.clear();
                }
            }
        }
        let typed = !r.target.is_empty();
        let href = if typed { r.target } else { r.url };
        if typed && !pages.contains(&href) {
            let (kind, unit) = if format == ExportFormat::Pdf {
                ("PDF", "page")
            } else {
                ("PPTX", "slide")
            };
            return Err(format!(
                "{kind} link {i} target {href:?} has no renderable {unit}"
            ));
        }
        if href.is_empty() {
            continue;
        }
        // The shared packagers consume logical coordinates (PNG pixels / 2).
        // PDF rounds outward after division; PPTX rounds in source pixels.
        let b = r.r#box;
        let (left, top, right, bottom) = if format == ExportFormat::Pdf {
            (
                (b.x / 2.0).floor(),
                (b.y / 2.0).floor(),
                ((b.x + b.width) / 2.0).ceil(),
                ((b.y + b.height) / 2.0).ceil(),
            )
        } else {
            (
                b.x / 2.0,
                b.y / 2.0,
                (b.x + b.width) / 2.0,
                (b.y + b.height) / 2.0,
            )
        };
        result.push(DocumentLinkMetadata {
            region: DocumentLinkRegion {
                href,
                rect: Rect {
                    origin: Point { x: left, y: top },
                    size: Size {
                        width: right - left,
                        height: bottom - top,
                    },
                },
            },
            tooltip: r.tooltip,
            board_target: typed,
        });
    }
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    input: &[u8],
    input_path: &str,
    output: &Path,
    format: ExportFormat,
    selected: &str,
    targets: &[String],
    all_targets: &[String],
    renderable: &BTreeSet<String>,
    layout: &str,
    seeds: Option<&str>,
    pad: Option<f64>,
    scale: Option<f64>,
    theme_id: Option<i64>,
    center: Option<bool>,
    adapter_args: &[String],
    started: std::time::Instant,
) -> Result<Vec<u8>, String> {
    if targets.len() > 1024 {
        return Err("paged board count exceeds 1024".into());
    }
    let visited = 1 + all_targets
        .iter()
        .filter(|t| selected.is_empty() || t.starts_with(&format!("{selected}.")))
        .count();
    if visited > 4096 {
        return Err("paged board tree exceeds 4096 total nodes".into());
    }
    let (width, height) = dimensions(format, scale.unwrap_or(-1.0))?;
    if width * height * targets.len() as i64 > MAX_PIXELS {
        return Err(format!(
            "isometric paged export exceeds the {MAX_PIXELS}-pixel operation limit; reduce --scale or select fewer boards"
        ));
    }
    let mut title_targets = targets.to_vec();
    if !selected.is_empty() {
        title_targets.push(selected.to_owned());
    }
    let mut titles = source_board_titles(input, input_path, &title_targets)?;
    if !selected.is_empty() {
        titles.root_label = titles
            .board_labels
            .get(selected)
            .cloned()
            .flatten()
            .or_else(|| compiled_board_components(selected).pop())
            .filter(|s| !s.is_empty());
    }
    let root_renderable = targets.iter().any(|t| t == selected);
    let board_paths = targets
        .iter()
        .map(|t| {
            let mut path = if root_renderable {
                vec!["root".into()]
            } else {
                Vec::new()
            };
            path.extend(
                rendered_board_ancestors(t, renderable)
                    .into_iter()
                    .filter(|(id, _)| {
                        selected.is_empty() || id.starts_with(&format!("{selected}."))
                    })
                    .map(|(id, _)| paged_board_id(&id, selected)),
            );
            path
        })
        .collect::<Vec<_>>();
    let title_paths = targets
        .iter()
        .map(|t| {
            let mut path = if root_renderable {
                vec![if format == ExportFormat::Pptx {
                    "root".into()
                } else {
                    titles.root_label.clone().unwrap_or_else(|| "root".into())
                }]
            } else {
                Vec::new()
            };
            for (id, name) in rendered_board_ancestors(t, renderable) {
                if !selected.is_empty() && !id.starts_with(&format!("{selected}.")) {
                    continue;
                }
                path.push(if format == ExportFormat::Pdf {
                    titles
                        .board_labels
                        .get(&id)
                        .cloned()
                        .flatten()
                        .unwrap_or(name)
                } else {
                    name
                });
            }
            path
        })
        .collect::<Vec<_>>();
    let page_ids = board_paths
        .iter()
        .filter_map(|p| p.last().cloned())
        .collect::<BTreeSet<_>>();
    let mut base = source_path_args(input_path);
    base.push(format!("--engine={layout}"));
    for (name, value) in [
        ("seeds", seeds.map(str::to_owned)),
        ("pad", pad.map(|v| v.to_string())),
        ("scale", scale.map(|v| v.to_string())),
        ("theme", theme_id.map(|v| v.to_string())),
        ("center", center.map(|v| v.to_string())),
    ] {
        if let Some(v) = value {
            base.push(format!("--{name}={v}"));
        }
    }
    base.extend(adapter_args.iter().cloned());
    let (mut assets, mut fonts) = resources(input_path)?;
    let count = targets.len() as i64;
    let b = &mut assets.svg_import_budget;
    for v in [
        &mut b.max_source_bytes,
        &mut b.max_elements,
        &mut b.max_attributes,
        &mut b.max_attribute_bytes,
        &mut b.max_path_commands,
        &mut b.max_transform_functions,
        &mut b.max_declared_resources,
        &mut b.max_expanded_use_instances,
    ] {
        *v = (*v / count).max(1);
    }
    for v in [
        &mut fonts.max_total_runes,
        &mut fonts.max_coverage_checks,
        &mut fonts.max_shaping_runs,
        &mut fonts.max_shaped_glyphs,
    ] {
        *v = (*v / count).max(1);
    }
    let mut budget = Budget::default();
    let (mut images, mut annotations, mut fills) = (Vec::new(), Vec::new(), Vec::new());
    let mut document_theme = theme_id;
    for (i, target) in targets.iter().enumerate() {
        check_context()?;
        let mut args = base.clone();
        args.push(format!("--target={target}"));
        let frame = render_source_target_animation(input, &args)?;
        let value = source_diagram_json(&frame, target.is_empty(), false)?;
        let diagram: Diagram = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
        document_theme = document_theme.or(frame.render_options.theme_id);
        let options = img::render::Options {
            render: d2_rs_isometric::scene::RenderOpts {
                theme_id: frame.render_options.theme_id,
                theme_overrides: diagram
                    .config
                    .as_ref()
                    .and_then(|c| c.theme_overrides.clone()),
            },
            width,
            height,
            fit_content: true,
            assets: Some(assets.clone()),
            fonts: Some(fonts.clone()),
            link_budget: Some(foundation::LinkBudget {
                max_regions: 4096.min((MAX_LINKS as i64 / count).max(1)),
                max_string_bytes: (1 << 20).min((MAX_LINK_BYTES as i64 / count).max(1)),
            }),
            ..Default::default()
        };
        let page = img::page::render_page(Some(&diagram), Some(&options))
            .map_err(|e| format!("isometric page {}: {e}", i + 1))?;
        budget.admit(&page, width, height)?;
        annotations.push(links(page.links, selected, &page_ids, format)?);
        images.push(page.png);
        if format == ExportFormat::Pdf {
            let fill = value["root"]["fill"].as_str().unwrap_or_default();
            let rgb = if fill.is_empty() || fill.eq_ignore_ascii_case("transparent") {
                [255; 3]
            } else {
                let palette = theme::palette(document_theme.unwrap_or(0))
                    .or_else(|| theme::palette(0))
                    .unwrap();
                let resolved = theme::resolve_color(palette, fill).unwrap_or(fill);
                let c = d2_rs_math::svg_color::solid(resolved)?;
                [c[0], c[1], c[2]]
            };
            fills.push(rgb);
        }
    }
    check_context()?;
    let encoded = if format == ExportFormat::Pdf {
        d2_rs_export::package_pdf_pages_with_navigation_metadata_for_theme(
            &images,
            &annotations,
            &board_paths,
            &title_paths,
            titles.root_label.is_some(),
            document_theme,
            &fills,
        )
    } else {
        let mut properties = pptx_document_properties(input_path, Some(&output.to_string_lossy()));
        // isometric_paged.go leaves creator empty, unlike the ordinary CLI path.
        properties.creator.clear();
        d2_rs_export::package_pptx_slides_with_document_properties(
            &images,
            &board_paths,
            &title_paths,
            &annotations,
            titles.root_label.is_some(),
            &properties,
        )
    }
    .map_err(|e| match e {
        d2_rs_export::ExportError::Pdf(s) | d2_rs_export::ExportError::Pptx(s) => s,
        other => other.to_string(),
    })?;
    let encoded = DocumentWriter {
        output: Vec::new(),
        remaining: MAX_ENCODED,
    }
    .admit_owned(encoded)
    .map_err(|error| error.to_string())?;
    if output.as_os_str() != "-" {
        atomic_image(output, &encoded)?;
    }
    log_compile_success(Path::new(input_path), output, started.elapsed());
    Ok(if output.as_os_str() == "-" {
        encoded
    } else {
        Vec::new()
    })
}

#[cfg(test)]
#[path = "isometric_cli/paged_admission_api_tests.rs"]
mod admission_api_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use d2_rs_scene::{Box as LinkBox, LinkRegion};

    fn link() -> LinkRegion {
        LinkRegion {
            r#box: LinkBox {
                x: 1.25,
                y: 3.75,
                width: 10.5,
                height: 8.5,
            },
            target: "root.layers.parent.layers.child".into(),
            tooltip: "Child".into(),
            ..Default::default()
        }
    }

    #[test]
    fn selected_page_links_keep_provenance_and_document_units() {
        let pages = BTreeSet::from(["root".into(), "root.layers.child".into()]);
        let mut relative = link();
        relative.target.clear();
        relative.url = "root.html".into();
        for format in [ExportFormat::Pdf, ExportFormat::Pptx] {
            let result = links(
                vec![link(), relative.clone()],
                "layers.parent",
                &pages,
                format,
            )
            .unwrap();
            assert_eq!(result[0].href, "root.layers.child");
            assert!(result[0].board_target);
            assert_eq!(result[0].tooltip, "Child");
            assert_eq!(result[1].href, "root.html");
            assert!(!result[1].board_target);
            assert_eq!(
                result[0].rect.origin.x,
                if format == ExportFormat::Pdf {
                    0.0
                } else {
                    0.625
                }
            );
            assert_eq!(
                result[0].rect.size.height,
                if format == ExportFormat::Pdf {
                    6.0
                } else {
                    4.25
                }
            );
        }
        let mut outside = link();
        outside.target = "root".into();
        assert!(
            links(vec![outside], "layers.parent", &pages, ExportFormat::Pdf)
                .unwrap_err()
                .contains("lies outside selected board")
        );
        assert_eq!(
            links(vec![link()], "", &pages, ExportFormat::Pptx).unwrap_err(),
            "PPTX link 0 target \"root.layers.parent.layers.child\" has no renderable slide"
        );
    }

    #[test]
    fn page_operation_budgets_are_cumulative() {
        let page = img::page::Page {
            png: b"encoded".to_vec(),
            width: 100,
            height: 80,
            links: vec![link()],
        };
        let mut budget = Budget {
            pixels: 16000,
            encoded: 14,
            links: 2,
            link_bytes: 200,
        };
        budget.admit(&page, 100, 100).unwrap();
        budget.admit(&page, 100, 100).unwrap();
        assert_eq!(
            budget.admit(&page, 100, 100).unwrap_err(),
            "isometric paged export exhausted its pixel budget"
        );
        for field in 0..4 {
            let mut budget = Budget::default();
            match field {
                0 => budget.pixels = 1,
                1 => budget.encoded = 1,
                2 => budget.links = 0,
                _ => budget.link_bytes = 1,
            };
            assert!(budget.admit(&page, 100, 100).is_err());
        }
        let mut bad = page;
        bad.links[0].r#box.x = f64::NAN;
        assert_eq!(
            Budget::default().admit(&bad, 100, 100).unwrap_err(),
            "isometric page link has non-finite coordinates"
        );
    }
}

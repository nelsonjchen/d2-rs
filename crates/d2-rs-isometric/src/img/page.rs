// SPDX-License-Identifier: MPL-2.0
//! d2isometricimg page.go: bounded link and tooltip regions on the mesh.

use d2_rs_core::stable_sort::StableSort;
use d2_rs_render::go_float::{go_fmadd, go_fmsub, gomath};
use d2_rs_scene::{Document, ViewportAlign, ViewportFit, go_max, go_min};

use super::framing::{ProjectedExtent, native_camera_axes};
use super::geometry::{LabelSurface, MeshBuilder};
use super::mesh::{Vec, nadd, nmul, nv};
use super::route_labels::caption_finite;
use super::svg::NativeLink;

/// d2scene.LinkRegion without its box.
#[derive(Clone, Debug, Default)]
pub struct LinkDestination {
    pub url: String,
    pub tooltip: String,
    pub target: String,
}

/// Match the native PDF/PPTX metadata contract (XML 1.0 characters).
pub fn validate_page_link_text(value: &str) -> Result<(), String> {
    for r in value.chars() {
        let c = r as u32;
        if !(r == '\t'
            || r == '\n'
            || r == '\r'
            || (0x20..=0xd7ff).contains(&c)
            || (0xe000..=0xfffd).contains(&c)
            || (0x10000..=0x10ffff).contains(&c))
        {
            return Err(
                "isometric page link metadata contains a character forbidden by XML 1.0".into(),
            );
        }
    }
    Ok(())
}

pub fn validate_native_link(link: &str) -> Result<(), String> {
    if d2_rs_scenebuild::is_dangerous_link(link) {
        return Err("isometric link uses an unsafe URL scheme".into());
    }
    Ok(())
}

pub fn native_link_destination(link: &str, tooltip: &str) -> LinkDestination {
    let r = LinkDestination {
        url: link.into(),
        tooltip: tooltip.into(),
        target: String::new(),
    };
    let Some(path) = d2_rs_language::parse_key(link.as_bytes())
        .ok()
        .and_then(|key| key.string_ida())
    else {
        return r;
    };
    if path.is_empty() || path.len() % 2 == 0 || path[0] != "root" {
        return r;
    }
    // Semantic board targets alternate a board collection and its name.
    for kind in path.iter().skip(1).step_by(2) {
        if !matches!(kind.as_str(), "layers" | "scenarios" | "steps") {
            return r;
        }
    }
    LinkDestination {
        url: String::new(),
        tooltip: tooltip.into(),
        target: link.into(),
    }
}

pub fn surface_corners(s: &LabelSurface) -> [Vec; 4] {
    let u = nv(gomath::cos(s.angle), 0.0, gomath::sin(s.angle));
    let v = nv(-gomath::sin(s.angle), 0.0, gomath::cos(s.angle));
    let point = |x: f64, y: f64| nadd(s.center, nadd(nmul(u, x * s.width), nmul(v, y * s.depth)));
    [
        point(-0.5, -0.5),
        point(0.5, -0.5),
        point(0.5, 0.5),
        point(-0.5, 0.5),
    ]
}

impl MeshBuilder {
    pub fn add_native_link(&mut self, points: [Vec; 4], region: LinkDestination) {
        let Some(budget) = self.options.links else {
            return;
        };
        if self.err.is_some()
            || region.url.is_empty() && region.target.is_empty() && region.tooltip.is_empty()
        {
            return;
        }
        if self.links.len() as i64 >= budget.max_regions {
            self.fail(format!(
                "isometric page link regions exceed {}",
                budget.max_regions
            ));
            return;
        }
        let bytes = region.url.len() + region.target.len() + region.tooltip.len();
        if bytes as i64 > budget.max_string_bytes - self.link_bytes as i64 {
            self.fail(format!(
                "isometric page link metadata exceeds {} bytes",
                budget.max_string_bytes
            ));
            return;
        }
        for v in [&region.url, &region.target, &region.tooltip] {
            if let Err(error) = validate_page_link_text(v) {
                self.fail(error);
                return;
            }
        }
        if let Err(error) = validate_native_link(&region.url) {
            self.fail(error);
            return;
        }
        if points.iter().any(|p| !caption_finite(&[p.x, p.y, p.z])) {
            self.fail("isometric page link bounds are non-finite");
            return;
        }
        self.link_bytes += bytes;
        self.links.push(NativeLink {
            points: points.to_vec(),
            url: region.url,
            tooltip: region.tooltip,
            target: region.target,
        });
    }

    pub fn validate_typed_link(&mut self, link: &str, tooltip: &str) -> bool {
        if self.options.vector {
            return true; // SVG preserves arbitrary source tooltip text in a title.
        }
        if !link.is_empty()
            && !tooltip.is_empty()
            && d2_rs_scenebuild::parse_request_uri_has_host(tooltip)
        {
            self.fail("isometric page tooltip must not be a URL when link is also set");
            return false;
        }
        true
    }

    pub fn add_surface_link(&mut self, link: &str, tooltip: &str, s: &LabelSurface) {
        if self.options.links.is_none()
            || link.is_empty() && tooltip.is_empty()
            || s.width <= 0.0
            || s.depth <= 0.0
        {
            return;
        }
        if !self.validate_typed_link(link, tooltip) {
            return;
        }
        self.add_native_link(surface_corners(s), native_link_destination(link, tooltip));
    }

    /// `first` starts the component's triangles (Go passes a subslice).
    pub fn add_mesh_link(&mut self, link: &str, tooltip: &str, first: usize) {
        if self.options.links.is_none()
            || link.is_empty() && tooltip.is_empty()
            || first >= self.triangles.len()
        {
            return;
        }
        if !self.validate_typed_link(link, tooltip) {
            return;
        }
        let mut e = ProjectedExtent::default();
        e.mesh(&self.triangles[first..]);
        let c = native_camera_axes();
        let point = |x: f64, y: f64| {
            nadd(
                nmul(c.right, x),
                nadd(nmul(c.up, y), nmul(c.direction, e.max_z)),
            )
        };
        self.add_native_link(
            [
                point(e.min_x, e.min_y),
                point(e.max_x, e.min_y),
                point(e.max_x, e.max_y),
                point(e.min_x, e.max_y),
            ],
            native_link_destination(link, tooltip),
        );
    }

    /// Map source-document link boxes through the same meet/stretch viewport
    /// used for its texture, then through the surface's orientation.
    pub fn add_document_links(&mut self, doc: Option<&Document>, s: &LabelSurface) {
        let Some(doc) = doc else {
            return;
        };
        let links = doc.links.as_deref().unwrap_or_default();
        if self.options.links.is_none() || links.is_empty() || self.err.is_some() {
            return;
        }
        let (vw, vh) = (doc.view_box.width, doc.view_box.height);
        if !caption_finite(&[vw, vh, s.width, s.depth])
            || vw <= 0.0
            || vh <= 0.0
            || s.width <= 0.0
            || s.depth <= 0.0
        {
            self.fail("isometric page document link viewport is invalid");
            return;
        }
        let (mut scale_x, mut scale_y) = (s.width / vw, s.depth / vh);
        let (mut ox, mut oy) = (0.0, 0.0);
        if doc.viewport_fit == ViewportFit::MEET {
            let (mut pw, mut ph) = (doc.logical_width, doc.logical_height);
            if pw <= 0.0 || ph <= 0.0 {
                (pw, ph) = (s.width, s.depth);
            }
            let uniform = go_min(pw / vw, ph / vh);
            scale_x = uniform * s.width / pw;
            scale_y = uniform * s.depth / ph;
            if doc.viewport_align == ViewportAlign::X_MID_Y_MID {
                // ARM64 page.go:266.
                ox = go_fmsub(vw, scale_x, s.width) / 2.0;
                oy = go_fmsub(vh, scale_y, s.depth) / 2.0;
            }
        }
        let u = nv(gomath::cos(s.angle), 0.0, gomath::sin(s.angle));
        let v = nv(-gomath::sin(s.angle), 0.0, gomath::cos(s.angle));
        let point = |x: f64, y: f64| {
            nadd(
                s.center,
                nadd(nmul(u, x - s.width / 2.0), nmul(v, y - s.depth / 2.0)),
            )
        };
        for r in links {
            // ARM64 page.go:272–273.
            let x0 = go_fmadd(r.r#box.x - doc.view_box.x, scale_x, ox);
            let y0 = go_fmadd(r.r#box.y - doc.view_box.y, scale_y, oy);
            let x1 = go_fmadd(r.r#box.width, scale_x, x0);
            let y1 = go_fmadd(r.r#box.height, scale_y, y0);
            let (x0, y0, x1, y1) = (
                go_max(0.0, x0),
                go_max(0.0, y0),
                go_min(s.width, x1),
                go_min(s.depth, y1),
            );
            if x1 <= x0 || y1 <= y0 {
                continue;
            }
            let region = LinkDestination {
                url: r.url.clone(),
                tooltip: r.tooltip.clone(),
                target: r.target.clone(),
            };
            self.add_native_link(
                [point(x0, y0), point(x1, y0), point(x1, y1), point(x0, y1)],
                region,
            );
            if self.err.is_some() {
                return;
            }
        }
    }
}

/// Native image and annotations in final image pixels (page.go:21–26).
#[derive(Debug)]
pub struct Page {
    pub png: std::vec::Vec<u8>,
    pub links: std::vec::Vec<d2_rs_scene::LinkRegion>,
    pub width: i64,
    pub height: i64,
}

/// Preserve cancellation causes for callers; messages retain Go's public text.
#[derive(Debug)]
pub enum PageError {
    Context(d2_rs_core::LayoutError),
    StageContext {
        message: String,
        cause: d2_rs_core::LayoutError,
    },
    Message(String),
}
impl std::fmt::Display for PageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Context(e) => e.fmt(f),
            Self::StageContext { message, .. } | Self::Message(message) => message.fmt(f),
        }
    }
}
impl std::error::Error for PageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Context(e) | Self::StageContext { cause: e, .. } => Some(e),
            Self::Message(_) => None,
        }
    }
}
impl From<String> for PageError {
    fn from(value: String) -> Self {
        Self::Message(value)
    }
}
impl From<super::context::RenderError> for PageError {
    fn from(error: super::context::RenderError) -> Self {
        match error {
            super::context::RenderError::Context(cause) => Self::Context(cause),
            super::context::RenderError::StageContext { message, cause } => {
                Self::StageContext { message, cause }
            }
            super::context::RenderError::Message(message) => Self::Message(message),
        }
    }
}

/// RenderPage for a background caller. Does not mutate or retain the diagram.
pub fn render_page(
    diagram: Option<&crate::target::Diagram>,
    options: Option<&super::render::Options>,
) -> Result<Page, PageError> {
    render_page_with_context(None, diagram, options)
}

/// Page export with typed caller cancellation. Cancellation is checked in the
/// metadata/projection loops, throughout mesh construction and between raster
/// and encoding stages. Foundation raster/encoding calls remain synchronous.
pub fn render_page_with_context(
    token: Option<&d2_rs_core::CancellationToken>,
    diagram: Option<&crate::target::Diagram>,
    options: Option<&super::render::Options>,
) -> Result<Page, PageError> {
    let mut o = super::render::normalize(options)?;
    if o.format != "png" {
        return Err(PageError::Message(
            "isometric page export requires PNG rendering".into(),
        ));
    }
    let budget = o.link_budget.get_or_insert(d2_rs_scenebuild::LinkBudget {
        max_regions: 4096,
        max_string_bytes: 1 << 20,
    });
    let caller = token
        .map(super::context::RenderContext::from_token)
        .unwrap_or_default();
    let diagram =
        diagram.ok_or_else(|| PageError::Message("isometric page requires a diagram".into()))?;
    preflight_page_metadata(&caller, diagram, *budget)?;
    caller.check()?;
    let ctx = super::context::RenderContext::new(token, o.timeout_ns);
    let (png, native) = super::render::render_static_png(&ctx, diagram, &o)?;
    let links = project_page_links(&ctx, &native)?;
    Ok(Page {
        png,
        links,
        width: native.width,
        height: native.height,
    })
}

fn preflight_page_metadata(
    ctx: &super::context::RenderContext,
    diagram: &crate::target::Diagram,
    budget: d2_rs_scenebuild::LinkBudget,
) -> Result<(), PageError> {
    let (mut regions, mut bytes) = (0, 0);
    let mut check = |link: &str, tooltip: &str, pretty: &str| -> Result<(), PageError> {
        ctx.check()?;
        if !pretty.is_empty() && link.is_empty() {
            return Err(PageError::Message(
                "isometric page prettyLink requires a non-empty link".into(),
            ));
        }
        if link.is_empty() && tooltip.is_empty() {
            return Ok(());
        }
        regions += 1;
        if regions > budget.max_regions {
            return Err(PageError::Message(format!(
                "isometric page link regions exceed {}",
                budget.max_regions
            )));
        }
        for value in [link, tooltip] {
            if value.len() as i64 > budget.max_string_bytes - bytes {
                return Err(PageError::Message(format!(
                    "isometric page link metadata exceeds {} bytes",
                    budget.max_string_bytes
                )));
            }
            bytes += value.len() as i64;
            for c in value.chars() {
                ctx.check()?;
                // Rust strings are valid UTF-8; apply the same XML scalar ranges.
                validate_page_link_text(c.encode_utf8(&mut [0; 4]))?;
            }
        }
        validate_native_link(link)?;
        if !link.is_empty()
            && !tooltip.is_empty()
            && d2_rs_scenebuild::parse_request_uri_has_host(tooltip)
        {
            return Err(PageError::Message(
                "isometric page tooltip must not be a URL when link is also set".into(),
            ));
        }
        Ok(())
    };
    for shape in diagram.shapes.iter().flatten() {
        check(&shape.link, &shape.tooltip, &shape.pretty_link)?;
    }
    for edge in diagram.connections.iter().flatten() {
        check(&edge.link, &edge.tooltip, &edge.pretty_link)?;
        if (!edge.link.is_empty() || !edge.tooltip.is_empty())
            && edge.text.label.is_empty()
            && edge.icon.is_none()
        {
            return Err(PageError::Message(format!(
                "isometric page connection {} needs a label or icon for its metadata hit region",
                d2_rs_core::quote_go_bytes(edge.id.as_bytes())
            )));
        }
    }
    ctx.check().map_err(Into::into)
}

fn project_page_links(
    ctx: &super::context::RenderContext,
    scene: &super::svg::NativeScene,
) -> Result<std::vec::Vec<d2_rs_scene::LinkRegion>, PageError> {
    let mut links = std::vec::Vec::with_capacity(scene.links.len());
    let camera = super::framing::camera_at_resolution(scene.camera, scene.width, scene.height);
    for link in &scene.links {
        ctx.check()?;
        let (mut x0, mut y0, mut x1, mut y1) = (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        for point in &link.points {
            let p = camera.project(*point);
            (x0, y0, x1, y1) = (
                go_min(x0, p.x),
                go_min(y0, p.y),
                go_max(x1, p.x),
                go_max(y1, p.y),
            );
        }
        (x0, y0, x1, y1) = (
            go_max(0.0, x0),
            go_max(0.0, y0),
            go_min(scene.width as f64, x1),
            go_min(scene.height as f64, y1),
        );
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        links.push(d2_rs_scene::LinkRegion {
            r#box: d2_rs_scene::Box {
                x: x0,
                y: y0,
                width: x1 - x0,
                height: y1 - y0,
            },
            url: link.url.clone(),
            tooltip: link.tooltip.clone(),
            target: link.target.clone(),
        });
    }
    // Stable order gives small, specific annotations priority in document viewers.
    links.stable_sort_by(|a, b| {
        (b.r#box.width * b.r#box.height).total_cmp(&(a.r#box.width * a.r#box.height))
    });
    Ok(links)
}

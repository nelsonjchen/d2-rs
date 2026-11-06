// SPDX-License-Identifier: MPL-2.0
//! d2isometricimg vector_surface.go: the retained d2scene document behind a
//! surface texture, which SVG output serializes instead of the pixels.
//! Go's context-scoped registry (`nativeVectorContext`) is a shared
//! [`VectorCtx`]; textures and surfaces keep Go's pointer identities as Rc
//! identities, and the fields Go mutates through shared pointers are cells.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use d2_rs_scene::{
    aspect_ratio_matrix, AspectAlign, AspectRatio, Box as SceneBox, Clip, Document, FillRule, Matrix, Node, Nrgba, Paint,
    Path, PathCommand, Primitive, Rect, ViewportAlign, ViewportFit,
};

use super::geometry::MeshBuilder;
use super::mesh::Texture;
use super::route_labels::caption_finite;

#[derive(Clone, Debug, Default)]
pub struct VectorSurface {
    pub document: Rc<RefCell<Document>>,
    pub ink_coverage: RefCell<Option<Rc<VectorSurface>>>,
    pub coverage_opacity: Cell<f64>,
    pub cap_background: Option<Nrgba>,
    pub cap_paint: Option<Paint>,
}

/// nativeVectorRegistry, keyed by texture identity. Holding the texture Rc
/// keeps its address unique for the registry's lifetime, as Go's map keys do.
#[derive(Default)]
pub struct VectorRegistry {
    surfaces: HashMap<usize, (Rc<Texture>, Rc<VectorSurface>)>,
}

/// The serial export's registry; None outside vector export.
pub type VectorCtx = Option<Rc<RefCell<VectorRegistry>>>;

pub fn native_vector_context() -> VectorCtx {
    Some(Rc::new(RefCell::new(VectorRegistry::default())))
}

/// A solid cap's source viewport includes its centered stroke; fill the
/// opaque substrate before its source paint.
pub fn native_vector_solid_cap(surface: Option<Rc<VectorSurface>>, fill: Nrgba) -> Option<Rc<VectorSurface>> {
    let surface = surface?;
    if fill.a != 255 {
        return Some(surface);
    }
    let mut copy = (*surface).clone();
    copy.cap_background = Some(fill);
    Some(Rc::new(copy))
}

pub fn retain_native_vector_surface(ctx: &VectorCtx, texture: &Rc<Texture>, document: &Document) -> Result<(), String> {
    let Some(registry) = ctx else {
        return Ok(());
    };
    let mut registry = registry.borrow_mut();
    if registry.surfaces.len() >= 100000 {
        return Err("native SVG exceeds 100000 retained surfaces".into());
    }
    let mut view = document.clone();
    if view.logical_width <= 0.0 || view.logical_height <= 0.0 {
        view.logical_width = texture.width as f64;
        view.logical_height = texture.height as f64;
    }
    let surface = Rc::new(VectorSurface { document: Rc::new(RefCell::new(view)), ..Default::default() });
    registry.surfaces.insert(Rc::as_ptr(texture) as usize, (texture.clone(), surface));
    Ok(())
}

pub fn native_vector_for_texture(ctx: &VectorCtx, texture: &Rc<Texture>) -> Option<Rc<VectorSurface>> {
    let registry = ctx.as_ref()?.borrow();
    registry.surfaces.get(&(Rc::as_ptr(texture) as usize)).map(|(_, surface)| surface.clone())
}

/// Match an RGBA compositing step performed after a document was
/// rasterized: background first, then the document, with one opacity fade.
pub fn retain_styled_native_vector_surface(
    ctx: &VectorCtx,
    texture: &Rc<Texture>,
    document: &Document,
    background: Option<&Paint>,
    opacity: f64,
) -> Result<(), String> {
    if ctx.is_none() {
        return Ok(());
    }
    let mut view = document.clone();
    let matrix = native_vector_viewport(&view)?;
    let mut content = Node::new(None);
    content.transform = Matrix::scale(view.logical_width, view.logical_height).mul(matrix);
    content.children = Some(view.root.take().map(|root| vec![*root]).unwrap_or_default());
    let mut root = Node::new(None);
    root.opacity = opacity;
    let bx = SceneBox { x: 0.0, y: 0.0, width: view.logical_width, height: view.logical_height };
    let mut children = vec![];
    if let Some(background) = background {
        children.push(Node::new(Some(Primitive::Rect(Rect { r#box: bx, fill: Some(background.clone()), ..Default::default() }))));
    }
    children.push(content);
    root.children = Some(children);
    view.root = Some(Box::new(root));
    view.view_box = bx;
    view.viewport_fit = ViewportFit::STRETCH;
    retain_native_vector_surface(ctx, texture, &view)
}

pub fn native_vector_viewport(doc: &Document) -> Result<Matrix, String> {
    let mut aspect = AspectRatio { align: AspectAlign::NONE, ..Default::default() };
    if doc.viewport_fit == ViewportFit::MEET {
        aspect.align = AspectAlign::X_MIN_Y_MIN;
        if doc.viewport_align == ViewportAlign::X_MID_Y_MID {
            aspect.align = AspectAlign::X_MID_Y_MID;
        }
    }
    let (width, height) = (doc.logical_width, doc.logical_height);
    if !caption_finite(&[width]) || !caption_finite(&[height]) || width <= 0.0 || height <= 0.0 {
        return Err("native SVG surface has invalid dimensions".into());
    }
    let matrix = aspect_ratio_matrix(doc.view_box, SceneBox { x: 0.0, y: 0.0, width, height }, aspect)
        .map_err(|error| format!("native SVG surface has invalid viewport: {error}"))?;
    Ok(Matrix::scale(1.0 / width, 1.0 / height).mul(matrix))
}

/// Match the border-label opening without baking its antialiased pixels
/// into SVG. Both rectangles live in the source document's coordinates.
pub fn native_vector_aperture(surface: &VectorSurface, aperture: SceneBox) {
    if aperture.width <= 0.0 || aperture.height <= 0.0 {
        return;
    }
    let mut document = surface.document.borrow_mut();
    let mut root = Node::new(None);
    root.children = Some(document.root.take().map(|r| vec![*r]).unwrap_or_default());
    let mut path = Path { fill_rule: FillRule::EVEN_ODD, ..Default::default() };
    for bx in [document.view_box, aperture] {
        path.commands.extend([
            PathCommand::move_to(bx.x, bx.y),
            PathCommand::line_to(bx.x + bx.width, bx.y),
            PathCommand::line_to(bx.x + bx.width, bx.y + bx.height),
            PathCommand::line_to(bx.x, bx.y + bx.height),
            PathCommand::close(),
        ]);
    }
    root.clip = Some(Clip { path, transform: Matrix::identity() });
    document.root = Some(Box::new(root));
}

impl MeshBuilder {
    pub fn vector_for_texture(&self, texture: &Rc<Texture>) -> Option<Rc<VectorSurface>> {
        native_vector_for_texture(&self.vectors, texture)
    }

    pub fn retain_vector_surface(&self, texture: &Rc<Texture>, document: &Document) -> Result<(), String> {
        retain_native_vector_surface(&self.vectors, texture, document)
    }
}

// SPDX-License-Identifier: MPL-2.0
//! Shared retained surface registry for the native mesh builder and SVG writer.
//! Apertures and coverage edits preserve document and material identities.

use d2_rs_scene::{Document, Nrgba, Paint};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone, Debug, Default)]
pub struct VectorSurface {
    pub document: Rc<RefCell<Document>>,
    pub ink_coverage: RefCell<Option<Rc<VectorSurface>>>,
    pub coverage_opacity: Cell<f64>,
    pub cap_background: Option<Nrgba>,
    pub cap_paint: Option<Paint>,
}

/// Serial export registry. Keeping the texture Rc prevents pointer reuse.
#[derive(Default)]
pub struct VectorRegistry {
    surfaces: std::collections::HashMap<usize, (Rc<super::mesh::Texture>, Rc<VectorSurface>)>,
}
pub type VectorCtx = Option<Rc<std::cell::RefCell<VectorRegistry>>>;
pub fn native_vector_context() -> VectorCtx {
    Some(Rc::new(std::cell::RefCell::new(VectorRegistry::default())))
}
pub fn native_vector_for_texture(
    ctx: &VectorCtx,
    texture: &Rc<super::mesh::Texture>,
) -> Option<Rc<VectorSurface>> {
    ctx.as_ref()?
        .borrow()
        .surfaces
        .get(&(Rc::as_ptr(texture) as usize))
        .map(|(_, surface)| surface.clone())
}
pub fn retain_native_vector_surface(
    ctx: &VectorCtx,
    texture: &Rc<super::mesh::Texture>,
    document: &Document,
) -> Result<(), String> {
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
    registry.surfaces.insert(
        Rc::as_ptr(texture) as usize,
        (
            texture.clone(),
            Rc::new(VectorSurface {
                document: Rc::new(RefCell::new(view)),
                ..Default::default()
            }),
        ),
    );
    Ok(())
}

/// vector_surface.go retainStyledNativeVectorSurface. The scene owns its
/// original coordinates under a normalized viewport, with backing and opacity
/// outside that content transform.
pub fn retain_styled_native_vector_surface(
    ctx: &VectorCtx,
    texture: &Rc<super::mesh::Texture>,
    document: &Document,
    background: Option<&Paint>,
    opacity: f64,
) -> Result<(), String> {
    use d2_rs_scene::*;
    if ctx.is_none() {
        return Ok(());
    }
    let mut view = document.clone();
    let mut aspect = AspectRatio {
        align: AspectAlign::NONE,
        ..Default::default()
    };
    if view.viewport_fit == ViewportFit::MEET {
        aspect.align = if view.viewport_align == ViewportAlign::X_MID_Y_MID {
            AspectAlign::X_MID_Y_MID
        } else {
            AspectAlign::X_MIN_Y_MIN
        };
    }
    let (w, h) = (view.logical_width, view.logical_height);
    if !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
        return Err("native SVG surface has invalid dimensions".into());
    }
    let matrix = aspect_ratio_matrix(
        view.view_box,
        Box {
            width: w,
            height: h,
            ..Default::default()
        },
        aspect,
    )
    .map_err(|e| format!("native SVG surface has invalid viewport: {e}"))?;
    let matrix = Matrix::scale(1.0 / w, 1.0 / h).mul(matrix);
    let mut content = Node::new(None);
    content.transform = Matrix::scale(w, h).mul(matrix);
    content.children = Some(view.root.take().map(|root| vec![*root]).unwrap_or_default());
    let mut root = Node::new(None);
    root.opacity = opacity;
    let bx = Box {
        width: w,
        height: h,
        ..Default::default()
    };
    let mut children = vec![];
    if let Some(background) = background {
        children.push(Node::new(Some(Primitive::Rect(Rect {
            r#box: bx,
            fill: Some(background.clone()),
            ..Default::default()
        }))));
    }
    children.push(content);
    root.children = Some(children);
    view.root = Some(std::boxed::Box::new(root));
    view.view_box = bx;
    view.viewport_fit = ViewportFit::STRETCH;
    retain_native_vector_surface(ctx, texture, &view)
}

/// A solid cap keeps the source paint on its opaque physical substrate.
pub fn native_vector_solid_cap(
    surface: Option<Rc<VectorSurface>>,
    fill: Nrgba,
) -> Option<Rc<VectorSurface>> {
    let surface = surface?;
    if fill.a != 255 {
        return Some(surface);
    }
    let mut copy = (*surface).clone();
    copy.cap_background = Some(fill);
    Some(Rc::new(copy))
}
/// Mutate the shared source document before the SVG writer visits any face.
pub fn native_vector_aperture(surface: &VectorSurface, aperture: d2_rs_scene::Box) {
    use d2_rs_scene::*;
    if aperture.width <= 0.0 || aperture.height <= 0.0 {
        return;
    }
    let mut document = surface.document.borrow_mut();
    let mut root = Node::new(None);
    root.children = Some(document.root.take().map(|r| vec![*r]).unwrap_or_default());
    let mut path = Path {
        fill_rule: FillRule::EVEN_ODD,
        ..Default::default()
    };
    for bx in [document.view_box, aperture] {
        path.commands.extend([
            PathCommand::move_to(bx.x, bx.y),
            PathCommand::line_to(bx.x + bx.width, bx.y),
            PathCommand::line_to(bx.x + bx.width, bx.y + bx.height),
            PathCommand::line_to(bx.x, bx.y + bx.height),
            PathCommand::close(),
        ]);
    }
    root.clip = Some(Clip {
        path,
        transform: Matrix::identity(),
    });
    document.root = Some(std::boxed::Box::new(root));
}

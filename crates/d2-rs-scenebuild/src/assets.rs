// SPDX-License-Identifier: MPL-2.0
//! assets.go: image/icon resource admission, deduplication and unavailable
//! placeholders. Fetching/probing belongs to the caller's bounded resolver.
//! SVG imports use the bounded native importer through svgimport.rs.

use d2_rs_scene::{
    AspectAlign, AspectFit, AspectRatio, Asset, Box as SceneBox, Image, Node, Primitive,
};
use d2_rs_target::{Connection, Shape};
use sha2::{Digest, Sha256};
use std::sync::Arc;

use crate::golib::geo;
use crate::golib::label::{self, Position};
use crate::golib::shape::Shape as _;
use crate::target_ext::*;
use crate::util::*;
use crate::{Builder, target_geometry};

/// imageasset.Kind. The byte discriminant is part of hashImageAsset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKind {
    Raster,
    Svg,
    Other(u8),
}

impl ImageKind {
    fn byte(self) -> u8 {
        match self {
            Self::Raster => 1,
            Self::Svg => 2,
            Self::Other(value) => value,
        }
    }
}

impl std::fmt::Display for ImageKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Raster => f.write_str("raster"),
            Self::Svg => f.write_str("svg"),
            Self::Other(value) => write!(f, "Kind({value})"),
        }
    }
}

/// Already-probed imageasset.Resource. The resolver is responsible for
/// validating the format, dimensions, first-frame semantics and its own
/// cumulative I/O/storage limits before returning this immutable resource.
/// Build copies the bytes, so returned scenes never borrow resolver storage.
#[derive(Clone, Debug)]
pub struct ImageResource {
    pub kind: ImageKind,
    pub mime_type: String,
    pub data: Arc<[u8]>,
    pub pixel_width: i64,
    pub pixel_height: i64,
    pub decoded_bytes: i64,
}

/// Keep absence/transport errors distinct from malformed data and limit or
/// policy failures. Only `unavailable` selects the upstream placeholder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageResolveError {
    pub unavailable: bool,
    pub message: String,
}

impl std::fmt::Display for ImageResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ImageResolveError {}

/// Native scene handoff for imageasset.Resolver.Resolve. `None` preserves
/// the source's nil-resource error. Cancellation is not yet exposed here.
pub trait ImageResolver: std::fmt::Debug + Send + Sync {
    fn resolve(
        &self,
        source: &str,
    ) -> std::result::Result<Option<ImageResource>, ImageResolveError>;
}

fn default_image_aspect() -> AspectRatio {
    AspectRatio {
        align: AspectAlign::X_MID_Y_MID,
        fit: AspectFit::MEET,
    }
}

/// shapeAssetIsEmitted.
pub(crate) fn shape_asset_is_emitted(s: &Shape) -> bool {
    s.icon.is_some()
        && (s.opacity != 0.0
            || matches!(
                s.type_.as_str(),
                SHAPE_IMAGE | SHAPE_CLASS | SHAPE_SQL_TABLE
            ))
}

impl Builder<'_> {
    /// preflightAssets.
    pub(crate) fn preflight_assets(&mut self) -> Result<()> {
        for s in self.diagram.shapes() {
            if shape_asset_is_emitted(s) {
                self.resolve_image_asset(
                    &format!("shape {}", q(&s.id)),
                    &s.icon.as_ref().expect("icon").string(),
                )?;
            }
        }
        for s in self.diagram.shapes() {
            if s.opacity == 0.0 || s.text.label.is_empty() || s.text.language != "latex" {
                continue;
            }
            self.resolve_latex_asset(&format!("shape {}", q(&s.id)), &s.text.label, &s.font_color())?;
        }
        for c in self.diagram.connections() {
            if let Some(icon) = &c.icon {
                self.resolve_image_asset(&format!("connection {}", q(&c.id)), &icon.string())?;
            }
        }
        for c in self.diagram.connections() {
            if c.opacity == 0.0 || c.text.label.is_empty() || c.text.language != "latex" {
                continue;
            }
            self.resolve_latex_asset(
                &format!("connection {}", q(&c.id)),
                &c.text.label,
                &c.text.color,
            )?;
        }
        if let Some(legend) = &self.diagram.legend {
            for (index, s) in legend.shapes.iter().flatten().enumerate() {
                if s.text.label.is_empty() {
                    continue;
                }
                let icon = crate::legend::legend_shape_icon_target(index, s);
                if !shape_asset_is_emitted(&icon) {
                    continue;
                }
                self.resolve_image_asset(
                    &format!("legend shape[{index}] {}", q(&s.id)),
                    &icon.icon.as_ref().expect("icon").string(),
                )?;
            }
        }
        Ok(())
    }

    /// resolveImageAsset.
    fn resolve_image_asset(&mut self, object: &str, source: &str) -> Result<String> {
        if let Some(id) = self.source_asset_ids.get(source) {
            return Ok(id.clone());
        }
        let Some(options) = self.options.assets.as_ref().filter(|a| a.has_resolver()) else {
            return Err(unsupported(
                object,
                "image/icon asset without a configured resolver",
            ));
        };
        let Some(resolver) = &options.resolver_impl else {
            return Err(unsupported(
                object,
                "image/icon asset resolution in the native scene builder (lib/imageasset is not ported)",
            ));
        };
        let resource = match resolver.resolve(source) {
            Err(error) if error.unavailable => {
                let id = "image:unavailable".to_owned();
                self.assets
                    .entry(id.clone())
                    .or_insert_with(unavailable_image_asset);
                self.source_asset_ids.insert(source.to_owned(), id.clone());
                return Ok(id);
            }
            Err(error) => return err(format!("scene: {object} image asset: {error}")),
            Ok(None) => return err(format!("scene: {object} image asset resolver returned nil")),
            Ok(Some(resource)) => resource,
        };
        let mut hash = Sha256::new();
        hash.update([resource.kind.byte(), 0]);
        hash.update(resource.mime_type.as_bytes());
        hash.update([0]);
        hash.update(&resource.data);
        let digest: [u8; 32] = hash.finalize().into();
        if let Some(id) = self.image_asset_ids.get(&digest) {
            self.source_asset_ids.insert(source.to_owned(), id.clone());
            return Ok(id.clone());
        }
        let id = format!("image:{:x}", sha2::digest::Output::<Sha256>::from(digest));
        let asset = match resource.kind {
            ImageKind::Raster => Asset::RasterAsset {
                mime_type: resource.mime_type,
                data: resource.data.to_vec(),
                pixel_width: resource.pixel_width,
                pixel_height: resource.pixel_height,
                decoded_bytes: resource.decoded_bytes,
            },
            ImageKind::Svg => {
                let imported = self.import_svg(
                    object,
                    &crate::svgimport::redact_source(source),
                    &resource.data,
                    None,
                    false,
                )?;
                crate::svgimport::vector_asset(&id, imported)
            }
            other => {
                return err(format!(
                    "scene: {object} image asset has unsupported resolved kind {other}"
                ));
            }
        };
        self.assets.insert(id.clone(), asset);
        self.image_asset_ids.insert(digest, id.clone());
        self.source_asset_ids.insert(source.to_owned(), id.clone());
        Ok(id)
    }

    fn build_image_node(
        &mut self,
        object: &str,
        node_id: &str,
        source: &str,
        b: SceneBox,
        radius: f64,
        svg_image_shape_clip: bool,
    ) -> Result<Node> {
        let asset = self.resolve_image_asset(object, source)?;
        let mut n = node(
            node_id,
            Some(Primitive::Image(Image {
                asset,
                r#box: b,
                aspect: default_image_aspect(),
            })),
        );
        let radius = clamp_border_radius(radius, b.width, b.height);
        if radius > 0.0 {
            n.clip = Some(if svg_image_shape_clip {
                image_shape_clip(b, radius)
            } else {
                rounded_box_clip(b, radius)
            });
        }
        Ok(n)
    }

    /// buildShapeImage.
    pub(crate) fn build_shape_image(&mut self, s: &Shape) -> Result<Node> {
        let object = format!("shape {}", q(&s.id));
        let Some(icon) = &s.icon else {
            return Err(invalid_field(
                &object,
                "icon",
                None,
                "must identify an image source for an image shape",
            ));
        };
        self.build_image_node(
            &object,
            &format!("{}:image", s.id),
            &icon.string(),
            SceneBox {
                x: s.pos.x as f64,
                y: s.pos.y as f64,
                width: s.width as f64,
                height: s.height as f64,
            },
            s.icon_border_radius as f64,
            true,
        )
    }

    /// buildShapeIcon.
    pub(crate) fn build_shape_icon(&mut self, s: &Shape, structured: bool) -> Result<Option<Node>> {
        let Some(icon) = &s.icon else { return Ok(None) };
        if s.type_ == SHAPE_IMAGE {
            return Ok(None);
        }
        let object = format!("shape {}", q(&s.id));
        let position = Position::from_string(&s.icon_position);
        if !position.is_shape_position() {
            return Err(invalid_field(
                &object,
                "iconPosition",
                Some(&s.icon_position),
                "must identify a supported icon position",
            ));
        }
        let b = if structured {
            geo::Box::new(
                geo::Point::new(s.pos.x as f64, s.pos.y as f64),
                s.width as f64,
                s.height as f64,
            )
        } else {
            let geometry = target_geometry(s);
            if position.is_outside() {
                geometry.get_box()
            } else {
                geometry.get_inner_box()
            }
        };
        let size = get_icon_size(&b, &s.icon_position);
        let top_left = position.get_point_on_box(&b, label::PADDING, size as f64, size as f64);
        let radius = if structured {
            0.0
        } else {
            s.icon_border_radius as f64
        };
        self.build_image_node(
            &object,
            &format!("{}:icon", s.id),
            &icon.string(),
            SceneBox {
                x: top_left.x,
                y: top_left.y,
                width: size as f64,
                height: size as f64,
            },
            radius,
            false,
        )
        .map(Some)
    }

    /// buildConnectionIcon.
    pub(crate) fn build_connection_icon(&mut self, c: &Connection) -> Result<Option<Node>> {
        let Some(icon) = &c.icon else { return Ok(None) };
        let object = format!("connection {}", q(&c.id));
        let Some(top_left) = connection_icon_position(c) else {
            return Err(invalid_field(
                &object,
                "iconPosition",
                Some(&c.icon_position),
                "does not resolve on the route",
            ));
        };
        let size = DEFAULT_ICON_SIZE as f64;
        self.build_image_node(
            &object,
            &format!("{}:icon", c.id),
            &icon.string(),
            SceneBox {
                x: top_left.x,
                y: top_left.y,
                width: size,
                height: size,
            },
            c.icon_border_radius,
            false,
        )
        .map(Some)
    }
}

/// newUnavailableImageAsset: one document-owned 64x64 vector placeholder.
/// Built-in missing-image icon shared by native exports and watch previews.
/// Source: d2scenebuild/assets.go at upstream 1c7d0204e.
pub const UNAVAILABLE_IMAGE_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" id="image-unavailable" width="64" height="64" viewBox="0 0 64 64">
  <path id="image-unavailable:page" d="M7 3H42L57 18V61H7Z" fill="#f7f8fa" stroke="#7b838c" stroke-width="2" stroke-linejoin="round"/>
  <rect id="image-unavailable:sky" x="13" y="23" width="38" height="30" fill="#b8d7f2"/>
  <circle id="image-unavailable:sun" cx="23" cy="31" r="4" fill="#fff"/>
  <path id="image-unavailable:hills" d="M13 49L24 37L32 45L39 39L51 49V53H13Z" fill="#55b94b"/>
  <path id="image-unavailable:break" d="M37 54L51 40" fill="none" stroke="#f7f8fa" stroke-width="5" stroke-linecap="square"/>
  <rect id="image-unavailable:image-outline" x="13" y="23" width="38" height="30" fill="none" stroke="#7b838c" stroke-linejoin="round"/>
  <path id="image-unavailable:fold" d="M42 3V18H57" fill="none" stroke="#7b838c" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
</svg>"##;

fn unavailable_image_asset() -> Asset {
    // The fixed icon has its own budget, independent of user SVG imports.
    let imported = d2_rs_math::svg_import::import_labeled(
        "image:unavailable",
        UNAVAILABLE_IMAGE_SVG.as_bytes(),
        [
            UNAVAILABLE_IMAGE_SVG.len(),
            4,
            32,
            64,
            UNAVAILABLE_IMAGE_SVG.len(),
            128,
            1,
            1,
            8,
        ],
        None,
        None,
    )
    .expect("built-in unavailable image is valid SVG");
    let d2_rs_math::svg_import::Outcome::Imported(imported) = imported else {
        unreachable!("built-in icon has no raster resources")
    };
    Asset::VectorAsset {
        view_box: SceneBox {
            width: 64.,
            height: 64.,
            ..Default::default()
        },
        root: Some(Box::new(crate::svgimport::root(imported.scene))),
    }
}

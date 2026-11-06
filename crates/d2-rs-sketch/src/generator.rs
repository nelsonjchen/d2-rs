// SPDX-License-Identifier: MIT
// Translated from rough-go v0.2.0 (github.com/d2lang/rough-go, pinned by the
// D2 v0.9.0 go.mod): core.go, generator.go, renderer.go, path.go, number.go,
// math.go, geometry.go, filler.go, filler_hachure.go, filler_hatch.go,
// filler_zigzag.go, filler_zigzag_line.go, filler_dot.go, filler_dashed.go and
// filler_scan_line_hachure.go. rough-go derives from Rough.js 4.6.6
// (Copyright (c) 2019 Preet Shihn), hachure-fill 0.5.2 (Copyright (c) 2023
// Preet Shihn) and path-data-parser 0.1.0 (Copyright (c) 2020 Preet Shihn);
// Copyright (c) 2026 rough-go contributors. See THIRD_PARTY_NOTICES.md.
//! Source-shaped rough-go `Generator`: drawables are operation sets that
//! `Generator::to_paths` serializes exactly like rough-go's `ToPaths`.
//!
//! Floating-point evaluation follows the pinned darwin/arm64 Go 1.27 build:
//! every `a ± x*y` the compiler contracts into FMADDD/FMSUBD (read from
//! `go build -gcflags=github.com/d2lang/rough-go=-S`) is written with
//! [`fmadd`]/[`fmsub`] and marked `ARM64:`; source sites guarded by
//! `jsNumberStep` stay separately rounded. Trigonometry uses Go's own
//! `math` kernels ([`super::gomath`]), not the platform libm.
//!
//! rough-go's unused `processSegment`/`bezierTo` (dead code in the pinned
//! module) are not translated. Like rough-go, an unseeded randomizer (seed 0
//! or NaN) is not reproducible; this port falls back to the crate's fixed
//! stream instead of Go's `math/rand`.

use std::cell::RefCell;
use std::rc::Rc;

use super::RandomState;
use super::gomath::{self, fmadd, fmsub};

/// core.go `Point`.
pub type RoughPoint = [f64; 2];
/// core.go `Line`.
pub type RoughLine = [RoughPoint; 2];

const NO_STYLE: &str = "none";

/// core.go `Options`: `None` is an omitted option.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Options {
    pub max_randomness_offset: Option<f64>,
    pub roughness: Option<f64>,
    pub bowing: Option<f64>,
    pub stroke: Option<String>,
    pub stroke_width: Option<f64>,
    pub curve_fitting: Option<f64>,
    pub curve_tightness: Option<f64>,
    pub curve_step_count: Option<f64>,
    pub fill: Option<String>,
    pub fill_style: Option<String>,
    pub fill_weight: Option<f64>,
    pub hachure_angle: Option<f64>,
    pub hachure_gap: Option<f64>,
    pub simplification: Option<f64>,
    pub dash_offset: Option<f64>,
    pub dash_gap: Option<f64>,
    pub zigzag_offset: Option<f64>,
    pub seed: Option<f64>,
    pub stroke_line_dash: Option<Vec<f64>>,
    pub stroke_line_dash_offset: Option<f64>,
    pub fill_line_dash: Option<Vec<f64>>,
    pub fill_line_dash_offset: Option<f64>,
    pub disable_multi_stroke: Option<bool>,
    pub disable_multi_stroke_fill: Option<bool>,
    pub preserve_vertices: Option<bool>,
    pub fixed_decimal_place_digits: Option<f64>,
    pub fill_shape_roughness_gain: Option<f64>,
}

/// core.go `ResolvedOptions`.
///
/// `Clone` is Go's struct copy: the randomizer pointer stays shared (fillers
/// rely on that). [`ResolvedOptions::deep_clone`] is `cloneResolvedOptions`.
#[derive(Clone, Debug)]
pub struct ResolvedOptions {
    pub max_randomness_offset: f64,
    pub roughness: f64,
    pub bowing: f64,
    pub stroke: String,
    pub stroke_width: f64,
    pub curve_fitting: f64,
    pub curve_tightness: f64,
    pub curve_step_count: f64,
    pub fill: String,
    pub fill_style: String,
    pub fill_weight: f64,
    pub hachure_angle: f64,
    pub hachure_gap: f64,
    pub simplification: f64,
    pub dash_offset: f64,
    pub dash_gap: f64,
    pub zigzag_offset: f64,
    pub seed: f64,
    pub stroke_line_dash: Vec<f64>,
    pub stroke_line_dash_offset: f64,
    pub fill_line_dash: Vec<f64>,
    pub fill_line_dash_offset: f64,
    pub disable_multi_stroke: bool,
    pub disable_multi_stroke_fill: bool,
    pub preserve_vertices: bool,
    pub fixed_decimal_place_digits: Option<f64>,
    pub fill_shape_roughness_gain: f64,
    pub roughness_gain: f64,
    randomizer: Option<Rc<RefCell<RandomState>>>,
}

impl ResolvedOptions {
    /// generator.go `cloneResolvedOptions`: the randomizer state is copied.
    pub fn deep_clone(&self) -> Self {
        let mut copy = self.clone();
        copy.randomizer = self
            .randomizer
            .as_ref()
            .map(|r| Rc::new(RefCell::new(*r.borrow())));
        copy
    }
}

impl Default for ResolvedOptions {
    /// core.go `defaultResolvedOptions`.
    fn default() -> Self {
        Self {
            max_randomness_offset: 2.0,
            roughness: 1.0,
            bowing: 1.0,
            stroke: "#000".into(),
            stroke_width: 1.0,
            curve_fitting: 0.95,
            curve_tightness: 0.0,
            curve_step_count: 9.0,
            fill: String::new(),
            fill_style: "hachure".into(),
            fill_weight: -1.0,
            hachure_angle: -41.0,
            hachure_gap: -1.0,
            simplification: 0.0,
            dash_offset: -1.0,
            dash_gap: -1.0,
            zigzag_offset: -1.0,
            seed: 0.0,
            stroke_line_dash: Vec::new(),
            stroke_line_dash_offset: 0.0,
            fill_line_dash: Vec::new(),
            fill_line_dash_offset: 0.0,
            disable_multi_stroke: false,
            disable_multi_stroke_fill: false,
            preserve_vertices: false,
            fixed_decimal_place_digits: None,
            fill_shape_roughness_gain: 0.8,
            roughness_gain: 1.0,
            randomizer: None,
        }
    }
}

/// core.go `mergeOptions` (base is passed by value, sharing its randomizer).
fn merge_options(mut base: ResolvedOptions, o: Option<&Options>) -> ResolvedOptions {
    let Some(o) = o else { return base };
    macro_rules! set {
        ($($field:ident),*) => {$(
            if let Some(v) = &o.$field {
                base.$field = v.clone();
            }
        )*};
    }
    set!(
        max_randomness_offset,
        roughness,
        bowing,
        stroke,
        stroke_width,
        curve_fitting,
        curve_tightness,
        curve_step_count,
        fill,
        fill_style,
        fill_weight,
        hachure_angle,
        hachure_gap,
        simplification,
        dash_offset,
        dash_gap,
        zigzag_offset,
        seed,
        stroke_line_dash,
        stroke_line_dash_offset,
        fill_line_dash,
        fill_line_dash_offset,
        disable_multi_stroke,
        disable_multi_stroke_fill,
        preserve_vertices,
        fill_shape_roughness_gain
    );
    if let Some(v) = o.fixed_decimal_place_digits {
        base.fixed_decimal_place_digits = Some(v);
    }
    base
}

/// core.go `OpType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpType {
    Move,
    BCurveTo,
    LineTo,
    QCurveTo,
}

/// core.go `OpSetType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpSetType {
    Path,
    FillPath,
    FillSketch,
    Path2DFill,
    Path2DPattern,
}

/// core.go `Op`.
#[derive(Clone, Debug, PartialEq)]
pub struct Op {
    pub op: OpType,
    pub data: Vec<f64>,
}

fn op(op: OpType, data: Vec<f64>) -> Op {
    Op { op, data }
}

/// core.go `OpSet`.
#[derive(Clone, Debug, PartialEq)]
pub struct OpSet {
    pub kind: OpSetType,
    pub ops: Vec<Op>,
    pub size: Option<RoughPoint>,
    pub path: String,
}

fn opset(kind: OpSetType, ops: Vec<Op>) -> OpSet {
    OpSet {
        kind,
        ops,
        size: None,
        path: String::new(),
    }
}

/// core.go `Drawable`.
#[derive(Clone, Debug)]
pub struct Drawable {
    pub shape: &'static str,
    pub options: Option<ResolvedOptions>,
    pub sets: Vec<OpSet>,
}

/// core.go `PathInfo`.
#[derive(Clone, Debug, PartialEq)]
pub struct PathInfo {
    pub d: String,
    pub stroke: String,
    pub stroke_width: f64,
    pub fill: String,
    pub pattern: Option<Box<PatternInfo>>,
}

/// core.go `PatternInfo`.
#[derive(Clone, Debug, PartialEq)]
pub struct PatternInfo {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub view_box: String,
    pub pattern_units: String,
    pub path: PathInfo,
}

/// core.go `Surface`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Surface {
    pub width: f64,
    pub height: f64,
}

/// generator.go `Generator`.
#[derive(Clone, Debug)]
pub struct Generator {
    default_options: ResolvedOptions,
    surface: Option<Surface>,
}

fn drawable(shape: &'static str, sets: Vec<OpSet>, options: &ResolvedOptions) -> Drawable {
    Drawable {
        shape,
        sets,
        options: Some(options.deep_clone()),
    }
}

impl Generator {
    /// generator.go `NewGenerator` with `Config.Options`.
    pub fn new(config_options: Option<&Options>, surface: Option<Surface>) -> Self {
        Self {
            default_options: merge_options(ResolvedOptions::default(), config_options),
            surface,
        }
    }

    /// generator.go `DefaultOptions`.
    pub fn default_options(&self) -> ResolvedOptions {
        self.default_options.deep_clone()
    }

    /// generator.go `options`: nil options use the generator's own defaults
    /// (and their persistent randomizer).
    fn with_options<R>(
        &mut self,
        options: Option<&Options>,
        f: impl FnOnce(&mut ResolvedOptions) -> R,
    ) -> R {
        match options {
            None => f(&mut self.default_options),
            Some(options) => {
                let mut merged = merge_options(self.default_options.clone(), Some(options));
                f(&mut merged)
            }
        }
    }

    /// generator.go `Line`.
    pub fn line(
        &mut self,
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        options: Option<&Options>,
    ) -> Drawable {
        self.with_options(options, |o| {
            let set = line(x1, y1, x2, y2, o);
            drawable("line", vec![set], o)
        })
    }

    /// generator.go `Rectangle`.
    pub fn rectangle(
        &mut self,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        options: Option<&Options>,
    ) -> Drawable {
        self.with_options(options, |o| {
            let mut sets = Vec::new();
            let outline = rectangle(x, y, width, height, o);
            if !o.fill.is_empty() {
                let mut points = vec![
                    [x, y],
                    [x + width, y],
                    [x + width, y + height],
                    [x, y + height],
                ];
                if o.fill_style == "solid" {
                    sets.push(solid_fill_polygons(&[points], o));
                } else {
                    sets.push(pattern_fill_polygon(&mut points, o));
                }
            }
            if o.stroke != NO_STYLE {
                sets.push(outline);
            }
            drawable("rectangle", sets, o)
        })
    }

    /// generator.go `Ellipse`.
    pub fn ellipse(
        &mut self,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        options: Option<&Options>,
    ) -> Drawable {
        self.with_options(options, |o| {
            let mut sets = Vec::new();
            let params = generate_ellipse_params(width, height, o);
            let mut response = ellipse_with_params(x, y, o, params);
            if !o.fill.is_empty() {
                if o.fill_style == "solid" {
                    let mut shape = ellipse_with_params(x, y, o, params).opset;
                    shape.kind = OpSetType::FillPath;
                    sets.push(shape);
                } else {
                    sets.push(pattern_fill_polygon(&mut response.estimated_points, o));
                }
            }
            if o.stroke != NO_STYLE {
                sets.push(response.opset);
            }
            drawable("ellipse", sets, o)
        })
    }

    /// generator.go `Circle`.
    pub fn circle(&mut self, x: f64, y: f64, diameter: f64, options: Option<&Options>) -> Drawable {
        let mut d = self.ellipse(x, y, diameter, diameter, options);
        d.shape = "circle";
        d
    }

    /// generator.go `LinearPath`.
    pub fn linear_path(&mut self, points: &[RoughPoint], options: Option<&Options>) -> Drawable {
        self.with_options(options, |o| {
            let set = linear_path(points, false, o);
            drawable("linearPath", vec![set], o)
        })
    }

    /// generator.go `Arc`.
    #[allow(clippy::too_many_arguments)]
    pub fn arc(
        &mut self,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        start: f64,
        stop: f64,
        closed: bool,
        options: Option<&Options>,
    ) -> Drawable {
        self.with_options(options, |o| {
            let mut sets = Vec::new();
            let outline = arc(x, y, width, height, start, stop, closed, true, o);
            if closed && !o.fill.is_empty() {
                if o.fill_style == "solid" {
                    let mut fill_options = o.deep_clone();
                    fill_options.disable_multi_stroke = true;
                    let mut shape = arc(
                        x,
                        y,
                        width,
                        height,
                        start,
                        stop,
                        true,
                        false,
                        &mut fill_options,
                    );
                    shape.kind = OpSetType::FillPath;
                    sets.push(shape);
                } else {
                    sets.push(pattern_fill_arc(x, y, width, height, start, stop, o));
                }
            }
            if o.stroke != NO_STYLE {
                sets.push(outline);
            }
            drawable("arc", sets, o)
        })
    }

    /// generator.go `Curve`.
    pub fn curve(&mut self, points: &[RoughPoint], options: Option<&Options>) -> Drawable {
        self.with_options(options, |o| {
            let set = curve(points, o);
            drawable("curve", vec![set], o)
        })
    }

    /// generator.go `Polygon`.
    pub fn polygon(&mut self, points: &[RoughPoint], options: Option<&Options>) -> Drawable {
        self.with_options(options, |o| {
            let mut sets = Vec::new();
            let outline = linear_path(points, true, o);
            if !o.fill.is_empty() {
                if o.fill_style == "solid" {
                    sets.push(solid_fill_polygons(&[points.to_vec()], o));
                } else {
                    let mut points = points.to_vec();
                    sets.push(pattern_fill_polygon(&mut points, o));
                }
            }
            if o.stroke != NO_STYLE {
                sets.push(outline);
            }
            drawable("polygon", sets, o)
        })
    }

    /// generator.go `Path`.
    pub fn path(&mut self, path_data: &str, options: Option<&Options>) -> Drawable {
        let surface = self.surface;
        self.with_options(options, |o| {
            let mut sets = Vec::new();
            if path_data.is_empty() {
                return drawable("path", sets, o);
            }
            let path_data = normalize_path_data(path_data);
            let outline = svg_path(&path_data, o);
            let has_fill = !o.fill.is_empty() && o.fill != "transparent" && o.fill != NO_STYLE;
            if has_fill {
                if o.fill_style == "solid" {
                    let path_points = RoughPath::new(&path_data).linear_points();
                    if path_points.len() == 1 {
                        let mut fill_options = o.deep_clone();
                        fill_options.disable_multi_stroke = true;
                        if fill_options.roughness != 0.0 {
                            fill_options.roughness += fill_options.fill_shape_roughness_gain;
                        }
                        let mut shape = svg_path(&path_data, &mut fill_options);
                        shape.kind = OpSetType::FillPath;
                        shape.ops = merged_shape(shape.ops);
                        sets.push(shape);
                    } else {
                        sets.push(solid_fill_polygons(&path_points, o));
                    }
                } else {
                    let size = compute_path_size(surface);
                    let mut points = vec![
                        [0.0, 0.0],
                        [size[0], 0.0],
                        [size[0], size[1]],
                        [0.0, size[1]],
                    ];
                    let mut shape = pattern_fill_polygon(&mut points, o);
                    shape.kind = OpSetType::Path2DPattern;
                    shape.size = Some(size);
                    shape.path = path_data.clone();
                    sets.push(shape);
                }
            }
            if o.stroke != NO_STYLE {
                sets.push(outline);
            }
            drawable("path", sets, o)
        })
    }

    /// generator.go `OpsToPath`.
    pub fn ops_to_path(&self, drawing: &OpSet) -> String {
        ops_to_path(drawing, None)
    }

    /// generator.go `ToPaths`.
    pub fn to_paths(&self, d: &Drawable) -> Vec<PathInfo> {
        let o = d.options.as_ref().unwrap_or(&self.default_options);
        let mut paths = Vec::with_capacity(d.sets.len());
        for drawing in &d.sets {
            let fixed = o.fixed_decimal_place_digits;
            let p = match drawing.kind {
                OpSetType::Path => PathInfo {
                    d: ops_to_path(drawing, fixed),
                    stroke: o.stroke.clone(),
                    stroke_width: o.stroke_width,
                    fill: NO_STYLE.into(),
                    pattern: None,
                },
                OpSetType::FillPath => PathInfo {
                    d: ops_to_path(drawing, fixed),
                    stroke: NO_STYLE.into(),
                    stroke_width: 0.0,
                    fill: if o.fill.is_empty() {
                        NO_STYLE.into()
                    } else {
                        o.fill.clone()
                    },
                    pattern: None,
                },
                OpSetType::FillSketch => fill_sketch(drawing, o),
                OpSetType::Path2DFill => PathInfo {
                    d: drawing.path.clone(),
                    stroke: NO_STYLE.into(),
                    stroke_width: 0.0,
                    fill: if o.fill.is_empty() {
                        NO_STYLE.into()
                    } else {
                        o.fill.clone()
                    },
                    pattern: None,
                },
                OpSetType::Path2DPattern => {
                    let Some(size) = drawing.size else { continue };
                    let pattern = PatternInfo {
                        x: 0.0,
                        y: 0.0,
                        width: 1.0,
                        height: 1.0,
                        view_box: format!(
                            "0 0 {} {}",
                            js_number_string(js_round(size[0])),
                            js_number_string(js_round(size[1]))
                        ),
                        pattern_units: "objectBoundingBox".into(),
                        path: fill_sketch(drawing, o),
                    };
                    PathInfo {
                        d: drawing.path.clone(),
                        stroke: NO_STYLE.into(),
                        stroke_width: 0.0,
                        fill: String::new(),
                        pattern: Some(Box::new(pattern)),
                    }
                }
            };
            paths.push(p);
        }
        paths
    }
}

/// generator.go `computePathSize`.
fn compute_path_size(surface: Option<Surface>) -> RoughPoint {
    match surface {
        Some(s) => [
            if s.width == 0.0 { 100.0 } else { s.width },
            if s.height == 0.0 { 100.0 } else { s.height },
        ],
        None => [100.0, 100.0],
    }
}

/// generator.go `opsToPath`.
fn ops_to_path(drawing: &OpSet, fixed_decimals: Option<f64>) -> String {
    let mut b = String::new();
    for item in &drawing.ops {
        let mut d = item.data.clone();
        if let Some(digits) = fixed_decimals.filter(|digits| *digits >= 0.0) {
            for v in &mut d {
                *v = js_to_fixed_number(*v, digits as i64);
            }
        }
        let n = |i: usize| js_number_string(d[i]);
        match item.op {
            OpType::Move => b += &format!("M{} {} ", n(0), n(1)),
            OpType::BCurveTo => {
                b += &format!("C{} {}, {} {}, {} {} ", n(0), n(1), n(2), n(3), n(4), n(5))
            }
            OpType::LineTo => b += &format!("L{} {} ", n(0), n(1)),
            OpType::QCurveTo => {}
        }
    }
    b.trim().to_owned()
}

/// generator.go `fillSketch`.
fn fill_sketch(drawing: &OpSet, o: &ResolvedOptions) -> PathInfo {
    let weight = if o.fill_weight < 0.0 {
        o.stroke_width / 2.0
    } else {
        o.fill_weight
    };
    PathInfo {
        d: ops_to_path(drawing, o.fixed_decimal_place_digits),
        stroke: if o.fill.is_empty() {
            NO_STYLE.into()
        } else {
            o.fill.clone()
        },
        stroke_width: weight,
        fill: NO_STYLE.into(),
        pattern: None,
    }
}

/// ECMAScript whitespace accepted after a minus by renderer.go `pathMinusSpaceRE`.
fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// generator.go `normalizePathData`.
fn normalize_path_data(path: &str) -> String {
    let path = path.replace('\n', " ");
    let mut out = String::with_capacity(path.len());
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if c == '-' && chars.peek().is_some_and(|&next| is_js_space(next)) {
            chars.next();
        }
    }
    out.replacen("/(ss)/g", " ", 1)
}

/// generator.go `mergedShape`.
fn merged_shape(input: Vec<Op>) -> Vec<Op> {
    input
        .into_iter()
        .enumerate()
        .filter(|(i, op)| *i == 0 || op.op != OpType::Move)
        .map(|(_, op)| op)
        .collect()
}

// ---------------------------------------------------------------- number.go

/// number.go `jsToFixedNumber`.
fn js_to_fixed_number(v: f64, digits: i64) -> f64 {
    if digits < 0 || !v.is_finite() {
        return v;
    }
    format!("{v:.*}", digits as usize).parse().unwrap_or(v)
}

/// number.go `jsNumberString`: JavaScript Number-to-String coercion.
pub fn js_number_string(v: f64) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if v == 0.0 {
        return "0".into();
    }
    let abs = v.abs();
    if !(1e-6..1e21).contains(&abs) {
        // Rust's `{:e}` is the shortest round-trip mantissa; JavaScript spells
        // the exponent with an explicit sign and no padding.
        let s = format!("{v:e}");
        let (mantissa, exponent) = s.split_once('e').expect("exponent");
        return if exponent.starts_with('-') {
            format!("{mantissa}e{exponent}")
        } else {
            format!("{mantissa}e+{exponent}")
        };
    }
    // Shortest round-trip fixed notation, as strconv 'f' -1.
    format!("{v}")
}

// ------------------------------------------------------------------ math.go

/// math.go `random`.
fn random(o: &mut ResolvedOptions) -> f64 {
    let seed = o.seed;
    o.randomizer
        .get_or_insert_with(|| Rc::new(RefCell::new(RandomState::new(seed))))
        .borrow_mut()
        .next()
}

/// math.go `jsRound`.
pub(crate) fn js_round(v: f64) -> f64 {
    if !v.is_finite() || v == 0.0 || v.abs() >= 4_503_599_627_370_496.0 {
        return v;
    }
    if (-0.5..0.0).contains(&v) {
        return -0.0;
    }
    (v + 0.5).floor()
}

// ------------------------------------------------------------- renderer.go

#[derive(Clone, Copy, Debug)]
struct EllipseParams {
    rx: f64,
    ry: f64,
    increment: f64,
}

struct EllipseResult {
    opset: OpSet,
    estimated_points: Vec<RoughPoint>,
}

fn line(x1: f64, y1: f64, x2: f64, y2: f64, o: &mut ResolvedOptions) -> OpSet {
    opset(OpSetType::Path, double_line(x1, y1, x2, y2, o))
}

fn linear_path(points: &[RoughPoint], close: bool, o: &mut ResolvedOptions) -> OpSet {
    let length = points.len();
    if length > 2 {
        let mut ops = Vec::new();
        for i in 0..length - 1 {
            ops.extend(double_line(
                points[i][0],
                points[i][1],
                points[i + 1][0],
                points[i + 1][1],
                o,
            ));
        }
        if close {
            ops.extend(double_line(
                points[length - 1][0],
                points[length - 1][1],
                points[0][0],
                points[0][1],
                o,
            ));
        }
        return opset(OpSetType::Path, ops);
    }
    if length == 2 {
        return line(points[0][0], points[0][1], points[1][0], points[1][1], o);
    }
    opset(OpSetType::Path, Vec::new())
}

fn rectangle(x: f64, y: f64, width: f64, height: f64, o: &mut ResolvedOptions) -> OpSet {
    linear_path(
        &[
            [x, y],
            [x + width, y],
            [x + width, y + height],
            [x, y + height],
        ],
        true,
        o,
    )
}

fn curve(points: &[RoughPoint], o: &mut ResolvedOptions) -> OpSet {
    // ARM64 renderer.go:75/80: `1+o.Roughness*0.2` contracts.
    let mut o1 = curve_with_offset(points, fmadd(o.roughness, 0.2, 1.0), o);
    if o.disable_multi_stroke {
        return opset(OpSetType::Path, o1);
    }
    let mut o2_options = clone_options_alter_seed(o);
    let o2 = curve_with_offset(points, 1.5 * fmadd(o.roughness, 0.22, 1.0), &mut o2_options);
    o1.extend(o2);
    opset(OpSetType::Path, o1)
}

fn ellipse(x: f64, y: f64, width: f64, height: f64, o: &mut ResolvedOptions) -> OpSet {
    let params = generate_ellipse_params(width, height, o);
    ellipse_with_params(x, y, o, params).opset
}

fn generate_ellipse_params(width: f64, height: f64, o: &mut ResolvedOptions) -> EllipseParams {
    let psq = (std::f64::consts::PI
        * 2.0
        * (((width / 2.0).powi(2) + (height / 2.0).powi(2)) / 2.0).sqrt())
    .sqrt();
    let step_count = gomath::max(
        o.curve_step_count,
        (o.curve_step_count / 200f64.sqrt()) * psq,
    )
    .ceil();
    let increment = (std::f64::consts::PI * 2.0) / step_count;
    let mut rx = (width / 2.0).abs();
    let mut ry = (height / 2.0).abs();
    let curve_fit_randomness = 1.0 - o.curve_fitting;
    rx += offset_opt(rx * curve_fit_randomness, o, 1.0);
    ry += offset_opt(ry * curve_fit_randomness, o, 1.0);
    EllipseParams { increment, rx, ry }
}

fn ellipse_with_params(x: f64, y: f64, o: &mut ResolvedOptions, p: EllipseParams) -> EllipseResult {
    let inner = offset(0.4, 1.0, o, 1.0);
    let overlap = p.increment * offset(0.1, inner, o, 1.0);
    let (ap1, cp1) = compute_ellipse_points(p.increment, x, y, p.rx, p.ry, 1.0, overlap, o);
    let mut o1 = rough_curve(&ap1, None, o);
    if !o.disable_multi_stroke && o.roughness != 0.0 {
        let (ap2, _) = compute_ellipse_points(p.increment, x, y, p.rx, p.ry, 1.5, 0.0, o);
        o1.extend(rough_curve(&ap2, None, o));
    }
    EllipseResult {
        estimated_points: cp1,
        opset: opset(OpSetType::Path, o1),
    }
}

#[allow(clippy::too_many_arguments)]
fn arc(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    start: f64,
    stop: f64,
    closed: bool,
    rough_closure: bool,
    o: &mut ResolvedOptions,
) -> OpSet {
    let (cx, cy) = (x, y);
    let mut rx = (width / 2.0).abs();
    let mut ry = (height / 2.0).abs();
    rx += offset_opt(rx * 0.01, o, 1.0);
    ry += offset_opt(ry * 0.01, o, 1.0);
    let (mut strt, mut stp) = (start, stop);
    while strt < 0.0 {
        strt += std::f64::consts::PI * 2.0;
        stp += std::f64::consts::PI * 2.0;
    }
    if stp - strt > std::f64::consts::PI * 2.0 {
        strt = 0.0;
        stp = std::f64::consts::PI * 2.0;
    }
    let ellipse_inc = (std::f64::consts::PI * 2.0) / o.curve_step_count;
    let arc_inc = gomath::min(ellipse_inc / 2.0, (stp - strt) / 2.0);
    let mut ops = rough_arc(arc_inc, cx, cy, rx, ry, strt, stp, 1.0, o);
    if !o.disable_multi_stroke {
        ops.extend(rough_arc(arc_inc, cx, cy, rx, ry, strt, stp, 1.5, o));
    }
    if closed {
        // ARM64 renderer.go:147/152/158: `cx+rx*cos` contracts.
        if rough_closure {
            ops.extend(double_line(
                cx,
                cy,
                fmadd(rx, gomath::cos(strt), cx),
                fmadd(ry, gomath::sin(strt), cy),
                o,
            ));
            ops.extend(double_line(
                cx,
                cy,
                fmadd(rx, gomath::cos(stp), cx),
                fmadd(ry, gomath::sin(stp), cy),
                o,
            ));
        } else {
            ops.push(self::op(OpType::LineTo, vec![cx, cy]));
            ops.push(self::op(
                OpType::LineTo,
                vec![
                    fmadd(rx, gomath::cos(strt), cx),
                    fmadd(ry, gomath::sin(strt), cy),
                ],
            ));
        }
    }
    opset(OpSetType::Path, ops)
}

/// renderer.go `svgPath`.
fn svg_path(path: &str, o: &mut ResolvedOptions) -> OpSet {
    let mut p = ParsedPath::new(path);
    if js_truthy_number(o.simplification) {
        let rp = RoughPath::new(path);
        let fitter = PathFitter {
            sets: rp.linear_points(),
            closed: rp.parsed.closed(),
        };
        p = ParsedPath::new(&fitter.fit(o.simplification));
    }
    let mut ops = Vec::new();
    let mut current: RoughPoint = [0.0, 0.0];
    let mut first: RoughPoint = [0.0, 0.0];
    let mut has_first = false;
    let mut previous_cubic_control: Option<RoughPoint> = None;
    let mut previous_quadratic_control: Option<RoughPoint> = None;
    let mut previous_key = String::new();
    for segment in &p.segments {
        let key = segment.key.as_str();
        let relative = key.chars().next().is_some_and(|c| c.is_ascii_lowercase());
        let upper = key.to_ascii_uppercase();
        let data = &segment.data;
        let absolute_point = |current: RoughPoint, x: f64, y: f64| -> RoughPoint {
            if relative {
                [x + current[0], y + current[1]]
            } else {
                [x, y]
            }
        };
        match upper.as_str() {
            "M" => {
                current = absolute_point(current, data[0], data[1]);
                first = current;
                has_first = true;
            }
            "L" => {
                let to = absolute_point(current, data[0], data[1]);
                ops.extend(double_line(current[0], current[1], to[0], to[1], o));
                current = to;
            }
            "H" => {
                let mut x = data[0];
                if relative {
                    x += current[0];
                }
                let to = [x, current[1]];
                ops.extend(double_line(current[0], current[1], to[0], to[1], o));
                current = to;
            }
            "V" => {
                let mut y = data[0];
                if relative {
                    y += current[1];
                }
                let to = [current[0], y];
                ops.extend(double_line(current[0], current[1], to[0], to[1], o));
                current = to;
            }
            "C" => {
                let c1 = absolute_point(current, data[0], data[1]);
                let c2 = absolute_point(current, data[2], data[3]);
                let to = absolute_point(current, data[4], data[5]);
                ops.extend(bezier_to_current(c1, c2, to, current, o));
                current = to;
                previous_cubic_control = Some(c2);
            }
            "S" => {
                let mut c1 = current;
                if previous_key == "C" || previous_key == "S" {
                    if let Some(prev) = previous_cubic_control {
                        c1 = [2.0 * current[0] - prev[0], 2.0 * current[1] - prev[1]];
                    }
                }
                let c2 = absolute_point(current, data[0], data[1]);
                let to = absolute_point(current, data[2], data[3]);
                ops.extend(bezier_to_current(c1, c2, to, current, o));
                current = to;
                previous_cubic_control = Some(c2);
            }
            "Q" | "T" => {
                let (q, to) = if upper == "Q" {
                    (
                        absolute_point(current, data[0], data[1]),
                        absolute_point(current, data[2], data[3]),
                    )
                } else {
                    let mut q = current;
                    if previous_key == "Q" || previous_key == "T" {
                        if let Some(prev) = previous_quadratic_control {
                            q = [2.0 * current[0] - prev[0], 2.0 * current[1] - prev[1]];
                        }
                    }
                    (q, absolute_point(current, data[0], data[1]))
                };
                let c1 = [
                    current[0] + (q[0] - current[0]) * 2.0 / 3.0,
                    current[1] + (q[1] - current[1]) * 2.0 / 3.0,
                ];
                let c2 = [
                    to[0] + (q[0] - to[0]) * 2.0 / 3.0,
                    to[1] + (q[1] - to[1]) * 2.0 / 3.0,
                ];
                ops.extend(bezier_to_current(c1, c2, to, current, o));
                current = to;
                previous_quadratic_control = Some(q);
            }
            "A" => {
                let to = absolute_point(current, data[5], data[6]);
                if to != current {
                    if data[0] == 0.0 || data[1] == 0.0 {
                        ops.extend(bezier_to_current(current, to, to, current, o));
                    } else {
                        let mut from = current;
                        for c in arc_to_cubic_curves(
                            current[0],
                            current[1],
                            to[0],
                            to[1],
                            data[0].abs(),
                            data[1].abs(),
                            data[2],
                            data[3],
                            data[4],
                            None,
                        ) {
                            ops.extend(bezier_to_current(
                                [c[0], c[1]],
                                [c[2], c[3]],
                                [c[4], c[5]],
                                from,
                                o,
                            ));
                            from = [c[4], c[5]];
                        }
                    }
                }
                current = to;
            }
            "Z" if has_first => {
                ops.extend(double_line(current[0], current[1], first[0], first[1], o));
                current = first;
            }
            _ => {}
        }
        if upper != "C" && upper != "S" {
            previous_cubic_control = None;
        }
        if upper != "Q" && upper != "T" {
            previous_quadratic_control = None;
        }
        previous_key = upper;
    }
    opset(OpSetType::Path, ops)
}

/// renderer.go `arcToCubicCurves` (path-data-parser).
#[allow(clippy::too_many_arguments)]
fn arc_to_cubic_curves(
    mut x1: f64,
    mut y1: f64,
    mut x2: f64,
    mut y2: f64,
    mut r1: f64,
    mut r2: f64,
    angle: f64,
    large_arc_flag: f64,
    sweep_flag: f64,
    recursive: Option<[f64; 4]>,
) -> Vec<[f64; 6]> {
    use std::f64::consts::PI;
    let angle_rad = PI * angle / 180.0;
    let mut params: Vec<RoughPoint> = Vec::new();
    let (mut f1, mut f2, cx, cy);
    if let Some(r) = recursive {
        (f1, f2, cx, cy) = (r[0], r[1], r[2], r[3]);
    } else {
        (x1, y1) = rotate_path_point(x1, y1, -angle_rad);
        (x2, y2) = rotate_path_point(x2, y2, -angle_rad);
        let x = (x1 - x2) / 2.0;
        let y = (y1 - y2) / 2.0;
        let mut h = x * x / (r1 * r1) + y * y / (r2 * r2);
        if h > 1.0 {
            h = h.sqrt();
            r1 *= h;
            r2 *= h;
        }
        let sign = if large_arc_flag == sweep_flag {
            -1.0
        } else {
            1.0
        };
        let (r1p, r2p) = (r1 * r1, r2 * r2);
        // ARM64 renderer.go:313-314: each trailing product contracts.
        let left = fmsub(r2p * x, x, fmsub(r1p * y, y, r1p * r2p));
        let right = fmadd(r2p * x, x, r1p * y * y);
        let k = sign * (left / right).abs().sqrt();
        // ARM64 :316-317 contract `(x1+x2)/2` (an exact *0.5) into the add.
        cx = k * r1 * y / r2 + (x1 + x2) / 2.0;
        cy = k * -r2 * x / r1 + (y1 + y2) / 2.0;
        f1 = gomath::asin(js_to_fixed_number((y1 - cy) / r2, 9));
        f2 = gomath::asin(js_to_fixed_number((y2 - cy) / r2, 9));
        if x1 < cx {
            f1 = PI - f1;
        }
        if x2 < cx {
            f2 = PI - f2;
        }
        if f1 < 0.0 {
            f1 += PI * 2.0;
        }
        if f2 < 0.0 {
            f2 += PI * 2.0;
        }
        if sweep_flag != 0.0 && f1 > f2 {
            f1 -= PI * 2.0;
        }
        if sweep_flag == 0.0 && f2 > f1 {
            f2 -= PI * 2.0;
        }
    }
    let mut df = f2 - f1;
    if df.abs() > PI * 120.0 / 180.0 {
        let (f2old, x2old, y2old) = (f2, x2, y2);
        if sweep_flag != 0.0 && f2 > f1 {
            f2 = f1 + PI * 120.0 / 180.0;
        } else {
            f2 = f1 - PI * 120.0 / 180.0;
        }
        // ARM64 renderer.go:347-348.
        x2 = fmadd(r1, gomath::cos(f2), cx);
        y2 = fmadd(r2, gomath::sin(f2), cy);
        for c in arc_to_cubic_curves(
            x2,
            y2,
            x2old,
            y2old,
            r1,
            r2,
            angle,
            0.0,
            sweep_flag,
            Some([f2, f2old, cx, cy]),
        ) {
            params.extend([[c[0], c[1]], [c[2], c[3]], [c[4], c[5]]]);
        }
    }
    df = f2 - f1;
    let (c1, s1, c2, s2) = (
        gomath::cos(f1),
        gomath::sin(f1),
        gomath::cos(f2),
        gomath::sin(f2),
    );
    let t = gomath::tan(df / 4.0);
    let (hx, hy) = (4.0 / 3.0 * r1 * t, 4.0 / 3.0 * r2 * t);
    let m1 = [x1, y1];
    // ARM64 renderer.go:358-359.
    let mut m2 = [fmadd(hx, s1, x1), fmsub(hy, c1, y1)];
    let m3 = [fmadd(hx, s2, x2), fmsub(hy, c2, y2)];
    let m4 = [x2, y2];
    m2 = [2.0 * m1[0] - m2[0], 2.0 * m1[1] - m2[1]];
    let mut all = vec![m2, m3, m4];
    all.extend(params);
    let params = all;
    let mut curves = Vec::with_capacity(params.len() / 3);
    let mut i = 0;
    while i + 2 < params.len() {
        if recursive.is_some() {
            curves.push([
                params[i][0],
                params[i][1],
                params[i + 1][0],
                params[i + 1][1],
                params[i + 2][0],
                params[i + 2][1],
            ]);
        } else {
            let (a, b) = rotate_path_point(params[i][0], params[i][1], angle_rad);
            let (c, d) = rotate_path_point(params[i + 1][0], params[i + 1][1], angle_rad);
            let (e, f) = rotate_path_point(params[i + 2][0], params[i + 2][1], angle_rad);
            curves.push([a, b, c, d, e, f]);
        }
        i += 3;
    }
    curves
}

/// renderer.go `rotatePathPoint` (ARM64:381 contracts both trailing products).
fn rotate_path_point(x: f64, y: f64, angle: f64) -> (f64, f64) {
    (
        fmsub(y, gomath::sin(angle), x * gomath::cos(angle)),
        fmadd(y, gomath::cos(angle), x * gomath::sin(angle)),
    )
}

/// renderer.go `bezierToCurrent`.
fn bezier_to_current(
    c1: RoughPoint,
    c2: RoughPoint,
    to: RoughPoint,
    current: RoughPoint,
    o: &mut ResolvedOptions,
) -> Vec<Op> {
    let ros0 = js_or(o.max_randomness_offset, 1.0);
    let ros = [ros0, ros0 + 0.3];
    let iterations = if o.disable_multi_stroke { 1 } else { 2 };
    let mut ops = Vec::with_capacity(iterations * 2);
    for (i, &radius) in ros.iter().enumerate().take(iterations) {
        let mut mv = current;
        if i != 0 && !o.preserve_vertices {
            let x = current[0] + offset_opt(ros[0], o, 1.0);
            let y = current[1] + offset_opt(ros[0], o, 1.0);
            mv = [x, y];
        }
        let mut end = to;
        if !o.preserve_vertices {
            let x = to[0] + offset_opt(radius, o, 1.0);
            let y = to[1] + offset_opt(radius, o, 1.0);
            end = [x, y];
        }
        ops.push(op(OpType::Move, vec![mv[0], mv[1]]));
        let a = c1[0] + offset_opt(radius, o, 1.0);
        let b = c1[1] + offset_opt(radius, o, 1.0);
        let c = c2[0] + offset_opt(radius, o, 1.0);
        let d = c2[1] + offset_opt(radius, o, 1.0);
        ops.push(op(OpType::BCurveTo, vec![a, b, c, d, end[0], end[1]]));
    }
    ops
}

/// renderer.go `solidFillPolygons`.
fn solid_fill_polygons(polygon_list: &[Vec<RoughPoint>], o: &mut ResolvedOptions) -> OpSet {
    let mut ops = Vec::new();
    for points in polygon_list {
        let rand_offset = js_or(o.max_randomness_offset, 0.0);
        if points.len() > 2 {
            let x = points[0][0] + offset_opt(rand_offset, o, 1.0);
            let y = points[0][1] + offset_opt(rand_offset, o, 1.0);
            ops.push(op(OpType::Move, vec![x, y]));
            for p in &points[1..] {
                let x = p[0] + offset_opt(rand_offset, o, 1.0);
                let y = p[1] + offset_opt(rand_offset, o, 1.0);
                ops.push(op(OpType::LineTo, vec![x, y]));
            }
        }
    }
    opset(OpSetType::FillPath, ops)
}

/// renderer.go `patternFillPolygon` / filler.go `getFiller`.
fn pattern_fill_polygon(points: &mut [RoughPoint], o: &mut ResolvedOptions) -> OpSet {
    match o.fill_style.as_str() {
        "zigzag" => zigzag_fill_polygon(points, o),
        "cross-hatch" => hatch_fill_polygon(points, o),
        "dots" => dot_fill_polygon(points, o),
        "dashed" => dashed_fill_polygon(points, o),
        "zigzag-line" => zigzag_line_fill_polygon(points, o),
        _ => hachure_fill_polygon(points, o, false),
    }
}

#[allow(clippy::too_many_arguments)]
fn pattern_fill_arc(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    start: f64,
    stop: f64,
    o: &mut ResolvedOptions,
) -> OpSet {
    let (cx, cy) = (x, y);
    let mut rx = (width / 2.0).abs();
    let mut ry = (height / 2.0).abs();
    rx += offset_opt(rx * 0.01, o, 1.0);
    ry += offset_opt(ry * 0.01, o, 1.0);
    let (mut strt, mut stp) = (start, stop);
    while strt < 0.0 {
        strt += std::f64::consts::PI * 2.0;
        stp += std::f64::consts::PI * 2.0;
    }
    if stp - strt > std::f64::consts::PI * 2.0 {
        strt = 0.0;
        stp = std::f64::consts::PI * 2.0;
    }
    let increment = (stp - strt) / o.curve_step_count;
    let mut points = Vec::new();
    let mut angle = strt;
    // ARM64 renderer.go:459/462.
    while angle <= stp {
        points.push([
            fmadd(rx, gomath::cos(angle), cx),
            fmadd(ry, gomath::sin(angle), cy),
        ]);
        angle += increment;
    }
    points.push([
        fmadd(rx, gomath::cos(stp), cx),
        fmadd(ry, gomath::sin(stp), cy),
    ]);
    points.push([cx, cy]);
    pattern_fill_polygon(&mut points, o)
}

/// renderer.go `offset`: `Roughness * gain * (random*(max-min) + min)`
/// (ARM64:485 contracts the inner sum).
fn offset(minimum: f64, maximum: f64, o: &mut ResolvedOptions, roughness_gain: f64) -> f64 {
    let scale = o.roughness * roughness_gain;
    scale * fmadd(random(o), maximum - minimum, minimum)
}

/// renderer.go `offsetOpt`.
fn offset_opt(x: f64, o: &mut ResolvedOptions, roughness_gain: f64) -> f64 {
    offset(-x, x, o, roughness_gain)
}

fn double_line(x1: f64, y1: f64, x2: f64, y2: f64, o: &mut ResolvedOptions) -> Vec<Op> {
    double_line_mode(x1, y1, x2, y2, o, false)
}

/// renderer.go `doubleLineFill` (the filler helper's `doubleLineOps`).
fn double_line_fill(x1: f64, y1: f64, x2: f64, y2: f64, o: &mut ResolvedOptions) -> Vec<Op> {
    double_line_mode(x1, y1, x2, y2, o, true)
}

fn double_line_mode(
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    o: &mut ResolvedOptions,
    filling: bool,
) -> Vec<Op> {
    let mut o1 = rough_line(x1, y1, x2, y2, o, true, false);
    if (!filling && o.disable_multi_stroke) || (filling && o.disable_multi_stroke_fill) {
        return o1;
    }
    o1.extend(rough_line(x1, y1, x2, y2, o, true, true));
    o1
}

/// renderer.go `roughLine`.
fn rough_line(
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    o: &mut ResolvedOptions,
    mv: bool,
    overlay: bool,
) -> Vec<Op> {
    let length_sq = (x1 - x2).powi(2) + (y1 - y2).powi(2);
    let length = length_sq.sqrt();
    let roughness_gain = if length < 200.0 {
        1.0
    } else if length > 500.0 {
        0.4
    } else {
        // jsNumberStep keeps the product separately rounded.
        (-0.0016668 * length) + 1.233334
    };
    let mut rand_offset = js_or(o.max_randomness_offset, 0.0);
    if rand_offset * rand_offset * 100.0 > length_sq {
        rand_offset = length / 10.0;
    }
    let half_offset = rand_offset / 2.0;
    // ARM64 renderer.go:530.
    let diverge_point = fmadd(random(o), 0.2, 0.2);
    let mut mid_disp_x = o.bowing * o.max_randomness_offset * (y2 - y1) / 200.0;
    let mut mid_disp_y = o.bowing * o.max_randomness_offset * (x1 - x2) / 200.0;
    mid_disp_x = offset_opt(mid_disp_x, o, roughness_gain);
    mid_disp_y = offset_opt(mid_disp_y, o, roughness_gain);
    let radius = if overlay { half_offset } else { rand_offset };
    let preserve = o.preserve_vertices;
    let mut ops = Vec::with_capacity(2);
    if mv {
        let x = x1
            + if preserve {
                0.0
            } else {
                offset_opt(radius, o, roughness_gain)
            };
        let y = y1
            + if preserve {
                0.0
            } else {
                offset_opt(radius, o, roughness_gain)
            };
        ops.push(op(OpType::Move, vec![x, y]));
    }
    // ARM64 renderer.go:554-566: `midDisp + x1 + d*divergePoint` contracts
    // the product into the preceding sum; the random offset is added after.
    let a = fmadd(x2 - x1, diverge_point, mid_disp_x + x1) + offset_opt(radius, o, roughness_gain);
    let b = fmadd(y2 - y1, diverge_point, mid_disp_y + y1) + offset_opt(radius, o, roughness_gain);
    let c = fmadd(2.0 * (x2 - x1), diverge_point, mid_disp_x + x1)
        + offset_opt(radius, o, roughness_gain);
    let d = fmadd(2.0 * (y2 - y1), diverge_point, mid_disp_y + y1)
        + offset_opt(radius, o, roughness_gain);
    let e = x2
        + if preserve {
            0.0
        } else {
            offset_opt(radius, o, roughness_gain)
        };
    let f = y2
        + if preserve {
            0.0
        } else {
            offset_opt(radius, o, roughness_gain)
        };
    ops.push(op(OpType::BCurveTo, vec![a, b, c, d, e, f]));
    ops
}

fn curve_with_offset(points: &[RoughPoint], rand_offset: f64, o: &mut ResolvedOptions) -> Vec<Op> {
    if points.is_empty() {
        return Vec::new();
    }
    let mut ps = Vec::with_capacity(points.len() + 2);
    for _ in 0..2 {
        let x = points[0][0] + offset_opt(rand_offset, o, 1.0);
        let y = points[0][1] + offset_opt(rand_offset, o, 1.0);
        ps.push([x, y]);
    }
    for i in 1..points.len() {
        let x = points[i][0] + offset_opt(rand_offset, o, 1.0);
        let y = points[i][1] + offset_opt(rand_offset, o, 1.0);
        ps.push([x, y]);
        if i == points.len() - 1 {
            let x = points[i][0] + offset_opt(rand_offset, o, 1.0);
            let y = points[i][1] + offset_opt(rand_offset, o, 1.0);
            ps.push([x, y]);
        }
    }
    rough_curve(&ps, None, o)
}

/// renderer.go `roughCurve` (ARM64:607-612 contract `s*a - s*b`).
fn rough_curve(
    points: &[RoughPoint],
    close_point: Option<RoughPoint>,
    o: &mut ResolvedOptions,
) -> Vec<Op> {
    let length = points.len();
    let mut ops = Vec::new();
    if length > 3 {
        let s = 1.0 - o.curve_tightness;
        ops.push(op(OpType::Move, vec![points[1][0], points[1][1]]));
        let mut i = 1;
        while i + 2 < length {
            let cached = points[i];
            let b1 = [
                cached[0] + fmsub(s, points[i - 1][0], s * points[i + 1][0]) / 6.0,
                cached[1] + fmsub(s, points[i - 1][1], s * points[i + 1][1]) / 6.0,
            ];
            let b2 = [
                points[i + 1][0] + fmsub(s, points[i + 2][0], s * points[i][0]) / 6.0,
                points[i + 1][1] + fmsub(s, points[i + 2][1], s * points[i][1]) / 6.0,
            ];
            let b3 = points[i + 1];
            ops.push(op(
                OpType::BCurveTo,
                vec![b1[0], b1[1], b2[0], b2[1], b3[0], b3[1]],
            ));
            i += 1;
        }
        if let Some(close_point) = close_point {
            let ro = o.max_randomness_offset;
            let x = close_point[0] + offset_opt(ro, o, 1.0);
            let y = close_point[1] + offset_opt(ro, o, 1.0);
            ops.push(op(OpType::LineTo, vec![x, y]));
        }
    } else if length == 3 {
        ops.push(op(OpType::Move, vec![points[1][0], points[1][1]]));
        ops.push(op(
            OpType::BCurveTo,
            vec![
                points[1][0],
                points[1][1],
                points[2][0],
                points[2][1],
                points[2][0],
                points[2][1],
            ],
        ));
    } else if length == 2 {
        ops.extend(double_line(
            points[0][0],
            points[0][1],
            points[1][0],
            points[1][1],
            o,
        ));
    }
    ops
}

/// renderer.go `computeEllipsePoints`. ARM64 (:650-687) contracts the
/// trailing `r*cos/sin` product into the preceding `offset + c` sum.
#[allow(clippy::too_many_arguments)]
fn compute_ellipse_points(
    mut increment: f64,
    cx: f64,
    cy: f64,
    rx: f64,
    ry: f64,
    rand_offset: f64,
    overlap: f64,
    o: &mut ResolvedOptions,
) -> (Vec<RoughPoint>, Vec<RoughPoint>) {
    use std::f64::consts::PI;
    let mut core_points = Vec::new();
    let mut all_points = Vec::new();
    if o.roughness == 0.0 {
        increment /= 4.0;
        all_points.push([
            fmadd(rx, gomath::cos(-increment), cx),
            fmadd(ry, gomath::sin(-increment), cy),
        ]);
        let mut angle = 0.0;
        while angle <= PI * 2.0 {
            let p = [
                fmadd(rx, gomath::cos(angle), cx),
                fmadd(ry, gomath::sin(angle), cy),
            ];
            core_points.push(p);
            all_points.push(p);
            angle += increment;
        }
        all_points.push([cx + rx, cy]);
        all_points.push([
            fmadd(rx, gomath::cos(increment), cx),
            fmadd(ry, gomath::sin(increment), cy),
        ]);
        return (all_points, core_points);
    }
    let rad_offset = offset_opt(0.5, o, 1.0) - PI / 2.0;
    let point = |o: &mut ResolvedOptions, kx: f64, ky: f64, angle: f64| -> RoughPoint {
        let x = offset_opt(rand_offset, o, 1.0) + cx;
        let x = fmadd(kx, gomath::cos(angle), x);
        let y = offset_opt(rand_offset, o, 1.0) + cy;
        let y = fmadd(ky, gomath::sin(angle), y);
        [x, y]
    };
    all_points.push(point(o, 0.9 * rx, 0.9 * ry, rad_offset - increment));
    let mut angle = rad_offset;
    while angle < PI * 2.0 + rad_offset - 0.01 {
        let p = point(o, rx, ry, angle);
        core_points.push(p);
        all_points.push(p);
        angle += increment;
    }
    all_points.push(point(o, rx, ry, rad_offset + PI * 2.0 + overlap * 0.5));
    all_points.push(point(o, 0.98 * rx, 0.98 * ry, rad_offset + overlap));
    all_points.push(point(o, 0.9 * rx, 0.9 * ry, rad_offset + overlap * 0.5));
    (all_points, core_points)
}

/// renderer.go `roughArc` (ARM64:697-706 as in computeEllipsePoints).
#[allow(clippy::too_many_arguments)]
fn rough_arc(
    increment: f64,
    cx: f64,
    cy: f64,
    rx: f64,
    ry: f64,
    strt: f64,
    stp: f64,
    rand_offset: f64,
    o: &mut ResolvedOptions,
) -> Vec<Op> {
    let rad_offset = strt + offset_opt(0.1, o, 1.0);
    let mut points = Vec::new();
    let point = |o: &mut ResolvedOptions, kx: f64, ky: f64, angle: f64| -> RoughPoint {
        let x = offset_opt(rand_offset, o, 1.0) + cx;
        let x = fmadd(kx, gomath::cos(angle), x);
        let y = offset_opt(rand_offset, o, 1.0) + cy;
        let y = fmadd(ky, gomath::sin(angle), y);
        [x, y]
    };
    points.push(point(o, 0.9 * rx, 0.9 * ry, rad_offset - increment));
    let mut angle = rad_offset;
    while angle <= stp {
        points.push(point(o, rx, ry, angle));
        angle += increment;
    }
    let end = [
        fmadd(rx, gomath::cos(stp), cx),
        fmadd(ry, gomath::sin(stp), cy),
    ];
    points.push(end);
    points.push(end);
    rough_curve(&points, None, o)
}

/// renderer.go `cloneOptionsAlterSeed`.
fn clone_options_alter_seed(options: &ResolvedOptions) -> ResolvedOptions {
    let mut copy = options.deep_clone();
    copy.randomizer = None;
    if options.seed != 0.0 {
        copy.seed = options.seed + 1.0;
    }
    copy
}

fn js_truthy_number(v: f64) -> bool {
    v != 0.0 && !v.is_nan()
}

fn js_or(v: f64, fallback: f64) -> f64 {
    if js_truthy_number(v) { v } else { fallback }
}

// ----------------------------------------------------------------- path.go

#[derive(Clone, Debug)]
enum PathToken {
    Command(char),
    Number(f64),
}

#[derive(Clone, Debug)]
struct PathSegment {
    key: String,
    data: Vec<f64>,
    point: Option<RoughPoint>,
}

fn path_params(key: &str) -> Option<usize> {
    Some(match key {
        "A" | "a" => 7,
        "C" | "c" => 6,
        "H" | "h" | "V" | "v" => 1,
        "L" | "l" | "M" | "m" | "T" | "t" => 2,
        "Q" | "q" | "S" | "s" => 4,
        "Z" | "z" => 0,
        _ => return None,
    })
}

/// path.go `pathNumberRE`: `[-+]?(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)(?:[eE][-+]?[0-9]+)?`.
fn match_number(s: &[u8]) -> usize {
    let digits = |from: usize| s[from..].iter().take_while(|b| b.is_ascii_digit()).count();
    let mut i = 0;
    if matches!(s.first(), Some(b'-' | b'+')) {
        i = 1;
    }
    let integer = digits(i);
    if integer > 0 {
        i += integer;
        if s.get(i) == Some(&b'.') {
            i += 1 + digits(i + 1);
        }
    } else if s.get(i) == Some(&b'.') && digits(i + 1) > 0 {
        i += 1 + digits(i + 1);
    } else {
        return 0;
    }
    if matches!(s.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(s.get(j), Some(b'-' | b'+')) {
            j += 1;
        }
        let exponent = digits(j);
        if exponent > 0 {
            i = j + exponent;
        }
    }
    i
}

/// path.go `parsedPath`.
#[derive(Clone, Debug, Default)]
struct ParsedPath {
    segments: Vec<PathSegment>,
}

impl ParsedPath {
    fn new(d: &str) -> Self {
        let mut p = ParsedPath::default();
        p.parse_data(d);
        p.process_points();
        p
    }

    /// path.go `tokenize`; `None` is the source's nil (an unrecognized byte).
    fn tokenize(d: &str) -> Option<Vec<PathToken>> {
        let bytes = d.as_bytes();
        let mut i = 0;
        let mut tokens = Vec::new();
        while i < bytes.len() {
            let b = bytes[i];
            if matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b',') {
                i += 1;
            } else if b"aAcChHlLmMqQsStTvVzZ".contains(&b) {
                tokens.push(PathToken::Command(b as char));
                i += 1;
            } else {
                let n = match_number(&bytes[i..]);
                if n == 0 {
                    return None;
                }
                // parseFloat, then `${...}` and unary plus: -0 becomes +0.
                let v: f64 = d[i..i + n].parse().unwrap_or(f64::NAN);
                tokens.push(PathToken::Number(if v == 0.0 { 0.0 } else { v }));
                i += n;
            }
        }
        Some(tokens)
    }

    /// path.go `parseData`. Tokens end with an implicit end-of-data marker.
    fn parse_data(&mut self, d: &str) {
        let Some(tokens) = Self::tokenize(d) else {
            self.segments = Vec::new();
            return;
        };
        let token_count = tokens.len() + 1;
        let token_at = |index: usize| tokens.get(index);
        let mut index = 0;
        let mut mode = String::from("BOD");
        self.segments = Vec::new();
        while let Some(token) = token_at(index) {
            let param_length;
            if mode == "BOD" {
                match token {
                    PathToken::Command(c @ ('M' | 'm')) => {
                        index += 1;
                        mode = c.to_string();
                        param_length = 2;
                    }
                    _ => {
                        self.parse_data(&format!("M0,0{d}"));
                        return;
                    }
                }
            } else if let PathToken::Number(_) = token {
                param_length = path_params(&mode).unwrap_or(0);
                if param_length == 0 {
                    // Rough.js loops forever on a number after Z; stop.
                    return;
                }
            } else if let PathToken::Command(c) = token {
                index += 1;
                let Some(length) = path_params(&c.to_string()) else {
                    return;
                };
                param_length = length;
                mode = c.to_string();
            } else {
                unreachable!()
            }
            if index + param_length < token_count {
                let mut params = Vec::with_capacity(param_length);
                for i in index..index + param_length {
                    match token_at(i) {
                        Some(PathToken::Number(v)) => params.push(*v),
                        _ => return,
                    }
                }
                self.segments.push(PathSegment {
                    key: mode.clone(),
                    data: params,
                    point: None,
                });
                index += param_length;
                if mode == "M" {
                    mode = "L".into();
                }
                if mode == "m" {
                    mode = "l".into();
                }
            } else {
                return;
            }
        }
    }

    fn closed(&self) -> bool {
        self.segments
            .iter()
            .any(|s| s.key.eq_ignore_ascii_case("z"))
    }

    /// path.go `processPoints`.
    fn process_points(&mut self) {
        let mut first: Option<RoughPoint> = None;
        let mut current: RoughPoint = [0.0, 0.0];
        for segment in &mut self.segments {
            let d = &segment.data;
            segment.point = match segment.key.as_str() {
                "M" | "L" | "T" => Some([d[0], d[1]]),
                "m" | "l" | "t" => Some([d[0] + current[0], d[1] + current[1]]),
                "H" => Some([d[0], current[1]]),
                "h" => Some([d[0] + current[0], current[1]]),
                "V" => Some([current[0], d[0]]),
                "v" => Some([current[0], d[0] + current[1]]),
                "z" | "Z" => first,
                "C" => Some([d[4], d[5]]),
                "c" => Some([d[4] + current[0], d[5] + current[1]]),
                "S" | "Q" => Some([d[2], d[3]]),
                "s" | "q" => Some([d[2] + current[0], d[3] + current[1]]),
                "A" => Some([d[5], d[6]]),
                "a" => Some([d[5] + current[0], d[6] + current[1]]),
                _ => None,
            };
            if segment.key == "m" || segment.key == "M" {
                first = None;
            }
            if let Some(point) = segment.point {
                current = point;
                if first.is_none() {
                    first = Some(point);
                }
            }
            if segment.key == "z" || segment.key == "Z" {
                first = None;
            }
        }
    }
}

/// path.go `roughPath` (the parts reachable from the generator).
struct RoughPath {
    parsed: ParsedPath,
}

impl RoughPath {
    fn new(d: &str) -> Self {
        Self {
            parsed: ParsedPath::new(d),
        }
    }

    /// path.go `linearPoints`.
    fn linear_points(&self) -> Vec<Vec<RoughPoint>> {
        let mut linear_points = Vec::new();
        let mut points = Vec::new();
        for segment in &self.parsed.segments {
            let key = segment.key.to_ascii_lowercase();
            if key == "m" || key == "z" {
                if !points.is_empty() {
                    linear_points.push(std::mem::take(&mut points));
                }
                if key == "z" {
                    continue;
                }
            }
            if let Some(point) = segment.point {
                points.push(point);
            }
        }
        if !points.is_empty() {
            linear_points.push(points);
        }
        linear_points
    }
}

/// path.go `pathFitter`.
struct PathFitter {
    sets: Vec<Vec<RoughPoint>>,
    closed: bool,
}

impl PathFitter {
    fn fit(&self, simplification: f64) -> String {
        let mut out_sets = Vec::new();
        for set in &self.sets {
            let length = set.len();
            let mut est_length = (simplification * length as f64).floor();
            if est_length < 5.0 {
                if length <= 5 {
                    continue;
                }
                est_length = 5.0;
            }
            out_sets.push(Self::reduce(set, est_length));
        }
        let mut d = String::new();
        for set in &out_sets {
            for (i, point) in set.iter().enumerate() {
                d.push(if i == 0 { 'M' } else { 'L' });
                d += &js_number_string(point[0]);
                d.push(',');
                d += &js_number_string(point[1]);
            }
            if self.closed {
                d += "z ";
            }
        }
        d
    }

    fn reduce(set: &[RoughPoint], count: f64) -> Vec<RoughPoint> {
        if set.len() as f64 <= count {
            return set.to_vec();
        }
        let length =
            |a: RoughPoint, b: RoughPoint| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
        let mut points = set.to_vec();
        while points.len() as f64 > count {
            let mut min_area = -1.0;
            let mut min_index = 0;
            for i in 1..points.len() - 1 {
                let a = length(points[i - 1], points[i]);
                let b = length(points[i], points[i + 1]);
                let c = length(points[i - 1], points[i + 1]);
                let s = (a + b + c) / 2.0;
                let area = (s * (s - a) * (s - b) * (s - c)).sqrt();
                if min_area < 0.0 || area < min_area {
                    min_area = area;
                    min_index = i;
                }
            }
            if min_index > 0 {
                points.remove(min_index);
            } else {
                break;
            }
        }
        points
    }
}

// ------------------------------------------------- geometry.go and fillers

/// geometry.go `rotatePoints`: jsNumberStep keeps every product rounded.
fn rotate_points(points: &mut [RoughPoint], center: RoughPoint, degrees: f64) {
    if points.is_empty() {
        return;
    }
    let (cx, cy) = (center[0], center[1]);
    let angle = (std::f64::consts::PI / 180.0) * degrees;
    let (cos, sin) = (gomath::cos(angle), gomath::sin(angle));
    for p in points {
        let (x, y) = (p[0], p[1]);
        let x_cos = (x - cx) * cos;
        let y_sin = (y - cy) * sin;
        let x_sin = (x - cx) * sin;
        let y_cos = (y - cy) * cos;
        p[0] = x_cos - y_sin + cx;
        p[1] = x_sin + y_cos + cy;
    }
}

fn rotate_lines(lines: &mut [RoughLine], center: RoughPoint, degrees: f64) {
    let mut points: Vec<RoughPoint> = lines.iter().flat_map(|l| [l[0], l[1]]).collect();
    rotate_points(&mut points, center, degrees);
    for (i, line) in lines.iter_mut().enumerate() {
        *line = [points[i * 2], points[i * 2 + 1]];
    }
}

fn line_length(line: &RoughLine) -> f64 {
    ((line[0][0] - line[1][0]).powi(2) + (line[0][1] - line[1][1]).powi(2)).sqrt()
}

#[derive(Clone, Copy, Debug)]
struct HachureEdge {
    ymin: f64,
    ymax: f64,
    x: f64,
    islope: f64,
}

const COLLAPSED_ENDPOINT_MAX_ULPS: f64 = 8.0;

/// filler_scan_line_hachure.go `polygonHachureLines`. Like the source, the
/// caller's points are rotated in place and back (an observable round trip).
fn polygon_hachure_lines(points: &mut [RoughPoint], o: &mut ResolvedOptions) -> Vec<RoughLine> {
    let rotation_center = [0.0, 0.0];
    let angle = o.hachure_angle + 90.0;
    let mut gap = o.hachure_gap;
    if gap < 0.0 {
        gap = o.stroke_width * 4.0;
    }
    let gap = js_round(gomath::max(gap, 0.1));
    let mut skip_offset = 1.0;
    if o.roughness >= 1.0 && random(o) > 0.7 {
        skip_offset = gap;
    }
    let mut scan_points = points.to_vec();
    if angle != 0.0 {
        let unrotated = points.to_vec();
        rotate_points(points, rotation_center, angle);
        scan_points = points.to_vec();
        stabilize_collapsed_rotated_ordinates(&unrotated, &mut scan_points, angle);
    }
    let mut lines = straight_hachure_lines_modern(&scan_points, gap, skip_offset);
    if angle != 0.0 {
        rotate_points(points, rotation_center, -angle);
        rotate_lines(&mut lines, rotation_center, -angle);
    }
    lines
}

fn hachure_less(a: &HachureEdge, b: &HachureEdge) -> bool {
    if a.ymin != b.ymin {
        return a.ymin < b.ymin;
    }
    if a.x != b.x {
        return a.x < b.x;
    }
    a.ymax < b.ymax
}

fn ordering_from_less<T>(less: impl Fn(&T, &T) -> bool) -> impl Fn(&T, &T) -> std::cmp::Ordering {
    move |a, b| {
        if less(a, b) {
            std::cmp::Ordering::Less
        } else if less(b, a) {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    }
}

/// filler_scan_line_hachure.go `straightHachureLinesModern`.
fn straight_hachure_lines_modern(
    points: &[RoughPoint],
    gap: f64,
    step_offset: f64,
) -> Vec<RoughLine> {
    let mut vertices = points.to_vec();
    if vertices.is_empty() {
        return Vec::new();
    }
    if !points_join_equal(vertices[0], vertices[vertices.len() - 1]) {
        vertices.push(vertices[0]);
    }
    let mut lines = Vec::new();
    if vertices.len() <= 2 {
        return lines;
    }
    let mut edges = Vec::with_capacity(vertices.len() - 1);
    for pair in vertices.windows(2) {
        let (p1, p2) = (pair[0], pair[1]);
        if p1[1] == p2[1] {
            continue;
        }
        let ymin = gomath::min(p1[1], p2[1]);
        edges.push(HachureEdge {
            ymin,
            ymax: gomath::max(p1[1], p2[1]),
            x: if ymin == p1[1] { p1[0] } else { p2[0] },
            islope: (p2[0] - p1[0]) / (p2[1] - p1[1]),
        });
    }
    edges.sort_by(ordering_from_less(hachure_less));
    if edges.is_empty() {
        return lines;
    }
    let mut edges = std::collections::VecDeque::from(edges);
    let mut active: Vec<HachureEdge> = Vec::new();
    let mut y = edges[0].ymin;
    let mut iteration = 0.0;
    while !active.is_empty() || !edges.is_empty() {
        let mut take = 0;
        for edge in &edges {
            if edge.ymin > y {
                break;
            }
            take += 1;
        }
        active.extend(edges.drain(..take));
        active.retain(|edge| edge.ymax > y);
        active.sort_by(ordering_from_less(|a: &HachureEdge, b: &HachureEdge| {
            a.x < b.x
        }));
        if step_offset != 1.0 || iteration % gap == 0.0 {
            let mut i = 0;
            while i + 1 < active.len() {
                lines.push([[js_round(active[i].x), y], [js_round(active[i + 1].x), y]]);
                i += 2;
            }
        }
        let next_y = y + step_offset;
        if next_y == y {
            // The source would never terminate; stop at a non-advancing scan.
            break;
        }
        y = next_y;
        // ARM64 filler_scan_line_hachure.go:131.
        for edge in &mut active {
            edge.x = fmadd(step_offset, edge.islope, edge.x);
        }
        iteration += 1.0;
    }
    lines
}

/// filler_scan_line_hachure.go `stabilizeCollapsedRotatedOrdinates`.
fn stabilize_collapsed_rotated_ordinates(
    unrotated: &[RoughPoint],
    rotated: &mut [RoughPoint],
    degrees: f64,
) {
    if unrotated.len() != rotated.len() || rotated.len() < 2 {
        return;
    }
    let angle = (std::f64::consts::PI / 180.0) * degrees;
    let (sin, cos) = (gomath::sin(angle), gomath::cos(angle));
    for i in 0..rotated.len() {
        let j = if i + 1 == rotated.len() { 0 } else { i + 1 };
        if rotated[i][1] != rotated[j][1]
            || points_join_equal(unrotated[i], unrotated[j])
            || !points_within_ulps(unrotated[i], unrotated[j], COLLAPSED_ENDPOINT_MAX_ULPS)
        {
            continue;
        }
        // ARM64 :157 contracts the second product.
        let delta_y = fmadd(
            unrotated[i][1] - unrotated[j][1],
            cos,
            (unrotated[i][0] - unrotated[j][0]) * sin,
        );
        if delta_y > 0.0 {
            rotated[i][1] = rotated[i][1].next_up();
        } else if delta_y < 0.0 {
            rotated[i][1] = rotated[i][1].next_down();
        }
    }
}

fn points_within_ulps(a: RoughPoint, b: RoughPoint, max_ulps: f64) -> bool {
    let mut scale = 1.0_f64;
    for axis in 0..2 {
        for coordinate in [a[axis], b[axis]] {
            if !coordinate.is_finite() {
                return false;
            }
            scale = gomath::max(scale, coordinate.abs());
        }
    }
    let tolerance = max_ulps * (scale.next_up() - scale);
    (0..2).all(|axis| (a[axis] - b[axis]).abs() <= tolerance)
}

fn points_join_equal(a: RoughPoint, b: RoughPoint) -> bool {
    (0..2).all(|i| a[i] == b[i] || (a[i].is_nan() && b[i].is_nan()))
}

/// filler_hachure.go `renderLines` with the doubleLineFill helper.
fn render_lines(lines: &[RoughLine], o: &mut ResolvedOptions, connect_ends: bool) -> Vec<Op> {
    let mut ops = Vec::new();
    let mut prev_point: Option<RoughPoint> = None;
    for line in lines {
        ops.extend(double_line_fill(
            line[0][0], line[0][1], line[1][0], line[1][1], o,
        ));
        if connect_ends {
            if let Some(prev) = prev_point {
                ops.extend(double_line_fill(
                    prev[0], prev[1], line[0][0], line[0][1], o,
                ));
            }
        }
        prev_point = Some(line[1]);
    }
    ops
}

/// filler_hachure.go `fillPolygonWithConnectEnds`.
fn hachure_fill_polygon(
    points: &mut [RoughPoint],
    o: &mut ResolvedOptions,
    connect_ends: bool,
) -> OpSet {
    let lines = polygon_hachure_lines(points, o);
    opset(OpSetType::FillSketch, render_lines(&lines, o, connect_ends))
}

/// filler_hatch.go.
fn hatch_fill_polygon(points: &mut [RoughPoint], o: &mut ResolvedOptions) -> OpSet {
    let mut set = hachure_fill_polygon(points, o, false);
    let mut o2 = o.clone();
    o2.hachure_angle = o.hachure_angle + 90.0;
    let set2 = hachure_fill_polygon(points, &mut o2, false);
    set.ops.extend(set2.ops);
    set
}

/// filler_zigzag.go. The Go struct copy shares the randomizer.
fn zigzag_fill_polygon(points: &mut [RoughPoint], o: &mut ResolvedOptions) -> OpSet {
    let mut gap = o.hachure_gap;
    if gap < 0.0 {
        gap = o.stroke_width * 4.0;
    }
    gap = gomath::max(gap, 0.1);
    let mut o2 = o.clone();
    o2.hachure_gap = gap;
    let lines = polygon_hachure_lines(points, &mut o2);
    let angle = std::f64::consts::PI / 180.0 * o.hachure_angle;
    let half_gap = gap * 0.5;
    let (cos, sin) = (gomath::cos(angle), gomath::sin(angle));
    let mut zigzag = Vec::with_capacity(lines.len() * 2);
    for line in &lines {
        let (p1, p2) = (line[0], line[1]);
        if line_length(line) == 0.0 {
            continue;
        }
        // ARM64 filler_zigzag.go:32-33 contract `gap*0.5*cos/sin`.
        zigzag.push([
            [fmsub(half_gap, cos, p1[0]), fmadd(half_gap, sin, p1[1])],
            p2,
        ]);
        zigzag.push([
            [fmadd(half_gap, cos, p1[0]), fmsub(half_gap, sin, p1[1])],
            p2,
        ]);
    }
    opset(OpSetType::FillSketch, render_lines(&zigzag, o, false))
}

/// filler_zigzag_line.go.
fn zigzag_line_fill_polygon(points: &mut [RoughPoint], o: &mut ResolvedOptions) -> OpSet {
    let mut gap = o.hachure_gap;
    if gap < 0.0 {
        gap = o.stroke_width * 4.0;
    }
    let mut zo = o.zigzag_offset;
    if zo < 0.0 {
        zo = gap;
    }
    let mut o2 = o.clone();
    o2.hachure_gap = gap + zo;
    let lines = polygon_hachure_lines(points, &mut o2);
    let mut ops = Vec::new();
    for line in &lines {
        let length = line_length(line);
        let count = js_round(length / (2.0 * zo));
        let (mut p1, mut p2) = (line[0], line[1]);
        if p1[0] > p2[0] {
            (p1, p2) = (line[1], line[0]);
        }
        let alpha = gomath::atan((p2[1] - p1[1]) / (p2[0] - p1[0]));
        let mut i = 0.0;
        while i < count {
            let lstart = i * 2.0 * zo;
            let lend = (i + 1.0) * 2.0 * zo;
            let dz = (2.0 * zo.powi(2)).sqrt();
            // ARM64 filler_zigzag_line.go:41-45.
            let start = [
                fmadd(lstart, gomath::cos(alpha), p1[0]),
                fmadd(lstart, gomath::sin(alpha), p1[1]),
            ];
            let end = [
                fmadd(lend, gomath::cos(alpha), p1[0]),
                fmadd(lend, gomath::sin(alpha), p1[1]),
            ];
            let middle = [
                fmadd(
                    dz,
                    gomath::cos(alpha + std::f64::consts::PI / 4.0),
                    start[0],
                ),
                fmadd(
                    dz,
                    gomath::sin(alpha + std::f64::consts::PI / 4.0),
                    start[1],
                ),
            ];
            ops.extend(double_line_fill(
                start[0], start[1], middle[0], middle[1], &mut o2,
            ));
            ops.extend(double_line_fill(
                middle[0], middle[1], end[0], end[1], &mut o2,
            ));
            i += 1.0;
        }
    }
    opset(OpSetType::FillSketch, ops)
}

/// filler_dot.go.
fn dot_fill_polygon(points: &mut [RoughPoint], o: &mut ResolvedOptions) -> OpSet {
    let mut o2 = o.clone();
    o2.curve_step_count = 4.0;
    o2.hachure_angle = 0.0;
    o2.roughness = 1.0;
    let lines = polygon_hachure_lines(points, &mut o2);
    let o = &mut o2;
    let mut ops = Vec::new();
    let mut gap = o.hachure_gap;
    if gap < 0.0 {
        gap = o.stroke_width * 4.0;
    }
    gap = gomath::max(gap, 0.1);
    let mut fweight = o.fill_weight;
    if fweight < 0.0 {
        fweight = o.stroke_width / 2.0;
    }
    for line in &lines {
        let length = line_length(line);
        let dl = length / gap;
        let count = dl.ceil() - 1.0;
        let alpha = gomath::atan((line[1][1] - line[0][1]) / (line[1][0] - line[0][0]));
        let mut i = 0.0;
        while i < count {
            let l = gap * (i + 1.0);
            let dy = l * gomath::sin(alpha);
            // ARM64 filler_dot.go:41 contracts `l*cos` into `line[0][0]-dx`.
            let c = [fmsub(l, gomath::cos(alpha), line[0][0]), line[0][1] + dy];
            let cx = offset(c[0] - gap / 4.0, c[0] + gap / 4.0, o, 1.0);
            let cy = offset(c[1] - gap / 4.0, c[1] + gap / 4.0, o, 1.0);
            ops.extend(ellipse(cx, cy, fweight, fweight, o).ops);
            i += 1.0;
        }
    }
    opset(OpSetType::FillSketch, ops)
}

/// filler_dashed.go.
fn dashed_fill_polygon(points: &mut [RoughPoint], o: &mut ResolvedOptions) -> OpSet {
    let lines = polygon_hachure_lines(points, o);
    let offset = if o.dash_offset < 0.0 {
        if o.hachure_gap < 0.0 {
            o.stroke_width * 4.0
        } else {
            o.hachure_gap
        }
    } else {
        o.dash_offset
    };
    let gap = if o.dash_gap < 0.0 {
        if o.hachure_gap < 0.0 {
            o.stroke_width * 4.0
        } else {
            o.hachure_gap
        }
    } else {
        o.dash_gap
    };
    let mut ops = Vec::new();
    for line in &lines {
        let length = line_length(line);
        let count = snap_one_ulp_to_integer(length / (offset + gap)).floor();
        // ARM64 filler_dashed.go:44 contracts `count*(offset+gap)`.
        let start_offset = fmsub(count, offset + gap, length + gap) / 2.0;
        let (mut p1, mut p2) = (line[0], line[1]);
        if p1[0] > p2[0] {
            (p1, p2) = (line[1], line[0]);
        }
        let alpha = gomath::atan((p2[1] - p1[1]) / (p2[0] - p1[0]));
        let (cos, sin) = (gomath::cos(alpha), gomath::sin(alpha));
        let mut i = 0.0;
        while i < count {
            let lstart = i * (offset + gap);
            // ARM64 :52 recomputes and contracts `i*(offset+gap) + offset`.
            let lend = fmadd(i, offset + gap, offset);
            // ARM64 :54-59.
            let start = [
                fmadd(start_offset, cos, fmadd(lstart, cos, p1[0])),
                fmadd(start_offset, sin, fmadd(lstart, sin, p1[1])),
            ];
            let end = [
                fmadd(start_offset, cos, fmadd(lend, cos, p1[0])),
                fmadd(start_offset, sin, fmadd(lend, sin, p1[1])),
            ];
            ops.extend(double_line_fill(start[0], start[1], end[0], end[1], o));
            i += 1.0;
        }
    }
    opset(OpSetType::FillSketch, ops)
}

fn snap_one_ulp_to_integer(v: f64) -> f64 {
    let nearest = v.round();
    let toward = if nearest > v {
        v.next_up()
    } else if nearest < v {
        v.next_down()
    } else {
        v
    };
    if v == nearest || toward == nearest {
        nearest
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_number_strings_follow_ecmascript() {
        assert_eq!(js_number_string(0.0), "0");
        assert_eq!(js_number_string(-0.0), "0");
        assert_eq!(js_number_string(1.5), "1.5");
        assert_eq!(js_number_string(1e21), "1e+21");
        assert_eq!(js_number_string(1.234e-7), "1.234e-7");
        assert_eq!(js_number_string(0.000001), "0.000001");
        assert_eq!(js_number_string(123456789.0), "123456789");
    }

    #[test]
    fn path_data_tokenizer_matches_path_data_parser() {
        let p = ParsedPath::new("M 1 2 L3,4 5 6 C 1 2 3 4 5 6 S 1 2 3 4 Z");
        let keys: Vec<_> = p.segments.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, ["M", "L", "L", "C", "S", "Z"]);
        assert_eq!(p.segments[2].data, [5.0, 6.0]);
        assert_eq!(p.segments[5].point, Some([1.0, 2.0]));
        // A missing leading moveto is prefixed with M0,0.
        assert_eq!(ParsedPath::new("L 1 1").segments[0].key, "M");
    }

    #[test]
    fn line_drawable_matches_legacy_line_path() {
        // The legacy helper was verified against rough-go for default options.
        let mut g = Generator::new(None, None);
        let d = g.line(
            0.0,
            0.0,
            100.0,
            0.0,
            Some(&Options {
                seed: Some(1.0),
                ..Default::default()
            }),
        );
        let paths = g.to_paths(&d);
        assert_eq!(paths.len(), 1);
        let expected = super::super::rough_line_path(
            d2_rs_core::Point { x: 0.0, y: 0.0 },
            d2_rs_core::Point { x: 100.0, y: 0.0 },
            super::super::SketchOptions::default(),
        );
        assert_eq!(paths[0].d, expected);
        assert_eq!(paths[0].fill, "none");
        assert_eq!(paths[0].stroke, "#000");
    }
}

// SPDX-License-Identifier: MPL-2.0
//! d2raster/animation_value.go: validated keyframe sampling at
//! FrameOptions.Time.

use d2_rs_scene::{
    AnimationProperty, AnimationValue, AnimationValueKind, DropShadow, Easing, EasingKind, Matrix,
    Nrgba, Track, go_max, go_min,
};

use crate::fma::{finite, go_fmadd};

pub fn animation_value_at(track: &Track, time: i64) -> Result<AnimationValue, String> {
    if track.target_index < 0 {
        return Err("d2scene: animation target index is negative".into());
    }
    if track.property.0 > AnimationProperty::DROP_SHADOW.0 {
        return Err(format!(
            "d2scene: unknown animation property {}",
            track.property.0
        ));
    }
    if track.delay < 0 {
        return Err("d2scene: animation delay is negative".into());
    }
    if track.duration <= 0 {
        return Err("d2scene: animation duration must be positive".into());
    }
    let keyframes = track.keyframes.as_deref().unwrap_or_default();
    if keyframes.is_empty() {
        return Err("d2scene: animation has no keyframes".into());
    }
    let kind = keyframes[0].value.kind;
    let expected_kind = animation_kind_for_property(track.property);
    if kind != expected_kind {
        return Err(format!(
            "d2scene: animation property {} requires value kind {}",
            track.property.0, expected_kind.0
        ));
    }
    let mut previous = -1.0;
    for (index, keyframe) in keyframes.iter().enumerate() {
        if !finite(keyframe.offset)
            || keyframe.offset < 0.0
            || keyframe.offset > 1.0
            || keyframe.offset < previous
        {
            return Err(format!(
                "d2scene: animation keyframe {index} has invalid offset"
            ));
        }
        if keyframe.value.kind != kind {
            return Err(format!(
                "d2scene: animation keyframe {index} changes value kind"
            ));
        }
        validate_animation_value(&keyframe.value)
            .map_err(|e| format!("d2scene: animation keyframe {index}: {e}"))?;
        validate_animation_easing(&keyframe.easing)
            .map_err(|e| format!("d2scene: animation keyframe {index}: {e}"))?;
        previous = keyframe.offset;
    }
    let first = &keyframes[0];
    let last = &keyframes[keyframes.len() - 1];
    if time <= track.delay {
        return Ok(first.value);
    }
    let mut local = time - track.delay;
    if !track.repeat && local >= track.duration {
        return Ok(last.value);
    }
    if track.repeat {
        local %= track.duration;
    }
    let progress = local as f64 / track.duration as f64;
    if progress <= first.offset {
        return Ok(first.value);
    }
    // sort.Search(len-1, keyframes[i+1].Offset >= progress) + 1.
    let n = keyframes.len() - 1;
    let right_index = (0..n)
        .find(|&index| keyframes[index + 1].offset >= progress)
        .unwrap_or(n)
        + 1;
    if right_index >= keyframes.len() {
        return Ok(last.value);
    }
    let right = &keyframes[right_index];
    let left = &keyframes[right_index - 1];
    let span = right.offset - left.offset;
    if span == 0.0 {
        return Ok(right.value);
    }
    let position = apply_animation_easing(&left.easing, (progress - left.offset) / span);
    interpolate_renderer_animation_value(&left.value, &right.value, position)
}

fn animation_kind_for_property(property: AnimationProperty) -> AnimationValueKind {
    match property {
        AnimationProperty::OPACITY | AnimationProperty::STROKE_DASH_OFFSET => {
            AnimationValueKind::NUMBER
        }
        AnimationProperty::TRANSFORM => AnimationValueKind::TRANSFORM,
        AnimationProperty::FILL_COLOR | AnimationProperty::STROKE_COLOR => {
            AnimationValueKind::COLOR
        }
        AnimationProperty::DROP_SHADOW => AnimationValueKind::SHADOW,
        _ => AnimationValueKind::NUMBER,
    }
}

fn validate_animation_value(value: &AnimationValue) -> Result<(), String> {
    match value.kind {
        AnimationValueKind::NUMBER => {
            if !finite(value.number) {
                return Err("non-finite animation number".into());
            }
        }
        AnimationValueKind::TRANSFORM => {
            if !value.transform.is_finite() {
                return Err("non-finite animation transform".into());
            }
        }
        AnimationValueKind::COLOR => {}
        AnimationValueKind::SHADOW => {
            let s = value.shadow;
            if s.sigma_x < 0.0
                || s.sigma_y < 0.0
                || !finite(s.sigma_x)
                || !finite(s.sigma_y)
                || !finite(s.offset_x)
                || !finite(s.offset_y)
            {
                return Err("invalid animation shadow".into());
            }
        }
        kind => return Err(format!("unknown animation value kind {}", kind.0)),
    }
    Ok(())
}

fn validate_animation_easing(easing: &Easing) -> Result<(), String> {
    match easing.kind {
        EasingKind::LINEAR | EasingKind::STEP_START | EasingKind::STEP_END => Ok(()),
        EasingKind::CUBIC_BEZIER => {
            if !finite(easing.x1)
                || !finite(easing.y1)
                || !finite(easing.x2)
                || !finite(easing.y2)
                || easing.x1 < 0.0
                || easing.x1 > 1.0
                || easing.x2 < 0.0
                || easing.x2 > 1.0
            {
                return Err("invalid cubic-bezier easing".into());
            }
            Ok(())
        }
        kind => Err(format!("unknown easing kind {}", kind.0)),
    }
}

fn apply_animation_easing(easing: &Easing, value: f64) -> f64 {
    let value = go_max(0.0, go_min(1.0, value));
    match easing.kind {
        EasingKind::STEP_START => {
            if value == 0.0 {
                0.0
            } else {
                1.0
            }
        }
        EasingKind::STEP_END => {
            if value == 1.0 {
                1.0
            } else {
                0.0
            }
        }
        EasingKind::CUBIC_BEZIER => {
            renderer_cubic_bezier_y_for_x(value, easing.x1, easing.y1, easing.x2, easing.y2)
        }
        _ => value,
    }
}

fn renderer_cubic_bezier_y_for_x(x: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    let (mut low, mut high) = (0.0, 1.0);
    let mut parameter = x;
    for _ in 0..30 {
        parameter = (low + high) / 2.0;
        let estimate = renderer_cubic_scalar(0.0, x1, x2, 1.0, parameter);
        if estimate < x {
            low = parameter;
        } else {
            high = parameter;
        }
    }
    renderer_cubic_scalar(0.0, y1, y2, 1.0, parameter)
}

fn renderer_cubic_scalar(p0: f64, p1: f64, p2: f64, p3: f64, value: f64) -> f64 {
    let u = 1.0 - value;
    // ARM64 animation_value.go:186: three FMADDD onto the rounded first term.
    let sum = go_fmadd(3.0 * u * u * value, p1, u * u * u * p0);
    let sum = go_fmadd(3.0 * u * value * value, p2, sum);
    go_fmadd(value * value * value, p3, sum)
}

fn interpolate_renderer_animation_value(
    left: &AnimationValue,
    right: &AnimationValue,
    value: f64,
) -> Result<AnimationValue, String> {
    if left.kind != right.kind {
        return Err("d2scene: cannot interpolate different animation value kinds".into());
    }
    // ARM64 animation_value.go:193.
    let lerp = |a: f64, b: f64| go_fmadd(b - a, value, a);
    let mut result = AnimationValue {
        kind: left.kind,
        ..Default::default()
    };
    match left.kind {
        AnimationValueKind::NUMBER => result.number = lerp(left.number, right.number),
        AnimationValueKind::TRANSFORM => {
            let (l, r) = (left.transform, right.transform);
            result.transform = Matrix {
                a: lerp(l.a, r.a),
                b: lerp(l.b, r.b),
                c: lerp(l.c, r.c),
                d: lerp(l.d, r.d),
                e: lerp(l.e, r.e),
                f: lerp(l.f, r.f),
            };
        }
        AnimationValueKind::COLOR => {
            result.color = interpolate_renderer_animation_color(left.color, right.color, value)
        }
        AnimationValueKind::SHADOW => {
            let (l, r) = (left.shadow, right.shadow);
            result.shadow = DropShadow {
                offset_x: lerp(l.offset_x, r.offset_x),
                offset_y: lerp(l.offset_y, r.offset_y),
                sigma_x: lerp(l.sigma_x, r.sigma_x),
                sigma_y: lerp(l.sigma_y, r.sigma_y),
                color: interpolate_renderer_animation_color(l.color, r.color, value),
            };
        }
        kind => return Err(format!("d2scene: unknown animation value kind {}", kind.0)),
    }
    Ok(result)
}

fn interpolate_renderer_animation_color(left: Nrgba, right: Nrgba, value: f64) -> Nrgba {
    // ARM64 animation_value.go:223.
    let channel = |a: u8, b: u8| go_fmadd(b as f64 - a as f64, value, a as f64).round() as u8;
    Nrgba::new(
        channel(left.r, right.r),
        channel(left.g, right.g),
        channel(left.b, right.b),
        channel(left.a, right.a),
    )
}

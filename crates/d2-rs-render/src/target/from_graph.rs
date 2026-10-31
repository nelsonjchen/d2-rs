// SPDX-License-Identifier: MPL-2.0
//! Internal source exporter handoff. Pinned d2exporter/export.go:22–76 passes
//! typed records to d2target.Diagram; JSON decoding belongs to raw-target APIs.
use super::*;

fn float(value: crate::GoFloat) -> GoFloat {
    // The old JSON boundary encoded nonfinite internal floats as null, which
    // leaves the decoder's zero-valued scalar. Public target validation remains.
    GoFloat(if value.0.is_finite() { value.0 } else { 0.0 })
}

fn url(value: crate::HashUrl) -> HashUrl {
    HashUrl {
        scheme: value.scheme,
        opaque: value.opaque,
        user: value.user.filter(|v| !v.is_null()).map(|_| UrlUser {}),
        host: value.host,
        path: value.path,
        raw_path: value.raw_path,
        omit_host: value.omit_host,
        force_query: value.force_query,
        raw_query: value.raw_query,
        fragment: value.fragment,
        raw_fragment: value.raw_fragment,
    }
}

fn text(value: SourceSqlText) -> TargetText {
    TargetText {
        label: value.label,
        font_size: value.font_size.into(),
        font_family: value.font_family,
        language: value.language,
        color: value.color,
        italic: value.italic,
        bold: value.bold,
        underline: value.underline,
        label_width: value.label_width.into(),
        label_height: value.label_height.into(),
        ..Default::default()
    }
}

fn shape(value: crate::HashShape) -> TargetShape {
    let accents = value.target_accents.unwrap_or_default();
    TargetShape {
        id: value.id,
        shape_type: value.shape_type,
        classes: if value.classes.is_empty() {
            GoSlice::default()
        } else {
            value.classes.into()
        },
        pos: HashIntPoint {
            x: value.pos.x,
            y: value.pos.y,
        },
        width: value.width,
        height: value.height,
        opacity: float(value.opacity),
        stroke_dash: float(value.stroke_dash),
        stroke_width: value.stroke_width,
        border_radius: value.border_radius,
        fill: value.fill,
        fill_pattern: value.fill_pattern,
        stroke: value.stroke,
        animated: value.animated,
        shadow: value.shadow,
        three_dee: value.three_dee,
        multiple: value.multiple,
        double_border: value.double_border,
        tooltip: value.tooltip,
        link: value.link,
        pretty_link: value.pretty_link,
        icon: value.icon.map(url),
        icon_border_radius: value.icon_border_radius,
        icon_position: value.icon_position,
        blend: value.blend,
        fields: value.fields.map(|fields| {
            fields
                .into_iter()
                .map(|f| TargetClassField {
                    name: f.name,
                    type_text: f.type_text,
                    visibility: f.visibility,
                    underline: f.underline,
                })
                .collect::<Vec<_>>()
                .into()
        }),
        methods: value.methods.map(|methods| {
            methods
                .into_iter()
                .map(|m| TargetClassMethod {
                    name: m.name,
                    return_type: m.return_type,
                    visibility: m.visibility,
                    underline: m.underline,
                })
                .collect::<Vec<_>>()
                .into()
        }),
        columns: value.columns.map(|columns| {
            columns
                .into_iter()
                .map(|c| TargetSqlColumn {
                    name: text(c.name),
                    type_text: text(c.type_text),
                    constraint: c.constraint.map(GoSlice::from),
                    reference: c.reference,
                })
                .collect::<Vec<_>>()
                .into()
        }),
        content_aspect_ratio: value
            .content_aspect_ratio
            .filter(|v| v.0.is_finite())
            .map(float),
        label: value.label,
        font_size: value.font_size,
        font_family: value.font_family,
        language: value.language,
        color: value.color,
        italic: value.italic,
        bold: value.bold,
        underline: value.underline,
        label_width: value.label_width,
        label_height: value.label_height,
        label_fill: value.label_fill,
        label_position: value.label_position,
        tooltip_position: value.tooltip_position,
        z_index: value.z_index,
        level: value.level,
        primary_accent_color: accents.primary_accent_color,
        secondary_accent_color: accents.secondary_accent_color,
        neutral_accent_color: accents.neutral_accent_color,
    }
}

fn connection(value: crate::HashConnection) -> TargetConnection {
    TargetConnection {
        id: value.id,
        classes: if value.classes.is_empty() {
            GoSlice::default()
        } else {
            value.classes.into()
        },
        src: value.src,
        src_arrow: value.src_arrow,
        src_label: value.src_label.map(text),
        dst: value.dst,
        dst_arrow: value.dst_arrow,
        dst_label: value.dst_label.map(text),
        opacity: float(value.opacity),
        stroke_dash: float(value.stroke_dash),
        stroke_width: value.stroke_width,
        stroke: value.stroke,
        fill: value.fill,
        border_radius: float(value.border_radius),
        label: value.label,
        font_size: value.font_size,
        font_family: value.font_family,
        language: value.language,
        color: value.color,
        italic: value.italic,
        bold: value.bold,
        underline: value.underline,
        label_width: value.label_width,
        label_height: value.label_height,
        label_position: value.label_position,
        label_percentage: float(value.label_percentage),
        link: value.link,
        route: Some(
            value
                .route
                .into_iter()
                .map(|p| {
                    Some(HashPoint {
                        x: float(p.x),
                        y: float(p.y),
                    })
                })
                .collect::<Vec<_>>()
                .into(),
        ),
        is_curve: value.is_curve,
        animated: value.animated,
        tooltip: value.tooltip,
        icon: value.icon.map(url),
        icon_position: value.icon_position,
        icon_border_radius: float(value.icon_border_radius),
        z_index: value.z_index,
        ..Default::default()
    }
}

pub(crate) fn export(
    graph: &Graph,
    result: &LayoutResult,
    labels: &BTreeMap<NodeId, String>,
    styles: &BTreeMap<NodeId, SourceNodeStyle>,
    options: &RenderOptions,
    sketch: bool,
) -> Result<TargetDiagram, String> {
    let legend = match graph.legend.as_ref() {
        None => None,
        Some(legend) => match legend.source_graph.as_deref() {
            Some(source) => crate::target_legend_metadata(source, options.theme_id)
                .map_err(|e| e.to_string())?
                .map(|value| serde_json::from_slice(&serde_json::to_vec(&value)?))
                .transpose()
                .map_err(|e| e.to_string())?,
            None => return Err("d2svg target export: legend without its source graph".into()),
        },
    };
    let shapes = crate::target_node_ids(graph)
        .into_iter()
        .filter_map(|id| crate::target_hash_shape(graph, result, labels, styles, id))
        .map(shape)
        .collect::<Vec<_>>();
    let mut connections = graph
        .edges()
        .filter_map(|(id, _)| crate::target_hash_connection(graph, result, id))
        .map(connection)
        .collect::<Vec<_>>();
    connections.extend(
        crate::target_hash_lifelines(graph, result, styles)
            .into_iter()
            .map(connection),
    );
    let root = shape(crate::target_hash_root(graph, options));
    let config = if options.include_config_in_hash {
        // Config data intentionally retains encoding/json's untyped float64
        // interpretation. It is one small record, outside the bulk shape path.
        let value = crate::HashConfig {
            sketch,
            theme_id: options.theme_id.unwrap_or(0),
            dark_theme_id: options.dark_theme_id,
            pad: graph.d2_config_pad,
            center: graph.d2_config_center,
            layout_engine: graph.d2_config_layout_engine.clone(),
            theme_overrides: crate::HashThemeOverrides::from_map(&options.theme_overrides),
            dark_theme_overrides: crate::HashThemeOverrides::from_map(
                &options.dark_theme_overrides,
            ),
            data: graph.d2_config_data.clone(),
        };
        Some(
            serde_json::from_slice(&serde_json::to_vec(&value).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    let font_family = if crate::target_theme_rules(graph).mono {
        "SourceCodePro"
    } else if sketch || options.hand_drawn_font {
        "HandDrawn"
    } else {
        "SourceSansPro"
    };
    Ok(TargetDiagram {
        config,
        font_family: Some(font_family.into()),
        mono_font_family: Some("SourceCodePro".into()),
        shapes: Some(shapes.into()),
        connections: Some(connections.into()),
        root,
        legend,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_null_floats_and_empty_slices_retain_json_semantics() {
        let mut value = crate::target_hash_root(&Graph::default(), &RenderOptions::default());
        value.opacity = crate::GoFloat(f64::NAN);
        value.stroke_dash = crate::GoFloat(-0.0);
        value.content_aspect_ratio = Some(crate::GoFloat(f64::INFINITY));
        value.fields = Some(Vec::new());
        let target = shape(value);
        assert_eq!(target.opacity.0.to_bits(), 0.0_f64.to_bits());
        assert_eq!(target.stroke_dash.0.to_bits(), (-0.0_f64).to_bits());
        assert!(target.content_aspect_ratio.is_none());
        // Omitted classes stay nil; explicitly allocated empty members stay [].
        let json = serde_json::to_value(target).unwrap();
        assert!(json.get("classes").is_none());
        assert_eq!(json["fields"], serde_json::json!([]));
        assert!(json.get("contentAspectRatio").is_none());
    }
}

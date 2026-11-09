// SPDX-License-Identifier: MPL-2.0
// Field declarations from pinned d2graph/d2graph.go, d2ast/d2ast.go,
// d2target/d2target.go, lib/geo and Go net/url. See THIRD_PARTY_NOTICES.md.
[
    ("*d2ast.BlockString".into(), Type {
        kind: "ptr".into(),
        elem: "d2ast.BlockString".into(),
        fields: vec![
        ],
    }),
    ("*d2ast.DoubleQuotedString".into(), Type {
        kind: "ptr".into(),
        elem: "d2ast.DoubleQuotedString".into(),
        fields: vec![
        ],
    }),
    ("*d2ast.KeyPath".into(), Type {
        kind: "ptr".into(),
        elem: "d2ast.KeyPath".into(),
        fields: vec![
        ],
    }),
    ("*d2ast.SingleQuotedString".into(), Type {
        kind: "ptr".into(),
        elem: "d2ast.SingleQuotedString".into(),
        fields: vec![
        ],
    }),
    ("*d2ast.StringBox".into(), Type {
        kind: "ptr".into(),
        elem: "d2ast.StringBox".into(),
        fields: vec![
        ],
    }),
    ("*d2ast.Substitution".into(), Type {
        kind: "ptr".into(),
        elem: "d2ast.Substitution".into(),
        fields: vec![
        ],
    }),
    ("*d2ast.UnquotedString".into(), Type {
        kind: "ptr".into(),
        elem: "d2ast.UnquotedString".into(),
        fields: vec![
        ],
    }),
    ("*d2graph.Attributes".into(), Type {
        kind: "ptr".into(),
        elem: "d2graph.Attributes".into(),
        fields: vec![
        ],
    }),
    ("*d2graph.Scalar".into(), Type {
        kind: "ptr".into(),
        elem: "d2graph.Scalar".into(),
        fields: vec![
        ],
    }),
    ("*d2target.Class".into(), Type {
        kind: "ptr".into(),
        elem: "d2target.Class".into(),
        fields: vec![
        ],
    }),
    ("*d2target.SQLTable".into(), Type {
        kind: "ptr".into(),
        elem: "d2target.SQLTable".into(),
        fields: vec![
        ],
    }),
    ("*float64".into(), Type {
        kind: "ptr".into(),
        elem: "float64".into(),
        fields: vec![
        ],
    }),
    ("*geo.Box".into(), Type {
        kind: "ptr".into(),
        elem: "geo.Box".into(),
        fields: vec![
        ],
    }),
    ("*geo.Point".into(), Type {
        kind: "ptr".into(),
        elem: "geo.Point".into(),
        fields: vec![
        ],
    }),
    ("*int".into(), Type {
        kind: "ptr".into(),
        elem: "int".into(),
        fields: vec![
        ],
    }),
    ("*string".into(), Type {
        kind: "ptr".into(),
        elem: "string".into(),
        fields: vec![
        ],
    }),
    ("*url.URL".into(), Type {
        kind: "ptr".into(),
        elem: "url.URL".into(),
        fields: vec![
        ],
    }),
    ("*url.Userinfo".into(), Type {
        kind: "ptr".into(),
        elem: "url.Userinfo".into(),
        fields: vec![
        ],
    }),
    ("[]*d2ast.StringBox".into(), Type {
        kind: "slice".into(),
        elem: "*d2ast.StringBox".into(),
        fields: vec![
        ],
    }),
    ("[]*geo.Point".into(), Type {
        kind: "slice".into(),
        elem: "*geo.Point".into(),
        fields: vec![
        ],
    }),
    ("[]d2ast.InterpolationBox".into(), Type {
        kind: "slice".into(),
        elem: "d2ast.InterpolationBox".into(),
        fields: vec![
        ],
    }),
    ("[]d2graph.EdgeReference".into(), Type {
        kind: "slice".into(),
        elem: "d2graph.EdgeReference".into(),
        fields: vec![
        ],
    }),
    ("[]d2graph.Reference".into(), Type {
        kind: "slice".into(),
        elem: "d2graph.Reference".into(),
        fields: vec![
        ],
    }),
    ("[]d2target.ClassField".into(), Type {
        kind: "slice".into(),
        elem: "d2target.ClassField".into(),
        fields: vec![
        ],
    }),
    ("[]d2target.ClassMethod".into(), Type {
        kind: "slice".into(),
        elem: "d2target.ClassMethod".into(),
        fields: vec![
        ],
    }),
    ("[]d2target.SQLColumn".into(), Type {
        kind: "slice".into(),
        elem: "d2target.SQLColumn".into(),
        fields: vec![
        ],
    }),
    ("[]string".into(), Type {
        kind: "slice".into(),
        elem: "string".into(),
        fields: vec![
        ],
    }),
    ("bool".into(), Type {
        kind: "bool".into(),
        elem: "".into(),
        fields: vec![
        ],
    }),
    ("d2ast.BlockString".into(), Type {
        kind: "struct".into(),
        elem: "".into(),
        fields: vec![
            Field { name: "range".into(), kind: "d2ast.Range".into(), omit: false },
            Field { name: "quote".into(), kind: "string".into(), omit: false },
            Field { name: "tag".into(), kind: "string".into(), omit: false },
            Field { name: "value".into(), kind: "string".into(), omit: false },
        ],
    }),
    ("d2ast.DoubleQuotedString".into(), Type {
        kind: "struct".into(),
        elem: "".into(),
        fields: vec![
            Field { name: "range".into(), kind: "d2ast.Range".into(), omit: false },
            Field { name: "value".into(), kind: "[]d2ast.InterpolationBox".into(), omit: false },
        ],
    }),
    ("d2ast.InterpolationBox".into(), Type {
        kind: "struct".into(),
        elem: "".into(),
        fields: vec![
            Field { name: "string".into(), kind: "*string".into(), omit: true },
            Field { name: "raw_string".into(), kind: "*string".into(), omit: true },
            Field { name: "substitution".into(), kind: "*d2ast.Substitution".into(), omit: true },
        ],
    }),
    ("d2ast.KeyPath".into(), Type {
        kind: "struct".into(),
        elem: "".into(),
        fields: vec![
            Field { name: "range".into(), kind: "d2ast.Range".into(), omit: false },
            Field { name: "path".into(), kind: "[]*d2ast.StringBox".into(), omit: false },
        ],
    }),
    ("d2ast.Range".into(), Type {
        kind: "text".into(),
        elem: "".into(),
        fields: vec![
        ],
    }),
    ("d2ast.SingleQuotedString".into(), Type {
        kind: "struct".into(),
        elem: "".into(),
        fields: vec![
            Field { name: "range".into(), kind: "d2ast.Range".into(), omit: false },
            Field { name: "raw".into(), kind: "string".into(), omit: false },
            Field { name: "value".into(), kind: "string".into(), omit: false },
        ],
    }),
    ("d2ast.StringBox".into(), Type {
        kind: "struct".into(),
        elem: "".into(),
        fields: vec![
            Field { name: "unquoted_string".into(), kind: "*d2ast.UnquotedString".into(), omit: true },
            Field { name: "double_quoted_string".into(), kind: "*d2ast.DoubleQuotedString".into(), omit: true },
            Field { name: "single_quoted_string".into(), kind: "*d2ast.SingleQuotedString".into(), omit: true },
            Field { name: "block_string".into(), kind: "*d2ast.BlockString".into(), omit: true },
        ],
    }),
    ("d2ast.Substitution".into(), Type {
        kind: "struct".into(),
        elem: "".into(),
        fields: vec![
            Field { name: "range".into(), kind: "d2ast.Range".into(), omit: false },
            Field { name: "spread".into(), kind: "bool".into(), omit: false },
            Field { name: "path".into(), kind: "[]*d2ast.StringBox".into(), omit: false },
        ],
    }),
    ("d2ast.UnquotedString".into(), Type {
        kind: "struct".into(),
        elem: "".into(),
        fields: vec![
            Field { name: "range".into(), kind: "d2ast.Range".into(), omit: false },
            Field { name: "value".into(), kind: "[]d2ast.InterpolationBox".into(), omit: false },
            Field { name: "pattern".into(), kind: "[]string".into(), omit: true },
        ],
    }),
    ("d2graph.Attributes".into(), Type {
        kind: "struct".into(),
        elem: "".into(),
        fields: vec![
            Field { name: "label".into(), kind: "d2graph.Scalar".into(), omit: false },
            Field { name: "labelDimensions".into(), kind: "d2target.TextDimensions".into(), omit: false },
            Field { name: "style".into(), kind: "d2graph.Style".into(), omit: false },
            Field { name: "icon".into(), kind: "*url.URL".into(), omit: true },
            Field { name: "iconStyle".into(), kind: "d2graph.Style".into(), omit: false },
            Field { name: "tooltip".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "link".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "width".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "height".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "top".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "left".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "near_key".into(), kind: "*d2ast.KeyPath".into(), omit: false },
            Field { name: "language".into(), kind: "string".into(), omit: true },
            Field { name: "shape".into(), kind: "d2graph.Scalar".into(), omit: false },
            Field { name: "direction".into(), kind: "d2graph.Scalar".into(), omit: false },
            Field { name: "constraint".into(), kind: "[]string".into(), omit: false },
            Field { name: "gridRows".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "gridColumns".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "gridGap".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "verticalGap".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "horizontalGap".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "labelPosition".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "iconPosition".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "tooltipPosition".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "classes".into(), kind: "[]string".into(), omit: true },
        ],
    }),
    ("d2graph.Edge".into(), Type {
        kind: "struct".into(),
        elem: "".into(),
        fields: vec![
            Field { name: "index".into(), kind: "int".into(), omit: false },
            Field { name: "srcTableColumnIndex".into(), kind: "*int".into(), omit: true },
            Field { name: "dstTableColumnIndex".into(), kind: "*int".into(), omit: true },
            Field { name: "labelPosition".into(), kind: "*string".into(), omit: true },
            Field { name: "labelPercentage".into(), kind: "*float64".into(), omit: true },
            Field { name: "isCurve".into(), kind: "bool".into(), omit: false },
            Field { name: "route".into(), kind: "[]*geo.Point".into(), omit: true },
            Field { name: "src_arrow".into(), kind: "bool".into(), omit: false },
            Field { name: "srcArrowhead".into(), kind: "*d2graph.Attributes".into(), omit: true },
            Field { name: "dst_arrow".into(), kind: "bool".into(), omit: false },
            Field { name: "dstArrowhead".into(), kind: "*d2graph.Attributes".into(), omit: true },
            Field { name: "references".into(), kind: "[]d2graph.EdgeReference".into(), omit: true },
            Field { name: "attributes".into(), kind: "d2graph.Attributes".into(), omit: true },
            Field { name: "zIndex".into(), kind: "int".into(), omit: false },
        ],
    }),
    ("d2graph.EdgeReference".into(), Type {
        kind: "struct".into(),
        elem: "".into(),
        fields: vec![
            Field { name: "map_key_edge_index".into(), kind: "int".into(), omit: false },
        ],
    }),
    ("d2graph.Object".into(), Type {
        kind: "struct".into(),
        elem: "".into(),
        fields: vec![
            Field { name: "id".into(), kind: "string".into(), omit: false },
            Field { name: "id_val".into(), kind: "string".into(), omit: false },
            Field { name: "references".into(), kind: "[]d2graph.Reference".into(), omit: true },
            Field { name: "box".into(), kind: "*geo.Box".into(), omit: true },
            Field { name: "labelPosition".into(), kind: "*string".into(), omit: true },
            Field { name: "iconPosition".into(), kind: "*string".into(), omit: true },
            Field { name: "contentAspectRatio".into(), kind: "*float64".into(), omit: true },
            Field { name: "class".into(), kind: "*d2target.Class".into(), omit: true },
            Field { name: "sql_table".into(), kind: "*d2target.SQLTable".into(), omit: true },
            Field { name: "attributes".into(), kind: "d2graph.Attributes".into(), omit: false },
            Field { name: "zIndex".into(), kind: "int".into(), omit: false },
        ],
    }),
    ("d2graph.Reference".into(), Type {
        kind: "struct".into(),
        elem: "".into(),
        fields: vec![
            Field { name: "key".into(), kind: "*d2ast.KeyPath".into(), omit: false },
            Field { name: "key_path_index".into(), kind: "int".into(), omit: false },
            Field { name: "map_key_edge_index".into(), kind: "int".into(), omit: false },
        ],
    }),
    ("d2graph.Scalar".into(), Type {
        kind: "struct".into(),
        elem: "".into(),
        fields: vec![
            Field { name: "value".into(), kind: "string".into(), omit: false },
        ],
    }),
    ("d2graph.Style".into(), Type {
        kind: "struct".into(),
        elem: "".into(),
        fields: vec![
            Field { name: "opacity".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "stroke".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "fill".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "fillPattern".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "strokeWidth".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "strokeDash".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "borderRadius".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "shadow".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "3d".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "multiple".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "font".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "fontSize".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "fontColor".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "animated".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "bold".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "italic".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "underline".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "filled".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "doubleBorder".into(), kind: "*d2graph.Scalar".into(), omit: true },
            Field { name: "textTransform".into(), kind: "*d2graph.Scalar".into(), omit: true },
        ],
    }),
    ("d2target.Class".into(), Type {
        kind: "struct".into(),
        elem: "".into(),
        fields: vec![
            Field { name: "fields".into(), kind: "[]d2target.ClassField".into(), omit: false },
            Field { name: "methods".into(), kind: "[]d2target.ClassMethod".into(), omit: false },
        ],
    }),
    ("d2target.ClassField".into(), Type {
        kind: "struct".into(),
        elem: "".into(),
        fields: vec![
            Field { name: "name".into(), kind: "string".into(), omit: false },
            Field { name: "type".into(), kind: "string".into(), omit: false },
            Field { name: "visibility".into(), kind: "string".into(), omit: false },
            Field { name: "underline".into(), kind: "bool".into(), omit: false },
        ],
    }),
    ("d2target.ClassMethod".into(), Type {
        kind: "struct".into(),
        elem: "".into(),
        fields: vec![
            Field { name: "name".into(), kind: "string".into(), omit: false },
            Field { name: "return".into(), kind: "string".into(), omit: false },
            Field { name: "visibility".into(), kind: "string".into(), omit: false },
            Field { name: "underline".into(), kind: "bool".into(), omit: false },
        ],
    }),
    ("d2target.SQLColumn".into(), Type {
        kind: "struct".into(),
        elem: "".into(),
        fields: vec![
            Field { name: "name".into(), kind: "d2target.Text".into(), omit: false },
            Field { name: "type".into(), kind: "d2target.Text".into(), omit: false },
            Field { name: "constraint".into(), kind: "[]string".into(), omit: false },
            Field { name: "reference".into(), kind: "string".into(), omit: false },
        ],
    }),
    ("d2target.SQLTable".into(), Type {
        kind: "struct".into(),
        elem: "".into(),
        fields: vec![
            Field { name: "columns".into(), kind: "[]d2target.SQLColumn".into(), omit: false },
        ],
    }),
    ("d2target.Text".into(), Type {
        kind: "struct".into(),
        elem: "".into(),
        fields: vec![
            Field { name: "label".into(), kind: "string".into(), omit: false },
            Field { name: "fontSize".into(), kind: "int".into(), omit: false },
            Field { name: "fontFamily".into(), kind: "string".into(), omit: false },
            Field { name: "language".into(), kind: "string".into(), omit: false },
            Field { name: "color".into(), kind: "string".into(), omit: false },
            Field { name: "italic".into(), kind: "bool".into(), omit: false },
            Field { name: "bold".into(), kind: "bool".into(), omit: false },
            Field { name: "underline".into(), kind: "bool".into(), omit: false },
            Field { name: "labelWidth".into(), kind: "int".into(), omit: false },
            Field { name: "labelHeight".into(), kind: "int".into(), omit: false },
            Field { name: "labelFill".into(), kind: "string".into(), omit: true },
        ],
    }),
    ("d2target.TextDimensions".into(), Type {
        kind: "struct".into(),
        elem: "".into(),
        fields: vec![
            Field { name: "width".into(), kind: "int".into(), omit: false },
            Field { name: "height".into(), kind: "int".into(), omit: false },
        ],
    }),
    ("float64".into(), Type {
        kind: "float64".into(),
        elem: "".into(),
        fields: vec![
        ],
    }),
    ("geo.Box".into(), Type {
        kind: "struct".into(),
        elem: "".into(),
        fields: vec![
            Field { name: "TopLeft".into(), kind: "*geo.Point".into(), omit: false },
            Field { name: "Width".into(), kind: "float64".into(), omit: false },
            Field { name: "Height".into(), kind: "float64".into(), omit: false },
        ],
    }),
    ("geo.Point".into(), Type {
        kind: "struct".into(),
        elem: "".into(),
        fields: vec![
            Field { name: "x".into(), kind: "float64".into(), omit: false },
            Field { name: "y".into(), kind: "float64".into(), omit: false },
        ],
    }),
    ("int".into(), Type {
        kind: "int".into(),
        elem: "".into(),
        fields: vec![
        ],
    }),
    ("string".into(), Type {
        kind: "string".into(),
        elem: "".into(),
        fields: vec![
        ],
    }),
    ("url.URL".into(), Type {
        kind: "struct".into(),
        elem: "".into(),
        fields: vec![
            Field { name: "Scheme".into(), kind: "string".into(), omit: false },
            Field { name: "Opaque".into(), kind: "string".into(), omit: false },
            Field { name: "User".into(), kind: "*url.Userinfo".into(), omit: false },
            Field { name: "Host".into(), kind: "string".into(), omit: false },
            Field { name: "Path".into(), kind: "string".into(), omit: false },
            Field { name: "Fragment".into(), kind: "string".into(), omit: false },
            Field { name: "RawQuery".into(), kind: "string".into(), omit: false },
            Field { name: "RawPath".into(), kind: "string".into(), omit: false },
            Field { name: "RawFragment".into(), kind: "string".into(), omit: false },
            Field { name: "ForceQuery".into(), kind: "bool".into(), omit: false },
            Field { name: "OmitHost".into(), kind: "bool".into(), omit: false },
        ],
    }),
    ("url.Userinfo".into(), Type {
        kind: "struct".into(),
        elem: "".into(),
        fields: vec![
        ],
    }),
]

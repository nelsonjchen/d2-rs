// SPDX-License-Identifier: MPL-2.0
//! Original D2 v0.9.0 d2scenebuild/markdown.go and metadata.go link regions.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug)]
pub struct LinkBudget {
    pub max_regions: isize,
    pub max_string_bytes: isize,
}
impl Default for LinkBudget {
    fn default() -> Self {
        Self {
            max_regions: 4096,
            max_string_bytes: 1 << 20,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LinkBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LinkRegion {
    pub r#box: LinkBox,
    #[serde(rename = "URL")]
    pub url: String,
    pub tooltip: String,
    pub target: String,
}
#[derive(Clone, Debug)]
pub struct LinkState {
    pub links: Vec<LinkRegion>,
    pub appendix: Vec<String>,
    pub link_bytes: usize,
    pub appendix_bytes: usize,
    pub budget: LinkBudget,
    pub append: bool,
}
impl LinkState {
    pub fn new(budget: LinkBudget, append: bool) -> Self {
        Self {
            links: Vec::new(),
            appendix: Vec::new(),
            link_bytes: 0,
            appendix_bytes: 0,
            budget,
            append,
        }
    }
    fn validate_string(object: &str, field: &str, value: &str) -> Result<(), String> {
        if value.chars().any(|c| {
            !matches!(c, '\t' | '\n' | '\r')
                && !(('\u{20}'..='\u{d7ff}').contains(&c)
                    || ('\u{e000}'..='\u{fffd}').contains(&c)
                    || c >= '\u{10000}')
        }) {
            return Err(format!(
                "scene: {object} field \"{field}\" contains a character forbidden by XML 1.0 metadata consumers"
            ));
        }
        Ok(())
    }
    fn check_budget(&self, markdown: bool) -> Result<(), String> {
        if self.budget.max_regions <= 0 || self.budget.max_string_bytes <= 0 {
            return Err(format!(
                "scene: options field \"linkBudget\" has invalid value {{{} {}}}: must provide positive MaxRegions and MaxStringBytes for {}",
                self.budget.max_regions,
                self.budget.max_string_bytes,
                if markdown {
                    "Markdown links"
                } else {
                    "link/tooltip metadata"
                }
            ));
        }
        Ok(())
    }
    fn validate_fields(&self, object: &str, url: &str, title: &str) -> Result<(), String> {
        let mut bytes = self.link_bytes;
        for (field, value) in [("link", url), ("tooltip", title)] {
            if value.len() as i128 > self.budget.max_string_bytes as i128 - bytes as i128 {
                return Err(format!(
                    "scene: link metadata string bytes exceed limit {}",
                    self.budget.max_string_bytes
                ));
            }
            Self::validate_string(object, field, value)?;
            bytes += value.len();
        }
        Ok(())
    }
    fn add_region(
        &mut self,
        object: &str,
        url: &str,
        title: &str,
        box_: LinkBox,
    ) -> Result<(), String> {
        if self.links.len() >= self.budget.max_regions as usize {
            return Err(format!(
                "scene: link region count exceeds limit {}",
                self.budget.max_regions
            ));
        }
        if ![box_.x, box_.y, box_.width, box_.height]
            .iter()
            .all(|v| v.is_finite())
            || box_.width <= 0.0
            || box_.height <= 0.0
        {
            return Err(format!(
                "scene: {object} field \"linkRegion\" has invalid value {{{} {} {} {}}}: must be a finite box with positive dimensions",
                box_.x, box_.y, box_.width, box_.height
            ));
        }
        if url.is_empty() && title.is_empty() {
            return Err(format!("scene: {object} link metadata is empty"));
        }
        self.links.push(LinkRegion {
            r#box: box_,
            url: url.to_owned(),
            tooltip: title.to_owned(),
            target: String::new(),
        });
        self.link_bytes += url.len() + title.len();
        Ok(())
    }
    pub fn add_appendix(&mut self, object: &str, field: &str, title: &str) -> Result<(), String> {
        if !self.append || title.is_empty() {
            return Ok(());
        }
        if self.budget.max_regions <= 0 || self.budget.max_string_bytes <= 0 {
            return Ok(());
        }
        let limit = self.budget.max_regions.saturating_mul(2) as usize;
        if self.appendix.len() >= limit {
            return Err(format!("scene: appendix item count exceeds limit {limit}"));
        }
        Self::validate_string(object, field, title)?;
        if title.len() as i128 > self.budget.max_string_bytes as i128 - self.appendix_bytes as i128
        {
            return Err(format!(
                "scene: appendix string bytes exceed limit {}",
                self.budget.max_string_bytes
            ));
        }
        self.appendix.push(title.to_owned());
        self.appendix_bytes += title.len();
        Ok(())
    }
    pub fn add_markdown_region(
        &mut self,
        object: &str,
        index: usize,
        url: &str,
        title: &str,
        box_: LinkBox,
    ) -> Result<(), String> {
        self.check_budget(true)?;
        let object = format!("{object} Markdown primitive {index}");
        self.validate_fields(&object, url, title)?;
        let start = self.links.len();
        let bytes = self.link_bytes;
        self.add_region(&object, url, title, box_)?;
        if let Err(error) = self.add_appendix(&object, "linkTitle", title) {
            self.links.truncate(start);
            self.link_bytes = bytes;
            return Err(error);
        }
        Ok(())
    }
    pub fn add_outer_region(
        &mut self,
        object: &str,
        url: &str,
        title: &str,
        pretty: &str,
        box_: LinkBox,
    ) -> Result<(), String> {
        self.check_budget(false)?;
        self.validate_fields(object, url, title)?;
        self.add_region(object, url, title, box_)?;
        self.add_appendix(object, "tooltip", title)?;
        self.add_appendix(object, "prettyLink", pretty)
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Primitive {
    pub kind: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub text: String,
    pub font_size: f64,
    pub link: String,
    pub link_title: String,
    pub synthetic_italic: bool,
}
impl Primitive {
    pub fn link_box(&self) -> Option<LinkBox> {
        if self.kind == "rect" {
            return Some(LinkBox {
                x: self.x,
                y: self.y,
                width: self.width,
                height: self.height,
            });
        }
        if self.kind != "text" {
            return None;
        }
        let width = if self.width > 0.0 {
            self.width
        } else {
            (self.text.chars().count() as f64 * self.font_size * 0.6).max(1.0)
        };
        let height = if self.height > 0.0 {
            self.height
        } else {
            self.font_size
        };
        let (mut left, mut right) = (self.x, self.x + width);
        let top = self.y - height;
        let bottom = self.y + self.font_size * 0.25;
        if self.synthetic_italic {
            let c = (-12.0 * std::f64::consts::PI / 180.0).tan();
            let e = -c * self.y;
            let points = [
                left + c * top + e,
                left + c * bottom + e,
                right + c * top + e,
                right + c * bottom + e,
            ];
            left = points.iter().copied().fold(f64::INFINITY, f64::min);
            right = points.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        }
        Some(LinkBox {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        })
    }
}
pub fn intersect(left: LinkBox, right: &LinkBox) -> Option<LinkBox> {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let right_x = (left.x + left.width).min(right.x + right.width);
    let bottom = (left.y + left.height).min(right.y + right.height);
    if right_x <= x || bottom <= y {
        return None;
    }
    Some(LinkBox {
        x,
        y,
        width: right_x - x,
        height: bottom - y,
    })
}
pub fn collect_primitives(
    state: &mut LinkState,
    object: &str,
    primitives: &[Primitive],
    viewport: LinkBox,
    origin: Point,
    disable_links: bool,
) -> Result<(), String> {
    if viewport.width == 0.0 || viewport.height == 0.0 || disable_links {
        return Ok(());
    }
    for (index, p) in primitives.iter().enumerate() {
        let Some(url) = super::native_markdown_safe_link(&p.link).filter(|s| !s.is_empty()) else {
            continue;
        };
        let Some(box_) = p.link_box().and_then(|b| intersect(b, &viewport)) else {
            continue;
        };
        let x = box_.x + origin.x;
        let y = box_.y + origin.y;
        let right = box_.x + box_.width + origin.x;
        let bottom = box_.y + box_.height + origin.y;
        state.add_markdown_region(
            object,
            index,
            &url,
            &p.link_title,
            LinkBox {
                x,
                y,
                width: right - x,
                height: bottom - y,
            },
        )?;
    }
    Ok(())
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod source_contract {
    use super::*;
    #[test]
    fn complete_original_scene_metadata_inventory() {
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/markdown-scene-metadata-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 3248);
        let mut outcomes = Vec::new();
        let mut failures = Vec::new();
        for row in rows.as_array().unwrap() {
            let r = &row["request"];
            let mut state = LinkState::new(
                LinkBudget {
                    max_regions: r["MaxRegions"].as_i64().unwrap() as isize,
                    max_string_bytes: r["MaxStringBytes"].as_i64().unwrap() as isize,
                },
                r["Appendix"].as_bool().unwrap(),
            );
            let width = if r["Width"].as_i64().unwrap() < 0 {
                row["layout"]["Width"].as_f64().unwrap()
            } else {
                r["Width"].as_f64().unwrap()
            };
            let height = if r["Height"].as_i64().unwrap() < 0 {
                row["layout"]["Height"].as_f64().unwrap()
            } else {
                r["Height"].as_f64().unwrap()
            };
            let disabled = r["DisableLinks"].as_bool().unwrap();
            let mut error = None;
            if disabled {
                if let Err(e) = state.add_outer_region(
                    "shape \"markdown\"",
                    "https://outer.example.com",
                    "",
                    "outer",
                    LinkBox {
                        x: 0.0,
                        y: 0.0,
                        width,
                        height,
                    },
                ) {
                    error = Some(e);
                }
            }
            if error.is_none() {
                let primitives: Vec<Primitive> =
                    serde_json::from_value(row["layout"]["Primitives"].clone()).unwrap();
                if let Err(e) = collect_primitives(
                    &mut state,
                    "shape \"markdown\"",
                    &primitives,
                    LinkBox {
                        x: 0.0,
                        y: 0.0,
                        width,
                        height,
                    },
                    Point { x: 5.0, y: 5.0 },
                    disabled,
                ) {
                    error = Some(e);
                }
            }
            let actual = serde_json::json!({"links":state.links,"appendix_titles":state.appendix,"error":error});
            let matched = if let Some(expected) = row.get("error") {
                actual["error"] == *expected
            } else {
                actual["error"].is_null()
                    && state.links
                        == serde_json::from_value::<Vec<LinkRegion>>(row["links"].clone()).unwrap()
                    && actual["appendix_titles"] == row["appendix_titles"]
            };
            if !matched {
                failures.push(r["Name"].clone());
            }
            outcomes.push(
                serde_json::json!({"request":r,"actual":actual,"expected":row,"matched":matched}),
            );
        }
        if let Ok(path) = std::env::var("D2_RS_TEST_MD_METADATA_OUTPUT") {
            std::fs::write(path, serde_json::to_vec(&outcomes).unwrap()).unwrap();
        }
        assert!(
            failures.is_empty(),
            "scene metadata differences: {failures:?}"
        );
    }
}

// Retain native f64 primitive fields across the existing SVG scene transport.
// These are production link hit regions, not diagnostic trace arguments.
pub(super) fn tag_primitive(
    svg: &mut String,
    from: usize,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    size: f64,
    italic: bool,
) {
    let Some(offset) = svg[from..].find(if size == 0.0 { "<rect " } else { "<text " }) else {
        return;
    };
    let Some(end) = svg[from + offset..].find('>') else {
        return;
    };
    let mut position = from + offset + end;
    if svg.as_bytes().get(position.wrapping_sub(1)) == Some(&b'/') {
        position -= 1;
    }
    let _fields = format!(
        " data-d2-md-x=\"{x}\" data-d2-md-y=\"{y}\" data-d2-md-width=\"{width}\" data-d2-md-height=\"{height}\" data-d2-md-size=\"{size}\" data-d2-md-italic=\"{italic}\" "
    );
    svg.insert_str(position, &_fields);
}
fn number_attr(node: roxmltree::Node<'_, '_>, name: &str) -> f64 {
    node.attribute(name)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0)
}
/// Collect retained link primitives from native label groups. HTML fallback
/// labels have no native primitive group and are outside this scene bridge.
pub fn collect_svg(svg: &[u8], budget: LinkBudget, append: bool) -> Result<LinkState, String> {
    let text =
        std::str::from_utf8(svg).map_err(|_| "scene: native SVG must be valid UTF-8".to_owned())?;
    let doc =
        roxmltree::Document::parse(text).map_err(|e| format!("scene: native SVG parse: {e}"))?;
    let mut state = LinkState::new(budget, append);
    for viewport in doc.descendants().filter(|n| {
        n.has_tag_name("svg")
            && n.children().any(|g| {
                g.attribute("class")
                    .is_some_and(|c| c.split_whitespace().any(|c| c == "md-native"))
            })
    }) {
        if viewport
            .ancestors()
            .any(|n| n.attribute("opacity").and_then(|v| v.parse::<f64>().ok()) == Some(0.0))
        {
            continue;
        }
        let width = number_attr(viewport, "width");
        let height = number_attr(viewport, "height");
        if width == 0.0 || height == 0.0 {
            continue;
        }
        if viewport.ancestors().skip(1).any(|n| n.has_tag_name("a")) {
            continue;
        }
        let group = viewport
            .children()
            .find(|g| {
                g.attribute("class")
                    .is_some_and(|c| c.split_whitespace().any(|c| c == "md-native"))
            })
            .unwrap();
        let mut primitives = Vec::new();
        for node in group
            .descendants()
            .filter(|n| n.is_element() && matches!(n.tag_name().name(), "text" | "rect" | "line"))
        {
            let anchor = node
                .ancestors()
                .take_while(|n| *n != viewport)
                .find(|n| n.has_tag_name("a"));
            let link = anchor
                .and_then(|a| a.attribute("href"))
                .unwrap_or("")
                .to_owned();
            let title = anchor
                .and_then(|a| a.children().find(|n| n.has_tag_name("title")))
                .and_then(|n| n.text())
                .unwrap_or("")
                .to_owned();
            let tagged = node.attribute("data-d2-md-x").is_some();
            let mut x = number_attr(node, if tagged { "data-d2-md-x" } else { "x" });
            let mut y = number_attr(node, if tagged { "data-d2-md-y" } else { "y" });
            for parent in node.ancestors().skip(1).take_while(|n| *n != viewport) {
                if let Some(t) = parent
                    .attribute("transform")
                    .and_then(|s| s.strip_prefix("translate("))
                    .and_then(|s| s.strip_suffix(')'))
                {
                    let values = t
                        .split(|c: char| c.is_whitespace() || c == ',')
                        .filter(|s| !s.is_empty())
                        .filter_map(|s| s.parse::<f64>().ok())
                        .collect::<Vec<_>>();
                    if let Some(tx) = values.first() {
                        x += tx;
                    }
                    if let Some(ty) = values.get(1) {
                        y += ty;
                    }
                }
            }
            primitives.push(Primitive {
                kind: node.tag_name().name().to_owned(),
                x,
                y,
                width: number_attr(node, if tagged { "data-d2-md-width" } else { "width" }),
                height: number_attr(
                    node,
                    if tagged {
                        "data-d2-md-height"
                    } else {
                        "height"
                    },
                ),
                font_size: number_attr(
                    node,
                    if tagged {
                        "data-d2-md-size"
                    } else {
                        "font-size"
                    },
                ),
                text: node.text().unwrap_or("").to_owned(),
                link,
                link_title: title,
                synthetic_italic: node.attribute("data-d2-md-italic") == Some("true"),
            });
        }
        collect_primitives(
            &mut state,
            "native Markdown label",
            &primitives,
            LinkBox {
                x: 0.0,
                y: 0.0,
                width,
                height,
            },
            Point {
                x: number_attr(viewport, "x"),
                y: number_attr(viewport, "y"),
            },
            false,
        )?;
    }
    Ok(state)
}

#[cfg(all(test, feature = "oracle-fixtures"))]
mod native_pipeline_contract {
    use super::*;
    #[test]
    fn complete_original_native_markdown_metadata_pipeline() {
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/markdown-scene-metadata-source-v090.json"
        ))
        .unwrap();
        let mut outcomes = Vec::new();
        let mut failures = Vec::new();
        for row in rows.as_array().unwrap() {
            let r = &row["request"];
            let family = match r["Family"].as_str().unwrap() {
                "mono" => MarkdownFamily::Mono,
                "hand" => MarkdownFamily::HandDrawn,
                _ => MarkdownFamily::Sans,
            };
            let size = r["FontSize"].as_u64().unwrap() as u32;
            let natural = native_markdown_measurer(family)
                .unwrap()
                .markdown_dimensions(
                    r["Markdown"].as_str().unwrap(),
                    size,
                    false,
                    false,
                    matches!(family, MarkdownFamily::Mono),
                );
            let width = if r["Width"].as_i64().unwrap() < 0 {
                natural.width
            } else {
                r["Width"].as_f64().unwrap()
            };
            let height = if r["Height"].as_i64().unwrap() < 0 {
                natural.height
            } else {
                r["Height"].as_f64().unwrap()
            };
            let mut svg = String::new();
            let supported = draw_native_markdown_layout_viewport(
                &mut svg,
                r["Markdown"].as_str().unwrap(),
                Rect {
                    origin: Point { x: 5.0, y: 5.0 },
                    size: Size { width, height },
                },
                size,
                "#0A0F25",
                None,
                true,
                family,
            );
            let disabled = r["DisableLinks"].as_bool().unwrap();
            let budget = LinkBudget {
                max_regions: r["MaxRegions"].as_i64().unwrap() as isize,
                max_string_bytes: r["MaxStringBytes"].as_i64().unwrap() as isize,
            };
            let mut state = LinkState::new(budget, r["Appendix"].as_bool().unwrap());
            let mut error = None;
            if disabled {
                if let Err(e) = state.add_outer_region(
                    "shape \"markdown\"",
                    "https://outer.example.com",
                    "",
                    "outer",
                    LinkBox {
                        x: 0.0,
                        y: 0.0,
                        width,
                        height,
                    },
                ) {
                    error = Some(e);
                }
            }
            if !disabled && error.is_none() {
                let wrapper = format!(
                    "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\">{svg}</svg>"
                );
                match collect_svg(wrapper.as_bytes(), budget, r["Appendix"].as_bool().unwrap()) {
                    Ok(s) => state = s,
                    Err(e) => error = Some(e),
                }
            }
            let matched = supported
                && if let Some(expected) = row.get("error") {
                    error.as_deref() == expected.as_str()
                } else {
                    error.is_none()
                        && state.links
                            == serde_json::from_value::<Vec<LinkRegion>>(row["links"].clone())
                                .unwrap()
                        && serde_json::json!(state.appendix) == row["appendix_titles"]
                };
            if !matched {
                failures.push(r["Name"].clone());
            }
            outcomes.push(serde_json::json!({"request":r,"supported":supported,"links":state.links,"appendix_titles":state.appendix,"error":error,"natural":natural,"matched":matched}));
        }
        if let Ok(path) = std::env::var("D2_RS_TEST_MD_NATIVE_METADATA_OUTPUT") {
            std::fs::write(path, serde_json::to_vec(&outcomes).unwrap()).unwrap();
        }
        assert!(
            failures.is_empty(),
            "native metadata pipeline differences: {failures:?}"
        );
    }
}

#[cfg(test)]
mod transaction_source_contract {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_original_link_budget_transactions_and_rollback() {
        let rows: Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/markdown-link-transactions-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 472);
        let mut outcomes = Vec::new();
        let mut failures = Vec::new();
        for row in rows.as_array().unwrap() {
            let r = &row["request"];
            let mut state = LinkState::new(
                LinkBudget {
                    max_regions: r["MaxRegions"].as_i64().unwrap() as isize,
                    max_string_bytes: r["MaxBytes"].as_i64().unwrap() as isize,
                },
                r["Appendix"].as_bool().unwrap(),
            );
            for _ in 0..r["Seed"].as_u64().unwrap() {
                state.links.push(LinkRegion {
                    r#box: LinkBox {
                        x: 0.0,
                        y: 0.0,
                        width: 1.0,
                        height: 1.0,
                    },
                    url: "s".into(),
                    tooltip: String::new(),
                    target: String::new(),
                });
                state.link_bytes += 1;
                state.appendix.push("seed".into());
                state.appendix_bytes += 4;
            }
            let error = state
                .add_markdown_region(
                    "node",
                    3,
                    r["URL"].as_str().unwrap(),
                    r["Title"].as_str().unwrap(),
                    serde_json::from_value(r["Box"].clone()).unwrap(),
                )
                .err()
                .unwrap_or_default();
            let expected = if row["links"].is_null() {
                Vec::new()
            } else {
                serde_json::from_value::<Vec<LinkRegion>>(row["links"].clone()).unwrap()
            };
            let matched = error == row["error"].as_str().unwrap()
                && state.links == expected
                && serde_json::json!(state.appendix) == row["appendix"]
                && state.link_bytes as u64 == row["link_bytes"].as_u64().unwrap()
                && state.appendix_bytes as u64 == row["appendix_bytes"].as_u64().unwrap();
            if !matched {
                failures.push(r["Name"].clone());
            }
            outcomes.push(serde_json::json!({"request":r,"error":error,"links":state.links,"appendix":state.appendix,"link_bytes":state.link_bytes,"appendix_bytes":state.appendix_bytes,"matched":matched}));
        }
        if let Ok(path) = std::env::var("D2_RS_TEST_MD_TRANSACTIONS_OUTPUT") {
            std::fs::write(path, serde_json::to_vec(&outcomes).unwrap()).unwrap();
        }
        assert!(
            failures.is_empty(),
            "link transaction differences: {failures:?}"
        );
    }
}

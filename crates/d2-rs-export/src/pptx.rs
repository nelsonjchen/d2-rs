//! Small OOXML package writer for the native document exporter.
//!
//! The base parts are the same D2 v0.9.0 template used by the Go exporter.
//! We copy the template members, preserving their ZIP metadata, then add one slide
//! and its PNG relationship. The same package writer handles one or many
//! rendered boards without changing the raster path.

// The slide writer follows the upstream package assembly shape so generated
// OOXML remains stable across release builds.
#![allow(clippy::too_many_arguments)]

use crate::go_zip::Writer;

const TEMPLATE: &[u8] = include_bytes!("../assets/d2-template.pptx");
pub(crate) const SLIDE_WIDTH: u32 = 9_144_000;
pub(crate) const SLIDE_HEIGHT: u32 = 5_143_500;
const HEADER_HEIGHT: u32 = 392_471;
const IMAGE_WIDTH: u32 = 8_446_273;

/// Document properties passed by the CLI before package assembly.
#[derive(Clone, Debug)]
pub struct PptxDocumentProperties {
    pub title: String,
    pub subject: String,
    pub creator: String,
    pub description: String,
    pub version: String,
}

impl Default for PptxDocumentProperties {
    fn default() -> Self {
        Self {
            title: "D2 diagram".into(),
            subject: String::new(),
            creator: "d2-rs".into(),
            description: "Presentation generated with D2 - https://d2lang.com".into(),
            version: "0.9.0".into(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct LinkRegion {
    pub left: i64,
    pub top: i64,
    pub width: i64,
    pub height: i64,
    pub href: String,
    pub tooltip: String,
    pub target_slide: Option<usize>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct BoardTitlePath {
    pub names: Vec<String>,
}

pub(crate) fn package_single_slide_with_links(
    png: &[u8],
    links: &[LinkRegion],
) -> Result<Vec<u8>, String> {
    package_slides_with_board_paths_and_links(
        &[png.to_vec()],
        &[vec!["root".to_owned()]],
        &[links.to_vec()],
    )
}

pub(crate) fn package_slides(slides: &[Vec<u8>]) -> Result<Vec<u8>, String> {
    package_slides_with_board_paths(
        slides,
        &slides
            .iter()
            .enumerate()
            .map(|(index, _)| {
                vec![if index == 0 {
                    "root".to_owned()
                } else {
                    format!("slide{}", index + 1)
                }]
            })
            .collect::<Vec<_>>(),
    )
}

pub(crate) fn package_slides_with_board_ids(
    slides: &[Vec<u8>],
    board_ids: &[String],
) -> Result<Vec<u8>, String> {
    if board_ids.is_empty() {
        return package_slides(slides);
    }
    package_slides_with_board_paths(
        slides,
        &board_ids
            .iter()
            .map(|board_id| vec![board_id.clone()])
            .collect::<Vec<_>>(),
    )
}

pub(crate) fn package_slides_with_board_paths(
    slides: &[Vec<u8>],
    board_paths: &[Vec<String>],
) -> Result<Vec<u8>, String> {
    let links = vec![Vec::new(); slides.len()];
    package_slides_with_board_paths_and_links(slides, board_paths, &links)
}

pub(crate) fn package_slides_with_board_paths_and_links(
    slides: &[Vec<u8>],
    board_paths: &[Vec<String>],
    links: &[Vec<LinkRegion>],
) -> Result<Vec<u8>, String> {
    let title_paths = vec![BoardTitlePath::default(); slides.len()];
    package_slides_with_board_paths_titles_and_links(
        slides,
        board_paths,
        &title_paths,
        links,
        false,
    )
}

pub(crate) fn package_slides_with_board_paths_titles_and_links(
    slides: &[Vec<u8>],
    board_paths: &[Vec<String>],
    title_paths: &[BoardTitlePath],
    links: &[Vec<LinkRegion>],
    include_nav: bool,
) -> Result<Vec<u8>, String> {
    package_slides_with_properties(
        slides,
        board_paths,
        title_paths,
        links,
        include_nav,
        &PptxDocumentProperties::default(),
    )
}

pub(crate) fn package_slides_with_properties(
    slides: &[Vec<u8>],
    board_paths: &[Vec<String>],
    title_paths: &[BoardTitlePath],
    links: &[Vec<LinkRegion>],
    include_nav: bool,
    properties: &PptxDocumentProperties,
) -> Result<Vec<u8>, String> {
    if slides.is_empty() {
        return Err("cannot create an empty PPTX presentation".into());
    }
    if board_paths.len() != slides.len() {
        return Err("PPTX board ID count must match slide count".into());
    }
    if links.len() != slides.len() {
        return Err("PPTX link-region count must match slide count".into());
    }
    if title_paths.len() != slides.len() {
        return Err("PPTX title-path count must match slide count".into());
    }
    let page_map = board_paths
        .iter()
        .enumerate()
        .filter_map(|(index, path)| path.last().map(|id| (id.as_str(), index + 1)))
        .collect::<std::collections::BTreeMap<_, _>>();

    let mut writer = Writer::default();

    // lib/pptx/pptx.go writeTo writes the content types first, then copies
    // every template member (including directories) except these two rels.
    write_entry(
        &mut writer,
        "[Content_Types].xml",
        content_types_xml(slides.len()),
    )?;
    writer.copy_template(
        TEMPLATE,
        &[
            "_rels/.rels",
            "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        ],
    )?;

    write_entry(&mut writer, "_rels/.rels", root_relationships())?;
    write_entry(
        &mut writer,
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        slide_master_relationships(),
    )?;
    for (index, png) in slides.iter().enumerate() {
        let (source_width, source_height) = png_dimensions(png)?;
        let (image_width, image_height, image_top) =
            fit_image_for_slide(source_width, source_height, include_nav);
        let image_left = (SLIDE_WIDTH - image_width) / 2;
        let slide_number = index + 1;
        let fallback_board_id = format!("slide{slide_number}");
        let board_id = board_paths[index]
            .last()
            .map(String::as_str)
            .unwrap_or_else(|| {
                if index == 0 {
                    "root"
                } else {
                    fallback_board_id.as_str()
                }
            });
        write_entry(
            &mut writer,
            &format!("ppt/media/slide{slide_number}Image.png"),
            png,
        )?;
        write_entry(
            &mut writer,
            &format!("ppt/slides/_rels/slide{slide_number}.xml.rels"),
            slide_relationships(slide_number, &board_paths[index], &page_map, &links[index]),
        )?;
        write_entry(
            &mut writer,
            &format!("ppt/slides/slide{slide_number}.xml"),
            slide_xml(
                image_left,
                image_top,
                image_width,
                image_height,
                slide_number,
                board_id,
                &title_paths[index].names,
                include_nav,
                &links[index],
            ),
        )?;
    }
    write_entry(
        &mut writer,
        "ppt/_rels/presentation.xml.rels",
        presentation_relationships(slides.len()),
    )?;
    write_entry(
        &mut writer,
        "ppt/presentation.xml",
        presentation_xml(slides.len()),
    )?;
    write_entry(
        &mut writer,
        "docProps/core.xml",
        core_properties(properties),
    )?;
    write_entry(
        &mut writer,
        "docProps/app.xml",
        app_properties(board_paths, &properties.version),
    )?;

    writer.finish()
}

pub(crate) fn fit_image_for_slide(
    source_width: u32,
    source_height: u32,
    include_nav: bool,
) -> (u32, u32, u32) {
    let header_height = if include_nav { HEADER_HEIGHT } else { 0 };
    let available_height = SLIDE_HEIGHT - header_height;
    let aspect_ratio = f64::from(IMAGE_WIDTH) / f64::from(available_height);
    let source_ratio = f64::from(source_width) / f64::from(source_height);
    let (width, height) = if source_ratio >= aspect_ratio {
        let width = SLIDE_WIDTH;
        let height = (f64::from(width) * f64::from(source_height) / f64::from(source_width)) as u32;
        if height > available_height {
            let width = IMAGE_WIDTH;
            let height =
                (f64::from(width) * f64::from(source_height) / f64::from(source_width)) as u32;
            (width, height)
        } else {
            (width, height)
        }
    } else {
        let height = available_height;
        let width = (f64::from(height) * f64::from(source_width) / f64::from(source_height)) as u32;
        (width, height)
    };
    (
        width,
        height,
        header_height + (available_height - height) / 2,
    )
}

fn write_entry(writer: &mut Writer, name: &str, bytes: impl AsRef<[u8]>) -> Result<(), String> {
    writer.create(name, bytes.as_ref())
}

fn png_dimensions(png: &[u8]) -> Result<(u32, u32), String> {
    if png.len() < 24 || &png[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err("rasterizer did not return a PNG image".into());
    }
    let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
    if width == 0 || height == 0 {
        return Err("rasterized PNG has empty dimensions".into());
    }
    Ok((width, height))
}

// Direct native translation of lib/pptx/templates; retain every part and attribute.
fn content_types_xml(slide_count: usize) -> String {
    let mut xml = String::new();
    xml.push_str(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
    <Default
        Extension="jpeg" ContentType="image/jpeg" />
        <Default Extension="png" ContentType="image/png" />
        <Default
        Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml" />
        <Default
        Extension="xml" ContentType="application/xml" />
        <Override PartName="/docProps/app.xml"
        ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml" />
        <Override
        PartName="/docProps/core.xml"
        ContentType="application/vnd.openxmlformats-package.core-properties+xml" />
        <Override
        PartName="/ppt/presProps.xml"
        ContentType="application/vnd.openxmlformats-officedocument.presentationml.presProps+xml" />
        <Override
        PartName="/ppt/presentation.xml"
        ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml" />
        <Override
        PartName="/ppt/slideLayouts/slideLayout1.xml"
        ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml" />
        <Override
        PartName="/ppt/slideLayouts/slideLayout10.xml"
        ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml" />
        <Override
        PartName="/ppt/slideLayouts/slideLayout11.xml"
        ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml" />
        <Override
        PartName="/ppt/slideLayouts/slideLayout2.xml"
        ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml" />
        <Override
        PartName="/ppt/slideLayouts/slideLayout3.xml"
        ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml" />
        <Override
        PartName="/ppt/slideLayouts/slideLayout4.xml"
        ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml" />
        <Override
        PartName="/ppt/slideLayouts/slideLayout5.xml"
        ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml" />
        <Override
        PartName="/ppt/slideLayouts/slideLayout6.xml"
        ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml" />
        <Override
        PartName="/ppt/slideLayouts/slideLayout7.xml"
        ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml" />
        <Override
        PartName="/ppt/slideLayouts/slideLayout8.xml"
        ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml" />
        <Override
        PartName="/ppt/slideLayouts/slideLayout9.xml"
        ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml" />
        <Override
        PartName="/ppt/slideMasters/slideMaster1.xml"
        ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml" />
        "#);
    for number in 1..=slide_count {
        xml.push_str(
            r#"
        <Override PartName="/ppt/slides/"#,
        );
        xml.push_str(&xml_escape(&(format!("slide{number}")).to_string()));
        xml.push_str(r#".xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml" />
        "#);
    }
    xml.push_str(" ");
    xml.push_str(r#"
        <Override PartName="/ppt/tableStyles.xml"
        ContentType="application/vnd.openxmlformats-officedocument.presentationml.tableStyles+xml" />
        <Override
        PartName="/ppt/theme/theme1.xml"
        ContentType="application/vnd.openxmlformats-officedocument.theme+xml" />
        <Override
        PartName="/ppt/viewProps.xml"
        ContentType="application/vnd.openxmlformats-officedocument.presentationml.viewProps+xml" />
</Types>
"#);
    xml
}

fn root_relationships() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
    <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>
    <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>
    <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/>
    <Relationship Id="rId4" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/thumbnail" Target="docProps/thumbnail.jpeg"/>
</Relationships>"#
}

// lib/pptx/templates/slidemaster_rels.xml regenerates relationships in
// layout order rather than retaining the template archive's original order.
fn slide_master_relationships() -> String {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n",
    );
    for number in 1..=11 {
        xml.push_str(&format!(
            "    <Relationship Id=\"rId{number}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"../slideLayouts/slideLayout{number}.xml\"/>\n",
        ));
    }
    xml.push_str(r#"    <Relationship Id="rId12" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme1.xml"/>
</Relationships>"#);
    xml
}

fn presentation_xml(slide_count: usize) -> String {
    let mut xml = String::new();
    xml.push_str(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
    xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
    xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" saveSubsetFonts="1"
    autoCompressPictures="0">
    <p:sldMasterIdLst>
        <p:sldMasterId id="2147483648" r:id="rId1" />
    </p:sldMasterIdLst>
    <p:sldIdLst>
        "#,
    );
    for index in 0..slide_count {
        xml.push_str(
            r#"
        <p:sldId id=""#,
        );
        xml.push_str(&xml_escape(&(256 + index).to_string()));
        xml.push_str(r#"" r:id=""#);
        xml.push_str(&xml_escape(&(format!("rId{}", 7 + index)).to_string()));
        xml.push_str(
            r#"" />
        "#,
        );
    }
    xml.push_str(
        r#"
    </p:sldIdLst>
    <p:sldSz cx=""#,
    );
    xml.push_str(&xml_escape(&(SLIDE_WIDTH).to_string()));
    xml.push_str(r#"" cy=""#);
    xml.push_str(&xml_escape(&(SLIDE_HEIGHT).to_string()));
    xml.push_str(
        r#"" type="screen16x9" />
    <p:notesSz cx="6858000" cy="9144000" />
    <p:defaultTextStyle>
        <a:defPPr>
            <a:defRPr lang="en-US" />
        </a:defPPr>
        <a:lvl1pPr marL="0" algn="l" defTabSz="457200" rtl="0" eaLnBrk="1" latinLnBrk="0"
            hangingPunct="1">
            <a:defRPr sz="1800" kern="1200">
                <a:solidFill>
                    <a:schemeClr val="tx1" />
                </a:solidFill>
                <a:latin typeface="+mn-lt" />
                <a:ea typeface="+mn-ea" />
                <a:cs typeface="+mn-cs" />
            </a:defRPr>
        </a:lvl1pPr>
        <a:lvl2pPr marL="457200" algn="l" defTabSz="457200" rtl="0" eaLnBrk="1" latinLnBrk="0"
            hangingPunct="1">
            <a:defRPr sz="1800" kern="1200">
                <a:solidFill>
                    <a:schemeClr val="tx1" />
                </a:solidFill>
                <a:latin typeface="+mn-lt" />
                <a:ea typeface="+mn-ea" />
                <a:cs typeface="+mn-cs" />
            </a:defRPr>
        </a:lvl2pPr>
        <a:lvl3pPr marL="914400" algn="l" defTabSz="457200" rtl="0" eaLnBrk="1" latinLnBrk="0"
            hangingPunct="1">
            <a:defRPr sz="1800" kern="1200">
                <a:solidFill>
                    <a:schemeClr val="tx1" />
                </a:solidFill>
                <a:latin typeface="+mn-lt" />
                <a:ea typeface="+mn-ea" />
                <a:cs typeface="+mn-cs" />
            </a:defRPr>
        </a:lvl3pPr>
        <a:lvl4pPr marL="1371600" algn="l" defTabSz="457200" rtl="0" eaLnBrk="1" latinLnBrk="0"
            hangingPunct="1">
            <a:defRPr sz="1800" kern="1200">
                <a:solidFill>
                    <a:schemeClr val="tx1" />
                </a:solidFill>
                <a:latin typeface="+mn-lt" />
                <a:ea typeface="+mn-ea" />
                <a:cs typeface="+mn-cs" />
            </a:defRPr>
        </a:lvl4pPr>
        <a:lvl5pPr marL="1828800" algn="l" defTabSz="457200" rtl="0" eaLnBrk="1" latinLnBrk="0"
            hangingPunct="1">
            <a:defRPr sz="1800" kern="1200">
                <a:solidFill>
                    <a:schemeClr val="tx1" />
                </a:solidFill>
                <a:latin typeface="+mn-lt" />
                <a:ea typeface="+mn-ea" />
                <a:cs typeface="+mn-cs" />
            </a:defRPr>
        </a:lvl5pPr>
        <a:lvl6pPr marL="2286000" algn="l" defTabSz="457200" rtl="0" eaLnBrk="1" latinLnBrk="0"
            hangingPunct="1">
            <a:defRPr sz="1800" kern="1200">
                <a:solidFill>
                    <a:schemeClr val="tx1" />
                </a:solidFill>
                <a:latin typeface="+mn-lt" />
                <a:ea typeface="+mn-ea" />
                <a:cs typeface="+mn-cs" />
            </a:defRPr>
        </a:lvl6pPr>
        <a:lvl7pPr marL="2743200" algn="l" defTabSz="457200" rtl="0" eaLnBrk="1" latinLnBrk="0"
            hangingPunct="1">
            <a:defRPr sz="1800" kern="1200">
                <a:solidFill>
                    <a:schemeClr val="tx1" />
                </a:solidFill>
                <a:latin typeface="+mn-lt" />
                <a:ea typeface="+mn-ea" />
                <a:cs typeface="+mn-cs" />
            </a:defRPr>
        </a:lvl7pPr>
        <a:lvl8pPr marL="3200400" algn="l" defTabSz="457200" rtl="0" eaLnBrk="1" latinLnBrk="0"
            hangingPunct="1">
            <a:defRPr sz="1800" kern="1200">
                <a:solidFill>
                    <a:schemeClr val="tx1" />
                </a:solidFill>
                <a:latin typeface="+mn-lt" />
                <a:ea typeface="+mn-ea" />
                <a:cs typeface="+mn-cs" />
            </a:defRPr>
        </a:lvl8pPr>
        <a:lvl9pPr marL="3657600" algn="l" defTabSz="457200" rtl="0" eaLnBrk="1" latinLnBrk="0"
            hangingPunct="1">
            <a:defRPr sz="1800" kern="1200">
                <a:solidFill>
                    <a:schemeClr val="tx1" />
                </a:solidFill>
                <a:latin typeface="+mn-lt" />
                <a:ea typeface="+mn-ea" />
                <a:cs typeface="+mn-cs" />
            </a:defRPr>
        </a:lvl9pPr>
    </p:defaultTextStyle>
    <p:extLst>
        <p:ext uri="{EFAFB233-063F-42B5-8137-9DF3F51BA10A}">
            <p15:sldGuideLst xmlns:p15="http://schemas.microsoft.com/office/powerpoint/2012/main">
                <p15:guide id="1" orient="horz" pos="2160">
                    <p15:clr>
                        <a:srgbClr val="A4A3A4" />
                    </p15:clr>
                </p15:guide>
                <p15:guide id="2" pos="2880">
                    <p15:clr>
                        <a:srgbClr val="A4A3A4" />
                    </p15:clr>
                </p15:guide>
            </p15:sldGuideLst>
        </p:ext>
    </p:extLst>
</p:presentation>
"#,
    );
    xml
}

fn presentation_relationships(slide_count: usize) -> String {
    let mut xml = String::new();
    xml.push_str(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId3"
    Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/presProps"
    Target="presProps.xml" />
<Relationship Id="rId4"
    Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/viewProps"
    Target="viewProps.xml" />
<Relationship Id="rId5"
    Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme"
    Target="theme/theme1.xml" />
<Relationship Id="rId6"
    Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/tableStyles"
    Target="tableStyles.xml" />
<Relationship Id="rId1"
    Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster"
    Target="slideMasters/slideMaster1.xml" />
    "#,
    );
    for index in 0..slide_count {
        xml.push_str(
            r#"
    <Relationship Id=""#,
        );
        xml.push_str(&xml_escape(&(format!("rId{}", 7 + index)).to_string()));
        xml.push_str(r#"" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/"#);
        xml.push_str(&xml_escape(&(format!("slide{}", index + 1)).to_string()));
        xml.push_str(
            r#".xml" />
    "#,
        );
    }
    xml.push_str(
        r#"
</Relationships>
"#,
    );
    xml
}

fn slide_xml(
    left: u32,
    top: u32,
    width: u32,
    height: u32,
    slide_number: usize,
    board_id: &str,
    title_names: &[String],
    include_nav: bool,
    links: &[LinkRegion],
) -> String {
    let mut xml = String::new();
    xml.push_str(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
    xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"
    xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
    <p:cSld>
        <p:spTree>
            <p:nvGrpSpPr>
                <p:cNvPr id="1" name="" />
                <p:cNvGrpSpPr />
                <p:nvPr />
            </p:nvGrpSpPr>
            <p:grpSpPr>
                <a:xfrm>
                    <a:off x="0" y="0" />
                    <a:ext cx="0" cy="0" />
                    <a:chOff x="0" y="0" />
                    <a:chExt cx="0" cy="0" />
                </a:xfrm>
            </p:grpSpPr>
            <p:pic>
                <p:nvPicPr>
                    <p:cNvPr id="2" name=""#,
    );
    xml.push_str(&xml_escape(&(board_id).to_string()));
    xml.push_str(r#"" descr=""#);
    xml.push_str(&xml_escape(&(board_id).to_string()));
    xml.push_str(
        r#"" />
                    <p:cNvPicPr>
                        <a:picLocks noChangeAspect="1" />
                    </p:cNvPicPr>
                    <p:nvPr />
                </p:nvPicPr>
                <p:blipFill>
                    <a:blip r:embed=""#,
    );
    xml.push_str(&xml_escape(
        &(format!("slide{slide_number}Image")).to_string(),
    ));
    xml.push_str(
        r#"" />
                    <a:stretch>
                        <a:fillRect />
                    </a:stretch>
                </p:blipFill>
                <p:spPr>
                    <a:xfrm>
                        <a:off x=""#,
    );
    xml.push_str(&xml_escape(&(left).to_string()));
    xml.push_str(r#"" y=""#);
    xml.push_str(&xml_escape(&(top).to_string()));
    xml.push_str(
        r#"" />
                        <a:ext cx=""#,
    );
    xml.push_str(&xml_escape(&(width).to_string()));
    xml.push_str(r#"" cy=""#);
    xml.push_str(&xml_escape(&(height).to_string()));
    xml.push_str(
        r#"" />
                    </a:xfrm>
                    <a:prstGeom prst="rect">
                        <a:avLst />
                    </a:prstGeom>
                </p:spPr>
            </p:pic>
            "#,
    );
    if include_nav {
        xml.push_str(
            r#"
            <p:sp>
                <p:nvSpPr>
                    <p:cNvPr id="95" name=""#,
        );
        xml.push_str(&xml_escape(&(board_id).to_string()));
        xml.push_str(
            r#"" />
                    <p:cNvSpPr txBox="1" />
                    <p:nvPr />
                </p:nvSpPr>
                <p:spPr>
                    <a:xfrm>
                        <a:off x="4001" y="6239" />
                        <a:ext cx="9135998" cy=""#,
        );
        xml.push_str(&xml_escape(&(HEADER_HEIGHT).to_string()));
        xml.push_str(r#"" />
                    </a:xfrm>
                    <a:prstGeom prst="rect">
                        <a:avLst />
                    </a:prstGeom>
                    <a:ln w="12700">
                        <a:miter lim="400000" />
                    </a:ln>
                    <a:extLst>
                        <a:ext uri="{C572A759-6A51-4108-AA02-DFA0A04FC94B}">
                            <ma14:wrappingTextBoxFlag
                                xmlns:ma14="http://schemas.microsoft.com/office/mac/drawingml/2011/main"
                                xmlns="" val="1" />
                        </a:ext>
                    </a:extLst>
                </p:spPr>
                <p:txBody>
                    <a:bodyPr lIns="45719" rIns="45719">
                        <a:spAutoFit />
                    </a:bodyPr>
                    <a:lstStyle>
                        <a:lvl1pPr>
                            <a:defRPr sz="2400" />
                        </a:lvl1pPr>
                    </a:lstStyle>
                    <a:p>
                        "#);
        for (index, name) in title_names
            .iter()
            .take(title_names.len().saturating_sub(1))
            .enumerate()
        {
            let relationship_id = format!("navLink{index}");
            xml.push_str(
                r#"
                        <a:r>
                            <a:rPr>
                                <a:hlinkClick r:id=""#,
            );
            xml.push_str(&xml_escape(&(relationship_id).to_string()));
            xml.push_str(
                r#"" invalidUrl=""
                                    action="ppaction://hlinksldjump" tgtFrame="" tooltip=""
                                    history="1" highlightClick="0" endSnd="0" />
                            </a:rPr>
                            <a:t>"#,
            );
            xml.push_str(&xml_escape(&(name).to_string()));
            xml.push_str(
                r#"</a:t>
                        </a:r>
                        <a:r><a:t>  /  </a:t></a:r>
                        "#,
            );
        }
        xml.push_str(
            r#"
                        <a:r>
                            <a:rPr b="1" />
                            <a:t>"#,
        );
        xml.push_str(&xml_escape(
            &(title_names.last().map(String::as_str).unwrap_or("")).to_string(),
        ));
        xml.push_str(
            r#"</a:t>
                        </a:r>
                    </a:p>
                </p:txBody>
            </p:sp>
            "#,
        );
    }
    xml.push_str(
        r#"
            "#,
    );
    for (index, link) in links.iter().enumerate() {
        let relationship_id = format!("link{}", index + 1);
        let name = &link.tooltip;
        xml.push_str(
            "\n            <p:sp>\n                <p:nvSpPr>\n                    <p:cNvPr id=\"",
        );
        xml.push_str(&xml_escape(&(index).to_string()));
        xml.push_str(r#"" name=""#);
        xml.push_str(&xml_escape(&(name).to_string()));
        xml.push_str(
            r#"">
                        <a:hlinkClick r:id=""#,
        );
        xml.push_str(&xml_escape(&(relationship_id).to_string()));
        xml.push_str(r#"" action=""#);
        xml.push_str(&xml_escape(
            &(if link.target_slide.is_some() {
                "ppaction://hlinksldjump"
            } else {
                ""
            })
            .to_string(),
        ));
        xml.push_str(r#"" tooltip=""#);
        xml.push_str(&xml_escape(&(name).to_string()));
        xml.push_str(
            r#"" history="1" invalidUrl=""
                            tgtFrame="" highlightClick="0" endSnd="0" />
                    </p:cNvPr>
                    <p:cNvSpPr />
                    <p:nvPr />
                </p:nvSpPr>
                <p:spPr>
                    <a:xfrm>
                        <a:off x=""#,
        );
        xml.push_str(&xml_escape(&(link.left).to_string()));
        xml.push_str(r#"" y=""#);
        xml.push_str(&xml_escape(&(link.top).to_string()));
        xml.push_str(
            r#"" />
                        <a:ext cx=""#,
        );
        xml.push_str(&xml_escape(&(link.width).to_string()));
        xml.push_str(r#"" cy=""#);
        xml.push_str(&xml_escape(&(link.height).to_string()));
        xml.push_str(
            r#"" />
                    </a:xfrm>
                    <a:prstGeom prst="rect">
                        <a:avLst />
                    </a:prstGeom>
                    <a:solidFill>
                        <a:srgbClr val="FFFFFF">
                            <a:alpha val="0" />
                        </a:srgbClr>
                    </a:solidFill>
                    <a:ln w="12700">
                        <a:miter lim="400000" />
                    </a:ln>
                </p:spPr>
            </p:sp>
            "#,
        );
    }
    xml.push_str(
        r#"
        </p:spTree>
    </p:cSld>
    <p:clrMapOvr>
        <a:masterClrMapping />
    </p:clrMapOvr>
</p:sld>
"#,
    );
    xml
}

fn slide_relationships(
    slide_number: usize,
    board_path: &[String],
    page_map: &std::collections::BTreeMap<&str, usize>,
    links: &[LinkRegion],
) -> String {
    let relationships = links
        .iter()
        .enumerate()
        .map(|(index, link)| {
            (
                format!("link{}", index + 1),
                if link.target_slide.is_some() {
                    ""
                } else {
                    link.href.as_str()
                },
                link.target_slide.unwrap_or(0),
            )
        })
        .chain(board_path.iter().enumerate().map(|(index, board_id)| {
            (
                format!("navLink{index}"),
                "",
                page_map.get(board_id.as_str()).copied().unwrap_or(0),
            )
        }));
    let relationship_id = format!("slide{slide_number}Image");
    let mut xml = String::new();
    xml.push_str(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
    <Relationship Id="rId1"
        Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout"
        Target="../slideLayouts/slideLayout7.xml" />
    <Relationship Id=""#,
    );
    xml.push_str(&xml_escape(&(relationship_id).to_string()));
    xml.push_str(
        r#""
        Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image"
        Target="../media/"#,
    );
    xml.push_str(&xml_escape(
        &(format!("slide{slide_number}Image")).to_string(),
    ));
    xml.push_str(
        r#".png" />
    "#,
    );
    for (relationship_id, external_url, target_page) in relationships {
        xml.push_str(
            r#"
    "#,
        );
        if !external_url.is_empty() {
            xml.push_str(
                r#"
    <Relationship Id=""#,
            );
            xml.push_str(&xml_escape(&(relationship_id).to_string()));
            xml.push_str(r#"" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target=""#);
            xml.push_str(&xml_escape(&(external_url).to_string()));
            xml.push_str(
                r#"" TargetMode="External" />
    "#,
            );
        } else {
            xml.push_str(
                r#"
    <Relationship Id=""#,
            );
            xml.push_str(&xml_escape(&(relationship_id).to_string()));
            xml.push_str(r#"" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slide"#);
            xml.push_str(&xml_escape(&(target_page).to_string()));
            xml.push_str(
                r#".xml" />
    "#,
            );
        }
        xml.push_str(
            r#"
    "#,
        );
    }
    xml.push_str(
        r#"
</Relationships>
"#,
    );
    xml
}

fn xml_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&#34;"),
            '\'' => escaped.push_str("&#39;"),
            '\t' => escaped.push_str("&#x9;"),
            '\n' => escaped.push_str("&#xA;"),
            '\r' => escaped.push_str("&#xD;"),
            '\u{0}'..='\u{8}'
            | '\u{b}'
            | '\u{c}'
            | '\u{e}'..='\u{1f}'
            | '\u{fffe}'
            | '\u{ffff}' => escaped.push('\u{fffd}'),
            other => escaped.push(other),
        }
    }
    escaped
}

fn core_properties(properties: &PptxDocumentProperties) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties
    xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties"
    xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/"
    xmlns:dcmitype="http://purl.org/dc/dcmitype/"
    xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
    <dc:title>{title}</dc:title>
    <dc:subject>{subject}</dc:subject>
    <dc:creator>{creator}</dc:creator>
    <cp:keywords />
    <dc:description>{description}</dc:description>
    <cp:lastModifiedBy>{creator}</cp:lastModifiedBy>
    <cp:revision>1</cp:revision>
    <cp:category />
</cp:coreProperties>
"#,
        title = xml_escape(&properties.title),
        subject = xml_escape(&properties.subject),
        creator = xml_escape(&properties.creator),
        description = xml_escape(&properties.description),
    )
}

fn app_properties(board_paths: &[Vec<String>], version: &str) -> String {
    let titles = board_paths
        .iter()
        .map(|path| {
            format!(
                "\n            <vt:lpstr>{}</vt:lpstr>\n            ",
                xml_escape(path.last().map(String::as_str).unwrap_or(""))
            )
        })
        .collect::<String>();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"
    xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes">
    <TotalTime>1</TotalTime>
    <Words>0</Words>
    <Application>D2</Application>
    <PresentationFormat>On-screen Show (16:9)</PresentationFormat>
    <Paragraphs>0</Paragraphs>
    <Slides>{slide_count}</Slides>
    <Notes>0</Notes>
    <HiddenSlides>0</HiddenSlides>
    <MMClips>0</MMClips>
    <ScaleCrop>false</ScaleCrop>
    <HeadingPairs>
        <vt:vector size="6" baseType="variant">
            <vt:variant>
                <vt:lpstr>Fonts</vt:lpstr>
            </vt:variant>
            <vt:variant>
                <vt:i4>2</vt:i4>
            </vt:variant>
            <vt:variant>
                <vt:lpstr>Theme</vt:lpstr>
            </vt:variant>
            <vt:variant>
                <vt:i4>1</vt:i4>
            </vt:variant>
            <vt:variant>
                <vt:lpstr>Slide Titles</vt:lpstr>
            </vt:variant>
            <vt:variant>
                <vt:i4>{slide_count}</vt:i4>
            </vt:variant>
        </vt:vector>
    </HeadingPairs>
    <TitlesOfParts>
        <vt:vector size="{titles_count}" baseType="lpstr">
            <vt:lpstr>Arial</vt:lpstr>
            <vt:lpstr>Calibri</vt:lpstr>
            <vt:lpstr>Office Theme</vt:lpstr>
            {titles}
        </vt:vector>
    </TitlesOfParts>
    <LinksUpToDate>false</LinksUpToDate>
    <SharedDoc>false</SharedDoc>
    <HyperlinkBase></HyperlinkBase>
    <HyperlinksChanged>false</HyperlinksChanged>
    <AppVersion>{version}</AppVersion>
</Properties>
"#,
        slide_count = board_paths.len(),
        titles_count = board_paths.len() + 3,
        version = xml_escape(version),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn generated_parts_match_complete_go_templates_with_and_without_navigation() {
        // Generated by recipe renderexport-public-pptx (harvested from
        // witness-inputs/run31-pptx-parts.go) against pinned Go.
        // Complete XML comparisons cover every style, content type, escaped value,
        // link ID/action/hit region and ordered navigation relationship.
        let root = vec!["root".to_owned()];
        let child = vec!["root".to_owned(), "root.layers.\"<&π".to_owned()];
        let page_map = [(root[0].as_str(), 1), (child[1].as_str(), 2)]
            .into_iter()
            .collect();
        let links = vec![
            LinkRegion {
                left: 20,
                top: 30,
                width: 40,
                height: 50,
                href: "https://example.com/?q=\"<&π".into(),
                tooltip: "URL <&π>".into(),
                target_slide: None,
            },
            LinkRegion {
                left: 60,
                top: 70,
                width: 80,
                height: 90,
                href: "".into(),
                tooltip: "child".into(),
                target_slide: Some(2),
            },
        ];
        for (name, nav) in [("plain", false), ("nav", true)] {
            // Regenerated by recipe renderexport-public-pptx (docs/FIXTURES.md).
            let fixture = format!("tests/fixtures/pptx-parts/{name}");
            let (width, height, top) = fit_image_for_slide(170, 100, nav);
            let left = (SLIDE_WIDTH - width) / 2;
            let parts = [
                ("[Content_Types].xml", content_types_xml(2)),
                ("_rels/.rels", root_relationships().to_owned()),
                ("ppt/presentation.xml", presentation_xml(2)),
                (
                    "ppt/_rels/presentation.xml.rels",
                    presentation_relationships(2),
                ),
                (
                    "ppt/slides/slide1.xml",
                    slide_xml(
                        left,
                        top,
                        width,
                        height,
                        1,
                        &root[0],
                        &["Root & <π>".into()],
                        nav,
                        &links,
                    ),
                ),
                (
                    "ppt/slides/slide2.xml",
                    slide_xml(
                        left,
                        top,
                        width,
                        height,
                        2,
                        &child[1],
                        &["Root & <π>".into(), "Child \"<&π".into()],
                        nav,
                        &[],
                    ),
                ),
                (
                    "ppt/slides/_rels/slide1.xml.rels",
                    slide_relationships(1, &root, &page_map, &links),
                ),
                (
                    "ppt/slides/_rels/slide2.xml.rels",
                    slide_relationships(2, &child, &page_map, &[]),
                ),
            ];
            for (part, actual) in parts {
                assert_eq!(
                    actual,
                    d2_rs_testdata::load_str(env!("CARGO_MANIFEST_DIR"), &format!("{fixture}/{part}")),
                    "{name}/{part}"
                );
            }
        }
    }

    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn document_properties_match_complete_pinned_go_parts() {
        // Original lib/pptx templates, generated by recipe
        // renderexport-public-pptx (from witness-inputs/run30-pptx-properties.go). The control
        // covers XML escaping and the complete ordered board-title vector.
        let properties = PptxDocumentProperties {
            title: "Title & <deck> \"'\t\n\r\0 π".into(),
            description: "Description & <text>".into(),
            subject: "Subject ' \"".into(),
            creator: "Creator & <name>".into(),
            version: "0.9.0".into(),
        };
        assert_eq!(
            core_properties(&properties),
            d2_rs_testdata::oracle_fixture!("tests/fixtures/pptx-properties-core.xml")
        );
        assert_eq!(
            app_properties(
                &[vec!["root".into()], vec!["root.layers.\"<&π".into()]],
                "0.9.0"
            ),
            d2_rs_testdata::oracle_fixture!("tests/fixtures/pptx-properties-app.xml")
        );
    }
}

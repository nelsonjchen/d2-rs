// SPDX-License-Identifier: MPL-2.0
// d2scenebuild.markdownFontFamilies and textmeasure.LayoutMarkdown use the
// diagram's selected faces for primitive advances as well as glyph painting.
use super::{MarkdownFamily, TrueTypeFont, bundled_markdown_measurer, markdown_measure};
use d2_rs_text::d2fonts::{CustomFonts, FONT_STYLES, FontStyle};
use std::{borrow::Cow, cell::RefCell, sync::Arc};

type Faces = [TrueTypeFont; 4];
#[derive(Default)]
struct Custom {
    primary: Option<Faces>,
    mono: Option<Faces>,
}
thread_local! {
    static CURRENT: RefCell<Option<Arc<Custom>>> = const { RefCell::new(None) };
}

pub(super) struct Scope(Option<Arc<Custom>>);
impl Scope {
    pub(super) fn new(fonts: &CustomFonts) -> Self {
        fn parse(faces: &std::collections::BTreeMap<FontStyle, Vec<u8>>) -> Option<Faces> {
            if faces.is_empty() {
                return None;
            }
            Some(FONT_STYLES.map(|style| {
                let fallback = d2_rs_text::d2fonts::font_face(
                    d2_rs_text::d2fonts::FontFamily::SourceSansPro.font(style),
                );
                faces
                    .get(&style)
                    .and_then(|bytes| TrueTypeFont::parse(bytes).ok())
                    .unwrap_or_else(|| TrueTypeFont::parse(fallback).expect("bundled face"))
            }))
        }
        let custom = (!fonts.regular.is_empty() || !fonts.mono.is_empty()).then(|| {
            Arc::new(Custom {
                primary: parse(&fonts.regular),
                mono: parse(&fonts.mono),
            })
        });
        Self(CURRENT.with(|current| current.replace(custom)))
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|current| current.replace(self.0.take()));
    }
}

pub(super) struct Fonts([Cow<'static, TrueTypeFont>; 8]);
impl Fonts {
    pub(super) fn measurer(&self) -> markdown_measure::MarkdownMeasurer<'_> {
        let [
            regular,
            bold,
            semibold,
            italic,
            mono,
            mono_bold,
            mono_semibold,
            mono_italic,
        ] = &self.0;
        markdown_measure::MarkdownMeasurer {
            font: regular,
            bold_font: bold,
            semibold_font: semibold,
            italic_font: italic,
            mono_font: mono,
            mono_bold_font: mono_bold,
            mono_semibold_font: mono_semibold,
            mono_italic_font: mono_italic,
        }
    }
}

pub(super) fn native_markdown_measurer(family: MarkdownFamily) -> Option<Fonts> {
    let bundled = bundled_markdown_measurer(family)?;
    let mut fonts = Fonts(
        [
            bundled.font,
            bundled.bold_font,
            bundled.semibold_font,
            bundled.italic_font,
            bundled.mono_font,
            bundled.mono_bold_font,
            bundled.mono_semibold_font,
            bundled.mono_italic_font,
        ]
        .map(Cow::Borrowed),
    );
    CURRENT.with_borrow(|current| {
        if let Some(custom) = current {
            let primary = if family.uses_mono_css() {
                &custom.mono
            } else {
                &custom.primary
            };
            for (offset, faces) in [(0, primary), (4, &custom.mono)] {
                if let Some(faces) = faces {
                    for (index, face) in faces.iter().enumerate() {
                        fonts.0[offset + index] = Cow::Owned(face.clone());
                    }
                }
            }
        }
    });
    Some(fonts)
}

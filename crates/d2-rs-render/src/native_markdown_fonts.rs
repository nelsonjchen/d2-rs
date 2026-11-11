// SPDX-License-Identifier: MPL-2.0
// d2scenebuild.markdownFontFamilies and textmeasure.LayoutMarkdown use the
// diagram's selected faces for primitive advances as well as glyph painting.
use super::{MarkdownFamily, TrueTypeFont, bundled_markdown_measurer, markdown_measure};
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

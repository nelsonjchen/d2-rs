// SPDX-License-Identifier: MPL-2.0
use d2_rs_text::d2fonts::{self, FONT_STYLES, FontFamily, FontStyle};

#[test]
fn omitted_registered_styles_use_source_sans_pro() {
    let name = "rust-registration-fallback";
    assert!(!d2fonts::is_registered_family(name));
    assert_eq!(
        d2fonts::add_font_family(name, None, None, None, None).unwrap(),
        name
    );
    assert!(d2fonts::is_registered_family(name));
    for style in FONT_STYLES {
        assert_eq!(
            d2fonts::registered_encoded_subset(name, style, "Aβ中 😀"),
            FontFamily::SourceSansPro
                .font(style)
                .get_encoded_subset("Aβ中 😀")
        );
    }
}

#[test]
fn supplied_face_and_reregistration_replace_the_same_name() {
    let name = "rust-registration-replace";
    let mono = FontFamily::SourceCodePro.font(FontStyle::Regular);
    d2fonts::add_font_family(name, Some(d2fonts::font_face(mono)), None, None, None).unwrap();
    assert!(d2fonts::is_registered_family(name));
    assert_eq!(
        d2fonts::registered_encoded_subset(name, FontStyle::Regular, "example"),
        mono.get_encoded_subset("example")
    );
    assert_eq!(
        d2fonts::registered_encoded_subset(name, FontStyle::Bold, "example"),
        FontFamily::SourceSansPro
            .font(FontStyle::Bold)
            .get_encoded_subset("example")
    );
    d2fonts::add_font_family(name, None, None, None, None).unwrap();
    assert_eq!(
        d2fonts::registered_encoded_subset(name, FontStyle::Regular, "example"),
        FontFamily::SourceSansPro
            .font(FontStyle::Regular)
            .get_encoded_subset("example")
    );
    assert!(
        d2fonts::registered_encoded_subset(
            "unregistered-rust-family",
            FontStyle::Regular,
            "example"
        )
        .is_none()
    );
}

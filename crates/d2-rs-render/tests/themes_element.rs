use d2_rs_render::{svg, themes::ThemableElement};

#[test]
fn typed_content_precedes_compatibility_content_even_when_empty() {
    let mut element = ThemableElement::new("g", None);
    element.content = "<title>legacy</title>".into();
    element.set_text("");
    assert_eq!(element.render(), "<g />");
    element.set_text("<&");
    assert_eq!(element.render(), "<g>&lt;&amp;</g>");
    element.set_inner_svg(svg::trusted_fragment("<title>trusted</title>"));
    assert_eq!(element.render(), "<g><title>trusted</title></g>");
}

#[test]
fn typed_attributes_and_local_ids_keep_order_and_copy_independently() {
    let mut element = ThemableElement::new("rect", None);
    element.mask = "invalid legacy mask".into();
    element.clip_path = "invalid legacy clip".into();
    element.set_mask_id(&svg::literal_id("mask"));
    element.set_clip_path_id(&svg::literal_id("clip"));
    element.add_attributes([svg::attr("data-a", "<&")]);
    element.attributes = "aria-label='label'".into();
    let mut copy = element.clone();
    copy.add_attributes([svg::attr("data-b", "second")]);
    assert_eq!(
        element.render(),
        "<rect mask=\"url(#mask)\" data-a=\"&lt;&amp;\" aria-label=\"label\" clip-path=\"url(#clip)\" />"
    );
    assert!(copy.render().contains("data-b=\"second\""));
    element.add_attributes([svg::attr("data-a", "duplicate")]);
    assert!(std::panic::catch_unwind(|| element.render()).is_err());
}

#[test]
fn nonfinite_radius_is_rejected_after_radius_calculation() {
    let mut element = ThemableElement::new("rect", None);
    element.rx = f64::NAN;
    element.width = 100.0;
    element.height = 100.0;
    assert!(std::panic::catch_unwind(|| element.render()).is_err());
}

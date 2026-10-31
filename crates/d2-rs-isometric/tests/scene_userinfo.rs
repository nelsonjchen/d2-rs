// SPDX-License-Identifier: MPL-2.0
//! scene_test.go: TestIconURLIdentityAndOwnership at master 6a7b48378.
use d2_rs_isometric::{scene::build_scene, target::{Diagram, Shape, Url, Userinfo}};

#[test]
fn scene_owns_icon_url_and_credentials() {
    let mut diagram = Diagram::default();
    diagram.shapes = Some(vec![Shape {
        id: "a".into(), type_: "rectangle".into(), width: 100, height: 70,
        opacity: 1.0, stroke_width: 2,
        icon: Some(Url {
            scheme: "https".into(), host: "example.test".into(), path: "/icon.svg".into(),
            raw_query: "variant=1".into(), user: Some(Userinfo::with_password("user", "pass")),
            ..Default::default()
        }),
        ..Default::default()
    }]);
    let mut scene = build_scene(&diagram, None).unwrap();
    let source = diagram.shapes()[0].icon.as_ref().unwrap();
    let owned = scene.nodes[0].metadata.original.icon.as_ref().unwrap();
    assert_eq!(scene.nodes[0].icon, source.string());
    assert_ne!(owned as *const Url, source as *const Url);
    let source_user = source.user.as_ref().unwrap();
    let owned_user = owned.user.as_ref().unwrap();
    assert_ne!(owned_user as *const Userinfo, source_user as *const Userinfo);
    assert_ne!(owned_user.username().as_ptr(), source_user.username().as_ptr());
    assert_ne!(owned_user.password().unwrap().as_ptr(), source_user.password().unwrap().as_ptr());
    scene.nodes[0].metadata.original.icon.as_mut().unwrap().user = Some(Userinfo::new("changed"));
    assert_eq!(source.string(), "https://user:pass@example.test/icon.svg?variant=1");
}

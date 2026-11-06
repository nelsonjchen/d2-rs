// SPDX-License-Identifier: MPL-2.0
//! gif_loop.go: private panel documents fit authored tracks to the GIF cycle.
use super::{context::RenderContext, svg::NativeScene};
use d2_rs_scene::Node;

pub(crate) fn loop_scene(
    context: &RenderContext,
    original: &NativeScene,
    loop_ns: i64,
) -> Result<NativeScene, String> {
    if loop_ns <= 0 || loop_ns > 600_000_000_000 {
        return Err("invalid native GIF loop".into());
    }
    // Nodes are owned values: unlike Go's pointers they cannot alias or cycle.
    // Check depth and cumulative work while cloning with an explicit stack.
    fn clone_node(
        context: &RenderContext,
        node: &Node,
        depth: usize,
        nodes: &mut usize,
        tracks: &mut usize,
        loop_ns: i64,
    ) -> Result<Node, String> {
        enum Work<'a> {
            Visit(&'a Node, usize),
            Finish(&'a Node),
        }
        let mut work = vec![Work::Visit(node, depth)];
        let mut completed = Vec::new();
        while let Some(item) = work.pop() {
            match item {
                Work::Visit(node, depth) => {
                    context.check_string()?;
                    if depth > 256 {
                        return Err("invalid native GIF animation tree".into());
                    }
                    let animations = node.animations.as_deref().unwrap_or_default();
                    if *nodes >= 200_000
                        || node.children().len() > 200_000
                        || animations.len() > 200_000 - *tracks
                    {
                        return Err("native GIF animation tree exceeds budget".into());
                    }
                    *nodes += 1;
                    *tracks += animations.len();
                    work.push(Work::Finish(node));
                    for child in node.children().iter().rev() {
                        work.push(Work::Visit(child, depth + 1));
                    }
                    if let Some(mask) = &node.mask {
                        let root = mask
                            .root
                            .as_ref()
                            .ok_or("invalid native GIF animation tree")?;
                        work.push(Work::Visit(root, depth + 1));
                    }
                }
                Work::Finish(node) => {
                    let count = node.children().len();
                    let children = completed.split_off(completed.len() - count);
                    let mask = node.mask.as_ref().map(|mask| d2_rs_scene::Mask {
                        r#type: mask.r#type,
                        transform: mask.transform,
                        root: Some(std::sync::Arc::new(
                            completed.pop().expect("mask root was visited"),
                        )),
                    });
                    let mut n = Node {
                        id: node.id.clone(),
                        classes: node.classes.clone(),
                        transform: node.transform,
                        opacity: node.opacity,
                        blend: node.blend,
                        clip: node.clip.clone(),
                        mask,
                        filters: node.filters.clone(),
                        primitive: node.primitive.clone(),
                        children: Some(children),
                        animations: node.animations.clone(),
                    };
                    for track in n.animations.iter_mut().flatten() {
                        if track.repeat && track.duration > 0 {
                            let cycles =
                                ((loop_ns as f64 / track.duration as f64).round() as i64).max(1);
                            track.duration = (loop_ns / cycles).max(1);
                        }
                    }
                    completed.push(n);
                }
            }
        }
        Ok(completed.pop().expect("root was visited"))
    }

    let mut out = NativeScene {
        triangles: original.triangles.clone(),
        camera: original.camera,
        width: original.width,
        height: original.height,
        background: original.background,
        background_paint: original.background_paint.clone(),
        links: original.links.clone(),
        packets: original.packets.clone(),
        animated_nodes: original.animated_nodes.clone(),
        pixel_scale: original.pixel_scale,
        panels: Vec::new(),
    };
    let (mut nodes, mut tracks) = (0, 0);
    for panel in &original.panels {
        if !panel.animated {
            out.panels.push(panel.clone());
            continue;
        }
        let doc = &panel.document;
        let root = doc
            .root
            .as_ref()
            .ok_or("invalid native GIF animation tree")?;
        let document = d2_rs_scene::Document {
            view_box: doc.view_box,
            logical_width: doc.logical_width,
            logical_height: doc.logical_height,
            viewport_fit: doc.viewport_fit,
            viewport_align: doc.viewport_align,
            assets: doc.assets.clone(),
            links: doc.links.clone(),
            root: Some(Box::new(clone_node(
                context,
                root,
                0,
                &mut nodes,
                &mut tracks,
                loop_ns,
            )?)),
        };
        out.panels.push(super::source_panels::NativePanel {
            document,
            width: panel.width,
            height: panel.height,
            first: panel.first,
            last: panel.last,
            animated: true,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::source_panels::NativePanel;
    use super::*;
    use d2_rs_scene::{Document, Mask, Track};
    #[test]
    fn loop_timing_copies_masks_and_preserves_authored_documents() {
        let mask = Node {
            animations: Some(vec![Track {
                repeat: true,
                duration: 2_300_000_000,
                ..Default::default()
            }]),
            ..Default::default()
        };
        let root = Node {
            mask: Some(Mask {
                root: Some(std::sync::Arc::new(mask)),
                ..Default::default()
            }),
            ..Default::default()
        };
        let original = NativeScene {
            panels: vec![NativePanel {
                animated: true,
                document: Document::new(Default::default(), root),
                ..Default::default()
            }],
            ..Default::default()
        };
        let before = serde_json::to_value(&original.panels[0].document).unwrap();
        let looped = loop_scene(&RenderContext::default(), &original, 8_000_000_000).unwrap();
        let duration = looped.panels[0]
            .document
            .root
            .as_ref()
            .unwrap()
            .mask
            .as_ref()
            .unwrap()
            .root
            .as_ref()
            .unwrap()
            .animations
            .as_ref()
            .unwrap()[0]
            .duration;
        assert_eq!(duration, 8_000_000_000 / 3);
        assert_eq!(
            serde_json::to_value(&original.panels[0].document).unwrap(),
            before
        );
    }
    #[test]
    fn loop_clone_observes_cancellation_depth_and_track_budgets() {
        let mut root = Node::default();
        for _ in 0..258 {
            root = Node {
                children: Some(vec![root]),
                ..Default::default()
            };
        }
        let mut original = NativeScene {
            panels: vec![NativePanel {
                animated: true,
                document: Document::new(Default::default(), root),
                ..Default::default()
            }],
            ..Default::default()
        };
        let context = RenderContext::default();
        assert_eq!(
            loop_scene(&context, &original, 8_000_000_000)
                .err()
                .unwrap(),
            "invalid native GIF animation tree"
        );
        original.panels[0].document.root = Some(Box::new(Node {
            animations: Some(vec![Track::default(); 200_001]),
            ..Default::default()
        }));
        assert_eq!(
            loop_scene(&context, &original, 8_000_000_000)
                .err()
                .unwrap(),
            "native GIF animation tree exceeds budget"
        );
        let token = d2_rs_core::CancellationToken::default();
        token.cancel();
        assert_eq!(
            loop_scene(&RenderContext::from_token(&token), &original, 8_000_000_000)
                .err()
                .unwrap(),
            "context canceled"
        );
        assert_eq!(
            loop_scene(&context, &original, 0).err().unwrap(),
            "invalid native GIF loop"
        );
    }
}

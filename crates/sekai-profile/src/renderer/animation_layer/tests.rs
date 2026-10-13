use super::{dynamic_canvas_expansion, LayerDynamic, LayerDynamicFrame};

fn line_indent(offsets: &[(f32, f32)]) -> LayerDynamic {
    LayerDynamic::TmpLineIndent {
        fps: 60,
        looped: false,
        frames: offsets
            .iter()
            .enumerate()
            .map(|(index, &(dx, dy))| LayerDynamicFrame {
                frame: index as u32,
                dx,
                dy,
            })
            .collect(),
    }
}

#[test]
fn expansion_covers_travel_that_only_enters_the_canvas_later() {
    // The layer sits outside the canvas at frame 0 and travels inward. An
    // expansion derived from the frame-0 position alone would crop the
    // content away before it ever becomes visible.
    let expansion =
        dynamic_canvas_expansion(&line_indent(&[(0.0, 0.0), (-400.0, 0.0), (-900.0, 0.0)]));
    assert!(
        expansion.right >= 900,
        "travel toward the canvas must widen the surface, got {}",
        expansion.right,
    );
}

#[test]
fn expansion_covers_travel_that_only_crosses_the_canvas_midway() {
    // Both endpoints sit far on the same side of the canvas while the path
    // between them crosses it. Sampling only the first and last frame would
    // report no travel at all and blank every frame in between.
    let expansion =
        dynamic_canvas_expansion(&line_indent(&[(0.0, 0.0), (-1200.0, 0.0), (-40.0, 0.0)]));
    assert!(
        expansion.right >= 1200,
        "a midway crossing must widen the surface, got {}",
        expansion.right,
    );
}

#[test]
fn dynamic_canvas_expansion_covers_the_complete_motion_range() {
    let expansion = dynamic_canvas_expansion(&LayerDynamic::TmpLineIndent {
        fps: 60,
        looped: false,
        frames: vec![
            LayerDynamicFrame {
                frame: 0,
                dx: -6400.25,
                dy: 120.5,
            },
            LayerDynamicFrame {
                frame: 1,
                dx: 310.1,
                dy: -90.75,
            },
        ],
    });
    assert_eq!(expansion.left, 319);
    assert_eq!(expansion.right, 6409);
    assert_eq!(expansion.top, 129);
    assert_eq!(expansion.bottom, 99);
}

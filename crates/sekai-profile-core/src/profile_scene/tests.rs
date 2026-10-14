use std::collections::BTreeMap;

use super::*;
use crate::SemanticCommandPayload;

fn text_scene(text_type: i32) -> ResolvedProfileScene {
    let card = serde_json::from_value(serde_json::json!({
        "texts": [{
            "objectData": {
                "layer": 0,
                "lock": false,
                "visible": true,
                "position": { "x": 12.0, "y": 24.0, "z": 0.0 },
                "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }
            },
            "colorId": 1,
            "fontId": 1,
            "lineSpacing": 0.0,
            "outlineColorId": 2,
            "outlineSize": 0.0,
            "size": 24.0,
            "text": "Left default",
            "type": text_type
        }]
    }))
    .unwrap();
    let snapshot = ProfileResolveSnapshot {
        fonts: BTreeMap::from([(1, "RegionFont".into())]),
        colors: BTreeMap::from([(1, [1.0; 4]), (2, [0.0; 4])]),
        ..ProfileResolveSnapshot::default()
    };
    resolve_profile_scene(&card, "text-alignment", &snapshot).unwrap()
}

#[test]
fn missing_horizontal_alignment_resolves_to_the_explicit_left_scene() {
    let left = text_scene(1);
    assert_eq!(left.commands.len(), 1);
    for text_type in [0, 8, 512, 1024] {
        assert_eq!(text_scene(text_type), left, "text_type={text_type}");
    }
}

#[test]
fn nonzero_horizontal_flags_are_preserved_when_vertical_flags_are_removed() {
    for alignment in 1..=7 {
        let scene = text_scene(512 | alignment);
        assert_eq!(scene, text_scene(alignment));
        assert!(matches!(
            scene.commands[0].payload,
            SemanticCommandPayload::Text { alignment: value, .. }
                if i32::from(value) == alignment
        ));
    }
}

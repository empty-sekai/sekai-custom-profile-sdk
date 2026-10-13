use super::*;

fn layer(id: u64, dynamic: bool) -> LayerSource {
    LayerSource {
        id: StableId(id),
        parent_id: None,
        kind: LayerKind::Text,
        authored_kind: AuthoredElementKind::Text,
        authored_index: id as u32,
        game_layer: id as i32,
        z: id as i32,
        authored_visible: true,
        source_content: "<line-indent=25%>abc".into(),
        resolved_parameters: BTreeMap::new(),
        bounds: Rect::default(),
        quad: [[0.0; 2]; 4],
        matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        hit_geometry: [[0.0; 2]; 4],
        line_indent: dynamic.then(|| LineIndentSource {
            percent: 25.0,
            line_advances_tmp: vec![vec![20.0, 20.0, 20.0]],
            rotation_deg: 0.0,
            scale_x: 1.0,
            alignment: 1,
        }),
    }
}

fn scene() -> Scene {
    Scene::new(SceneSource {
        scene_id: StableId(9),
        region: "cn".into(),
        font_engine_fingerprint: "ft".into(),
        raster_contract: "sdf".into(),
        layers: vec![layer(1, true), layer(2, false)],
        glyphs: Vec::new(),
        semantic_commands: Vec::new(),
        interaction_regions: Vec::new(),
        component_controls: Vec::new(),
    })
    .unwrap()
}

#[test]
fn animation_preflight_uses_compiled_visible_programs_not_source_text() {
    let dynamic_scene = scene();
    let descriptor = dynamic_scene.animation_preflight(480).unwrap();
    assert_eq!(descriptor.compiled_program_count, 1);
    assert_eq!(descriptor.observable_program_count, 1);
    assert_eq!(descriptor.observable_layer_ids, vec![StableId(1)]);
    assert!(descriptor.animated);

    let mut hidden_source = dynamic_scene.source.clone();
    hidden_source.layers[0].authored_visible = false;
    let hidden = Scene::new(hidden_source)
        .unwrap()
        .animation_preflight(480)
        .unwrap();
    assert_eq!(hidden.compiled_program_count, 1);
    assert_eq!(hidden.observable_program_count, 0);
    assert!(hidden.observable_layer_ids.is_empty());
    assert!(!hidden.animated);
}

#[test]
fn periodic_program_exposes_a_proven_timeline_descriptor() {
    let mut source = scene().source.clone();
    let dynamic = source.layers[0].line_indent.as_mut().unwrap();
    dynamic.percent = -100.0;
    dynamic.line_advances_tmp = vec![vec![48.0; 6]];
    let mut scene = Scene::new(source).unwrap();
    scene.advance_to_tick(8);
    let dynamic = scene.state(StableId(1)).unwrap().dynamic.as_ref().unwrap();
    assert_eq!(dynamic.status, DynamicStatus::Periodic);
    assert_eq!(
        dynamic.timeline,
        Some(TimelineDescriptor {
            loop_start_tick: 0,
            period_ticks: 2,
        })
    );
}

#[test]
fn negative_full_percent_keeps_emitting_the_two_cycle_after_long_runtime() {
    let mut source = scene().source.clone();
    let dynamic = source.layers[0].line_indent.as_mut().unwrap();
    dynamic.percent = -100.0;
    dynamic.line_advances_tmp = vec![vec![48.0; 6]];
    let mut scene = Scene::new(source).unwrap();

    scene.advance_to_tick(1_000);
    let first = scene
        .state(StableId(1))
        .unwrap()
        .dynamic
        .as_ref()
        .unwrap()
        .clone();
    let next = scene.advance_to_tick(1_001);
    let second = scene
        .state(StableId(1))
        .unwrap()
        .dynamic
        .as_ref()
        .unwrap()
        .clone();
    let repeated = scene.advance_to_tick(1_002);
    let third = scene.state(StableId(1)).unwrap().dynamic.as_ref().unwrap();

    assert_eq!(first.status, DynamicStatus::Periodic);
    assert_eq!(second.status, DynamicStatus::Periodic);
    assert_ne!(first.transform, second.transform);
    assert_eq!(first.transform, third.transform);
    assert_eq!(next.patches.len(), 1);
    assert_eq!(repeated.patches.len(), 1);
}

#[test]
fn positive_percent_emits_initial_patches_then_settles() {
    let mut source = scene().source.clone();
    let dynamic = source.layers[0].line_indent.as_mut().unwrap();
    dynamic.percent = 97.0;
    dynamic.line_advances_tmp = vec![vec![24.0; 6]];
    let mut scene = Scene::new(source).unwrap();

    let initial = scene.advance_to_tick(1);
    assert_eq!(initial.patches.len(), 1);
    assert_eq!(
        scene
            .state(StableId(1))
            .unwrap()
            .dynamic
            .as_ref()
            .unwrap()
            .status,
        DynamicStatus::Running
    );

    scene.advance_to_tick(2_000);
    assert_eq!(
        scene
            .state(StableId(1))
            .unwrap()
            .dynamic
            .as_ref()
            .unwrap()
            .status,
        DynamicStatus::Settled
    );
    assert!(scene.advance_to_tick(2_001).patches.is_empty());
}

#[test]
fn static_final_bootstrap_leaves_no_dynamic_program_running() {
    let mut scene = scene();
    let delta = scene.advance_to_static_final();
    let dynamic = scene.state(StableId(1)).unwrap().dynamic.as_ref().unwrap();

    assert!(scene.tick() > 0);
    assert!(matches!(
        dynamic.status,
        DynamicStatus::Settled | DynamicStatus::Periodic | DynamicStatus::Held
    ));
    assert!(delta.dirty.transform);

    scene.advance_to_tick(0);
    assert_eq!(scene.tick(), 0);
    assert_eq!(
        scene
            .state(StableId(1))
            .unwrap()
            .dynamic
            .as_ref()
            .unwrap()
            .status,
        DynamicStatus::Running
    );
}

#[test]
fn text_command_exposes_tmp_stripped_numeric_runs_in_schema() {
    let command = SemanticCommandSource::text(
        StableId(1),
        StableId(2),
        "value",
        "<color=#fff>12</color><b>34</b>-05",
        FontRole::RegionFontId(1),
    );
    assert_eq!(command.numeric_text_runs.len(), 2);
    assert_eq!(command.numeric_text_runs[0].text, "1234");
    assert_eq!(command.numeric_text_runs[0].plain_start, 0);
    assert_eq!(command.numeric_text_runs[0].plain_end, 4);
    assert_eq!(command.numeric_text_runs[1].text, "05");
}

#[test]
fn mask_toggle_does_not_reset_timeline_layout_command_or_atlas() {
    let mut scene = scene();
    scene.advance_to_tick(120);
    let before = scene.revisions();
    let state_before = scene.state(StableId(1)).unwrap().dynamic.clone();
    for index in 0..10_000 {
        scene.set_render_mask(StableId(1), index % 2 == 0).unwrap();
    }
    let after = scene.revisions();
    assert_eq!(scene.tick(), 120);
    assert_eq!(before.timeline, after.timeline);
    assert_eq!(before.local_layout, after.local_layout);
    assert_eq!(before.command, after.command);
    assert_eq!(before.atlas, after.atlas);
    assert_eq!(state_before, scene.state(StableId(1)).unwrap().dynamic);
}

#[test]
fn parent_mask_preserves_child_override_and_emits_only_resolved_changes() {
    let mut parent = layer(1, false);
    let mut child = layer(2, false);
    child.parent_id = Some(parent.id);
    parent.authored_visible = true;
    let mut scene = Scene::new(SceneSource {
        scene_id: StableId(10),
        region: "cn".into(),
        font_engine_fingerprint: "ft".into(),
        raster_contract: "sdf".into(),
        layers: vec![child, parent],
        glyphs: Vec::new(),
        semantic_commands: Vec::new(),
        interaction_regions: Vec::new(),
        component_controls: Vec::new(),
    })
    .unwrap();
    scene.set_render_mask(StableId(2), true).unwrap();
    let hidden = scene.set_render_mask(StableId(1), false).unwrap();
    assert_eq!(hidden.patches.len(), 2);
    let while_hidden = scene.set_render_mask(StableId(2), false).unwrap();
    assert!(while_hidden.dirty.mask && while_hidden.patches.is_empty());
    scene.set_render_mask(StableId(2), true).unwrap();
    let shown = scene.set_render_mask(StableId(1), true).unwrap();
    assert_eq!(shown.patches.len(), 2);
    assert!(scene.state(StableId(2)).unwrap().render_mask);
}

#[test]
fn layer_table_uses_dfs_slots_and_contiguous_subtree_intervals() {
    let mut root = layer(1, false);
    let mut child_a = layer(2, false);
    let mut grandchild = layer(3, false);
    let mut child_b = layer(4, false);
    let other_root = layer(5, false);
    root.authored_visible = true;
    child_a.parent_id = Some(root.id);
    grandchild.parent_id = Some(child_a.id);
    child_b.parent_id = Some(root.id);
    let scene = Scene::new(SceneSource {
        scene_id: StableId(13),
        region: "cn".into(),
        font_engine_fingerprint: "ft".into(),
        raster_contract: "sdf".into(),
        // Deliberately not in tree order.
        layers: vec![grandchild, child_b, other_root, child_a, root],
        glyphs: Vec::new(),
        semantic_commands: Vec::new(),
        interaction_regions: Vec::new(),
        component_controls: Vec::new(),
    })
    .unwrap();

    let table = scene.snapshot().layer_table;
    assert_eq!(
        table.iter().map(|entry| entry.layer_id).collect::<Vec<_>>(),
        vec![
            StableId(5),
            StableId(1),
            StableId(4),
            StableId(2),
            StableId(3)
        ]
    );
    let root_entry = table
        .iter()
        .find(|entry| entry.layer_id == StableId(1))
        .unwrap();
    assert_eq!((root_entry.subtree_start, root_entry.subtree_end), (1, 5));
    let child_entry = table
        .iter()
        .find(|entry| entry.layer_id == StableId(2))
        .unwrap();
    assert_eq!((child_entry.subtree_start, child_entry.subtree_end), (3, 5));
}

#[test]
fn bulk_mask_rejects_stale_layer_table_and_preserves_non_mask_revisions() {
    let mut root = layer(1, true);
    let mut child = layer(2, true);
    child.parent_id = Some(root.id);
    root.authored_visible = true;
    let mut scene = Scene::new(SceneSource {
        scene_id: StableId(14),
        region: "cn".into(),
        font_engine_fingerprint: "ft".into(),
        raster_contract: "sdf".into(),
        layers: vec![child, root],
        glyphs: Vec::new(),
        semantic_commands: Vec::new(),
        interaction_regions: Vec::new(),
        component_controls: Vec::new(),
    })
    .unwrap();
    scene.advance_to_tick(120);
    let before = scene.revisions();
    let stale = scene.set_render_mask_overrides(
        before.layer_table + 1,
        &[(StableId(1), RenderMaskOverride::ForceHidden)],
    );
    assert!(matches!(
        stale,
        Err(CoreError::StaleLayerTableRevision { .. })
    ));
    assert_eq!(scene.revisions(), before);

    let delta = scene
        .set_render_mask_overrides(
            before.layer_table,
            &[(StableId(1), RenderMaskOverride::ForceHidden)],
        )
        .unwrap();
    assert_eq!(delta.patches.len(), 2);
    let after = scene.revisions();
    assert_eq!(before.timeline, after.timeline);
    assert_eq!(before.local_layout, after.local_layout);
    assert_eq!(before.command, after.command);
    assert_eq!(before.atlas, after.atlas);
    assert_eq!(before.layer_table, after.layer_table);
}

#[test]
fn analytic_distance_field_preserves_curve_distance_and_winding() {
    use crate::sdf_geometry::{AnalyticDistanceField, QuadSeg, Segment, Vec2};

    let contour = vec![
        Segment::Quad(QuadSeg {
            p0: Vec2::new(0.0, 0.0),
            p1: Vec2::new(5.0, 10.0),
            p2: Vec2::new(10.0, 0.0),
        }),
        Segment::line(Vec2::new(10.0, 0.0), Vec2::new(0.0, 0.0)),
    ];
    let field = AnalyticDistanceField::new(&[contour]);
    assert!(field.signed_distance(Vec2::new(5.0, 2.0)) < 0.0);
    assert!(field.signed_distance(Vec2::new(5.0, 8.0)) > 0.0);
    assert!((field.signed_distance(Vec2::new(5.0, 5.0))).abs() <= 1e-4);
}

#[test]
fn invalid_layer_trees_fail_closed() {
    let mut orphan = layer(1, false);
    orphan.parent_id = Some(StableId(99));
    assert!(matches!(
        Scene::new(SceneSource {
            scene_id: StableId(11),
            region: "cn".into(),
            font_engine_fingerprint: "ft".into(),
            raster_contract: "sdf".into(),
            layers: vec![orphan],
            glyphs: Vec::new(),
            semantic_commands: Vec::new(),
            interaction_regions: Vec::new(),
            component_controls: Vec::new(),
        }),
        Err(CoreError::UnknownParent { .. })
    ));
    let mut first = layer(1, false);
    let mut second = layer(2, false);
    first.parent_id = Some(second.id);
    second.parent_id = Some(first.id);
    assert!(matches!(
        Scene::new(SceneSource {
            scene_id: StableId(12),
            region: "cn".into(),
            font_engine_fingerprint: "ft".into(),
            raster_contract: "sdf".into(),
            layers: vec![first, second],
            glyphs: Vec::new(),
            semantic_commands: Vec::new(),
            interaction_regions: Vec::new(),
            component_controls: Vec::new(),
        }),
        Err(CoreError::LayerTreeCycle(_))
    ));
}

#[test]
fn dynamic_updates_are_transform_only_and_rewind_is_deterministic() {
    let mut scene = scene();
    let initial = scene
        .state(StableId(1))
        .unwrap()
        .dynamic
        .as_ref()
        .unwrap()
        .transform;
    let first = scene.advance_to_tick(30);
    assert!(first.dirty.transform && !first.dirty.layout && !first.dirty.atlas);
    let transform = scene
        .state(StableId(1))
        .unwrap()
        .dynamic
        .as_ref()
        .unwrap()
        .transform;
    let reset = scene.advance_to_tick(0);
    assert_eq!(reset.patches[0].transform, Some(initial));
    scene.advance_to_tick(30);
    assert_eq!(
        transform,
        scene
            .state(StableId(1))
            .unwrap()
            .dynamic
            .as_ref()
            .unwrap()
            .transform
    );
}

#[test]
fn batched_advance_returns_latest_transform_and_same_tick_is_noop() {
    let mut scene = scene();
    let delta = scene.advance_to_tick(8);
    let current = scene
        .state(StableId(1))
        .unwrap()
        .dynamic
        .as_ref()
        .unwrap()
        .transform;
    assert_eq!(delta.patches.len(), 1);
    assert_eq!(delta.patches[0].transform, Some(current));
    let revisions = scene.revisions();
    let noop = scene.advance_to_tick(8);
    assert!(noop.patches.is_empty());
    assert_eq!(scene.revisions(), revisions);
}

#[test]
fn snapshot_round_trip_and_major_reject() {
    let mut scene = scene();
    let bytes = scene.encode_snapshot().unwrap();
    let decoded = Scene::decode_snapshot(&bytes).unwrap();
    assert_eq!(decoded.scene_id, StableId(9));
    let mut bad = decoded;
    bad.schema_major += 1;
    let bytes = bincode::encode_to_vec(bad, bincode::config::standard()).unwrap();
    assert!(matches!(
        Scene::decode_snapshot(&bytes),
        Err(CoreError::IncompatibleSchema { .. })
    ));
}

/// A scene that exercises every tagged value the snapshot carries:
/// each command payload and text source, every control binding and
/// control kind, every parameter value, clips, glyphs and a dynamic layer.
fn snapshot_fixture_scene() -> (Scene, StableId, StableId) {
    let parent_id = StableId(71);
    let child_id = StableId(72);
    let tab_id = StableId(7100);
    let scroll_id = StableId(7200);
    let parameters = BTreeMap::from([
        ("bool".to_string(), ParameterValue::Bool(true)),
        ("i64".to_string(), ParameterValue::I64(-7)),
        ("f64".to_string(), ParameterValue::F64(0.25)),
        ("text".to_string(), ParameterValue::Text("rank".into())),
        ("vec2".to_string(), ParameterValue::Vec2([3.0, 4.0])),
        (
            "color".to_string(),
            ParameterValue::Color([0.1, 0.2, 0.3, 1.0]),
        ),
    ]);
    let mut parent = layer(71, true);
    parent.kind = LayerKind::Composite;
    parent.authored_kind = AuthoredElementKind::General;
    parent.resolved_parameters = parameters.clone();
    let mut child = layer(72, false);
    child.parent_id = Some(parent_id);
    child.kind = LayerKind::Image;
    child.authored_kind = AuthoredElementKind::Other;
    let viewport = Rect {
        x: 0.0,
        y: 0.0,
        width: 40.0,
        height: 60.0,
    };
    let bounds = Rect {
        x: 2.0,
        y: 4.0,
        width: 20.0,
        height: 10.0,
    };

    let mut title = SemanticCommandSource::text(
        StableId(7101),
        parent_id,
        "title",
        "Rank 12",
        FontRole::RegionFontId(1),
    );
    title.render_placement = Some(TextRenderPlacementSource {
        anchor_x: -5.0,
        baseline: Some(3.5),
    });
    title
        .control_bindings
        .push(CommandControlBinding::TabOption {
            control_id: tab_id,
            value: "rank".into(),
        });
    let mut field = SemanticCommandSource::profile_text(
        StableId(7102),
        parent_id,
        "field",
        "userProfile.word",
        "hello 42",
        FontRole::RegionFontId(2),
    );
    field.clip = Some(crate::profile_scene::rect_quad(viewport));
    field
        .control_bindings
        .push(CommandControlBinding::ScrollContent {
            control_id: scroll_id,
        });
    let label = SemanticCommandSource::localized_text(
        StableId(7103),
        parent_id,
        "label",
        "general.title",
        "ja-JP",
        "タイトル",
        FontRole::RegionFontId(1),
    );
    let mut master = SemanticCommandSource::text(
        StableId(7104),
        parent_id,
        "master",
        "Stage 3",
        FontRole::RegionFontId(1),
    );
    if let SemanticCommandPayload::Text { source, .. } = &mut master.payload {
        *source = TextSource::MasterData {
            table: "musics".into(),
            key: "3".into(),
            value: "Stage 3".into(),
        };
    }
    let mut image = SemanticCommandSource::image(
        StableId(7201),
        child_id,
        "artwork",
        ResourceKey {
            namespace: "assets".into(),
            key: "other/artwork".into(),
        },
        bounds,
    );
    image.blend_mode = BlendMode::Multiply;
    image.metadata = parameters.clone();
    if let SemanticCommandPayload::Image {
        clip, alpha_mask, ..
    } = &mut image.payload
    {
        *clip = Some(ImageClip::RoundedRect { radius: [4.0, 4.0] });
        *alpha_mask = Some(ResourceKey {
            namespace: "static".into(),
            key: "mask/round".into(),
        });
    }
    let mut avatar = SemanticCommandSource::image(
        StableId(7202),
        child_id,
        "avatar",
        ResourceKey {
            namespace: "assets".into(),
            key: "other/avatar".into(),
        },
        bounds,
    );
    if let SemanticCommandPayload::Image { clip, .. } = &mut avatar.payload {
        *clip = Some(ImageClip::Ellipse);
    }
    let mut track = SemanticCommandSource::shape(
        StableId(7301),
        parent_id,
        "scroll-track",
        viewport,
        ShapePrimitive::RoundedRect { radius: [2.0, 2.0] },
    );
    track
        .control_bindings
        .push(CommandControlBinding::ScrollViewport {
            control_id: scroll_id,
        });
    let mut thumb = SemanticCommandSource::shape(
        StableId(7302),
        parent_id,
        "scroll-thumb",
        bounds,
        ShapePrimitive::Ellipse,
    );
    thumb
        .control_bindings
        .push(CommandControlBinding::ScrollThumb {
            control_id: scroll_id,
        });
    let masked = SemanticCommandSource::shape(
        StableId(7303),
        parent_id,
        "masked",
        bounds,
        ShapePrimitive::AssetMask {
            resource: ResourceKey {
                namespace: "static".into(),
                key: "shape/mask".into(),
            },
        },
    );
    let mut gradient = SemanticCommandSource::shape(
        StableId(7304),
        parent_id,
        "gradient",
        bounds,
        ShapePrimitive::Rect,
    );
    if let SemanticCommandPayload::Shape {
        gradient: target,
        stroke,
        stroke_width,
        ..
    } = &mut gradient.payload
    {
        *target = Some(LinearGradient {
            start: [0.0, 0.0],
            end: [1.0, 0.0],
            start_color: [1.0, 0.0, 0.0, 1.0],
            end_color: [0.0, 0.0, 1.0, 1.0],
        });
        *stroke = [0.0, 0.0, 0.0, 1.0];
        *stroke_width = 1.5;
    }
    let mut commands = vec![
        title, field, label, master, image, avatar, track, thumb, masked, gradient,
    ];
    for (index, operation) in [
        CompositeOperation::BeginIsolation,
        CompositeOperation::Marker,
        CompositeOperation::EndIsolation,
    ]
    .into_iter()
    .enumerate()
    {
        let mut composite = SemanticCommandSource::composite(
            StableId(7401 + index as u64),
            parent_id,
            format!("group-{index}"),
            bounds,
        );
        if let SemanticCommandPayload::Composite {
            operation: target,
            opacity,
            clip,
        } = &mut composite.payload
        {
            *target = operation;
            *opacity = 0.5;
            *clip = Some(crate::profile_scene::rect_quad(viewport));
        }
        commands.push(composite);
    }

    let region =
        |id: u64, role: &str, bindings: Vec<CommandControlBinding>| InteractionRegionSource {
            id: StableId(id),
            layer_id: parent_id,
            role: role.into(),
            bounds,
            quad: crate::profile_scene::rect_quad(bounds),
            matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            hit_geometry: crate::profile_scene::rect_quad(bounds),
            clip: Some(crate::profile_scene::rect_quad(viewport)),
            control_bindings: bindings,
            resolved_data: parameters.clone(),
            capabilities: vec!["inspect".into(), "activate".into()],
        };
    let scene = Scene::new(SceneSource {
        scene_id: StableId(70),
        region: "jp".into(),
        font_engine_fingerprint: "ft".into(),
        raster_contract: "sdf".into(),
        layers: vec![parent, child],
        glyphs: vec![GlyphSource {
            id: StableId(7501),
            layer_id: parent_id,
            output_ordinal: 0,
            source_span: [0, 1],
            bounds,
            quad: crate::profile_scene::rect_quad(bounds),
        }],
        semantic_commands: commands,
        interaction_regions: vec![
            region(
                7601,
                "rank-tab",
                vec![CommandControlBinding::TabOption {
                    control_id: tab_id,
                    value: "rank".into(),
                }],
            ),
            region(
                7602,
                "item",
                vec![CommandControlBinding::ScrollContent {
                    control_id: scroll_id,
                }],
            ),
        ],
        component_controls: vec![
            crate::profile_scene::ComponentControlSource {
                id: tab_id,
                layer_id: parent_id,
                role: "mode-tabs".into(),
                state: crate::profile_scene::ComponentControlState::Tabs {
                    options: vec!["rank".into(), "challenge".into()],
                    active: "rank".into(),
                },
            },
            crate::profile_scene::ComponentControlSource {
                id: scroll_id,
                layer_id: parent_id,
                role: "scroll".into(),
                state: crate::profile_scene::ComponentControlState::Scroll {
                    offset: 0.0,
                    min: 0.0,
                    max: 30.0,
                    viewport_extent: 60.0,
                    content_extent: 90.0,
                    step: 5.0,
                },
            },
        ],
    })
    .unwrap();
    (scene, tab_id, scroll_id)
}

#[test]
fn snapshot_round_trips_semantic_commands_controls_and_regions() {
    let (mut scene, tab_id, scroll_id) = snapshot_fixture_scene();
    scene.advance_to_tick(6);
    scene.set_tab(tab_id, "challenge").unwrap();
    scene.scroll_by(scroll_id, 10.0).unwrap();
    let expected = scene.snapshot();
    assert!(!expected.semantic_commands.is_empty());
    assert_eq!(expected.component_controls.len(), 2);
    assert_eq!(expected.interaction_regions.len(), 2);

    let bytes = scene.encode_snapshot().unwrap();
    let decoded = Scene::decode_snapshot(&bytes).unwrap();

    assert_eq!(decoded, expected);
}

#[test]
fn dump_exposes_source_parameters_geometry_and_mask() {
    let scene = scene();
    let dump = scene.dump();
    assert_eq!(dump.layers[0].source_content, "<line-indent=25%>abc");
    assert!(dump.layers[0].render_mask);
    assert_eq!(dump.coordinate_space, "card-device-v1");
}

#[test]
fn authored_layer_owns_ordered_commands_without_component_layers() {
    let layer_id = StableId(41);
    let commands = vec![
        SemanticCommandSource::shape(
            StableId(4101),
            layer_id,
            "panel-background",
            Rect::default(),
            ShapePrimitive::RoundedRect { radius: [8.0, 8.0] },
        ),
        SemanticCommandSource::text(
            StableId(4102),
            layer_id,
            "player-name",
            "<color=#ffffff>Player</color>",
            FontRole::RegionFontId(1),
        ),
        SemanticCommandSource::localized_text(
            StableId(4103),
            layer_id,
            "comment-title",
            "custom_profile.general.comment.title",
            "ja-JP",
            "ひとこと",
            FontRole::RegionFontId(1),
        ),
    ];
    let mut source = SceneSource {
        scene_id: StableId(40),
        region: "jp".into(),
        font_engine_fingerprint: "ft".into(),
        raster_contract: "sdf".into(),
        layers: vec![layer(41, false)],
        glyphs: Vec::new(),
        semantic_commands: commands,
        interaction_regions: vec![InteractionRegionSource {
            id: StableId(4199),
            layer_id,
            role: "player-name".into(),
            bounds: Rect::default(),
            quad: [[0.0; 2]; 4],
            matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            hit_geometry: [[0.0; 2]; 4],
            clip: None,
            control_bindings: Vec::new(),
            resolved_data: BTreeMap::new(),
            capabilities: vec!["inspect".into(), "select_text".into()],
        }],
        component_controls: Vec::new(),
    };
    source.layers[0].kind = LayerKind::Composite;
    let scene = Scene::new(source).unwrap();
    let snapshot = scene.snapshot();
    assert_eq!(snapshot.layer_table.len(), 1);
    assert_eq!(snapshot.layer_tree.len(), 1);
    assert_eq!(snapshot.commands[0].command_start, 0);
    assert_eq!(snapshot.commands[0].command_count, 3);
    assert_eq!(snapshot.semantic_commands.len(), 3);
    assert_eq!(snapshot.interaction_regions.len(), 1);
    assert_eq!(snapshot.interaction_regions[0].layer_id, layer_id);
    assert!(snapshot
        .semantic_commands
        .iter()
        .all(|command| command.layer_id == layer_id));
}

#[test]
fn component_controls_patch_command_state_without_rebuilding_layout_atlas_or_timeline() {
    let layer_id = StableId(61);
    let tab_id = StableId(6100);
    let scroll_id = StableId(6200);
    let mut first = SemanticCommandSource::text(
        StableId(6101),
        layer_id,
        "rank",
        "1",
        FontRole::RegionFontId(1),
    );
    first
        .control_bindings
        .push(CommandControlBinding::TabOption {
            control_id: tab_id,
            value: "rank".into(),
        });
    let mut challenge = SemanticCommandSource::text(
        StableId(6102),
        layer_id,
        "challenge",
        "2",
        FontRole::RegionFontId(1),
    );
    challenge
        .control_bindings
        .push(CommandControlBinding::TabOption {
            control_id: tab_id,
            value: "challenge".into(),
        });
    let mut content = SemanticCommandSource::shape(
        StableId(6201),
        layer_id,
        "scroll-content",
        Rect::default(),
        ShapePrimitive::Rect,
    );
    content
        .control_bindings
        .push(CommandControlBinding::ScrollContent {
            control_id: scroll_id,
        });
    let mut thumb = SemanticCommandSource::shape(
        StableId(6202),
        layer_id,
        "scroll-thumb",
        Rect {
            x: 0.0,
            y: 0.0,
            width: 8.0,
            height: 25.0,
        },
        ShapePrimitive::Rect,
    );
    thumb
        .control_bindings
        .push(CommandControlBinding::ScrollThumb {
            control_id: scroll_id,
        });
    let mut scene = Scene::new(SceneSource {
        scene_id: StableId(60),
        region: "cn".into(),
        font_engine_fingerprint: "ft".into(),
        raster_contract: "sdf".into(),
        layers: vec![layer(61, false)],
        glyphs: Vec::new(),
        semantic_commands: vec![first, challenge, content, thumb],
        interaction_regions: vec![
            InteractionRegionSource {
                id: StableId(6299),
                layer_id,
                role: "item".into(),
                bounds: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 20.0,
                    height: 20.0,
                },
                quad: crate::profile_scene::rect_quad(Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 20.0,
                    height: 20.0,
                }),
                matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                hit_geometry: crate::profile_scene::rect_quad(Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 20.0,
                    height: 20.0,
                }),
                clip: Some(crate::profile_scene::rect_quad(Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 20.0,
                    height: 100.0,
                })),
                control_bindings: vec![CommandControlBinding::ScrollContent {
                    control_id: scroll_id,
                }],
                resolved_data: BTreeMap::new(),
                capabilities: vec!["inspect".into()],
            },
            InteractionRegionSource {
                id: StableId(6199),
                layer_id,
                role: "rank-tab".into(),
                bounds: Rect {
                    x: 30.0,
                    y: 0.0,
                    width: 20.0,
                    height: 20.0,
                },
                quad: crate::profile_scene::rect_quad(Rect {
                    x: 30.0,
                    y: 0.0,
                    width: 20.0,
                    height: 20.0,
                }),
                matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                hit_geometry: crate::profile_scene::rect_quad(Rect {
                    x: 30.0,
                    y: 0.0,
                    width: 20.0,
                    height: 20.0,
                }),
                clip: None,
                control_bindings: vec![CommandControlBinding::TabOption {
                    control_id: tab_id,
                    value: "rank".into(),
                }],
                resolved_data: BTreeMap::new(),
                capabilities: vec!["activate".into()],
            },
            InteractionRegionSource {
                id: StableId(6298),
                layer_id,
                role: "scroll-viewport".into(),
                bounds: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 20.0,
                    height: 100.0,
                },
                quad: crate::profile_scene::rect_quad(Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 20.0,
                    height: 100.0,
                }),
                matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                hit_geometry: crate::profile_scene::rect_quad(Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 20.0,
                    height: 100.0,
                }),
                clip: None,
                control_bindings: vec![CommandControlBinding::ScrollViewport {
                    control_id: scroll_id,
                }],
                resolved_data: BTreeMap::new(),
                capabilities: vec!["scroll".into()],
            },
        ],
        component_controls: vec![
            crate::profile_scene::ComponentControlSource {
                id: tab_id,
                layer_id,
                role: "mode".into(),
                state: crate::profile_scene::ComponentControlState::Tabs {
                    options: vec!["rank".into(), "challenge".into()],
                    active: "rank".into(),
                },
            },
            crate::profile_scene::ComponentControlSource {
                id: scroll_id,
                layer_id,
                role: "list".into(),
                state: crate::profile_scene::ComponentControlState::Scroll {
                    offset: 0.0,
                    min: 0.0,
                    max: 100.0,
                    viewport_extent: 100.0,
                    content_extent: 200.0,
                    step: 20.0,
                },
            },
        ],
    })
    .unwrap();
    let baseline = scene.revisions();
    let initial = scene.snapshot();
    let item_region = initial
        .interaction_regions
        .iter()
        .find(|region| region.role == "item")
        .unwrap();
    assert_eq!(
        item_region.control_bindings,
        vec![CommandControlBinding::ScrollContent {
            control_id: scroll_id,
        }]
    );
    let tab_region = initial
        .interaction_regions
        .iter()
        .find(|region| region.role == "rank-tab")
        .unwrap();
    assert_eq!(
        tab_region.control_bindings,
        vec![CommandControlBinding::TabOption {
            control_id: tab_id,
            value: "rank".into(),
        }]
    );
    assert_eq!(
        scene.dump().interaction_regions,
        initial.interaction_regions
    );
    assert_eq!(
        initial
            .command_states
            .iter()
            .map(|state| state.render_mask)
            .collect::<Vec<_>>(),
        vec![true, false, true, true]
    );

    let tab = scene.set_tab(tab_id, "challenge").unwrap();
    assert_eq!(tab.command_patches.len(), 2);
    assert!(tab.dirty.material && !tab.dirty.layout && !tab.dirty.command && !tab.dirty.atlas);
    let scroll = scene.scroll_by(scroll_id, 30.0).unwrap();
    assert_eq!(scroll.command_patches.len(), 2);
    assert_eq!(scene.snapshot().command_states[2].transform.dy, -30.0);
    assert_eq!(scene.snapshot().command_states[3].transform.dy, 22.5);
    let after_controls = scene.snapshot();
    let region = after_controls
        .interaction_regions
        .iter()
        .find(|region| region.role == "item")
        .unwrap();
    assert_eq!(region.bounds.y, -30.0);
    assert_eq!(region.clip.unwrap()[0][1], 0.0);
    let viewport = after_controls
        .interaction_regions
        .iter()
        .find(|region| region.role == "scroll-viewport")
        .unwrap();
    assert_eq!(viewport.bounds.y, 0.0);
    assert_eq!(viewport.hit_geometry[0][1], 0.0);
    assert_eq!(
        viewport.control_bindings,
        vec![CommandControlBinding::ScrollViewport {
            control_id: scroll_id,
        }]
    );
    assert!(
        !after_controls
            .interaction_regions
            .iter()
            .find(|region| region.role == "rank-tab")
            .unwrap()
            .render_mask
    );
    let after = scene.revisions();
    assert_eq!(baseline.local_layout, after.local_layout);
    assert_eq!(baseline.command, after.command);
    assert_eq!(baseline.atlas, after.atlas);
    assert_eq!(baseline.timeline, after.timeline);

    scene.set_render_mask(layer_id, false).unwrap();
    scene.set_render_mask(layer_id, true).unwrap();
    assert_eq!(scene.snapshot().command_states[2].transform.dy, -30.0);
}

#[test]
fn interaction_snapshot_applies_authored_layer_transform_and_mask() {
    let layer_id = StableId(51);
    let mut authored_layer = layer(51, false);
    authored_layer.matrix = [2.0, 0.0, 0.0, 3.0, 10.0, 20.0];
    let local_quad = [[1.0, 2.0], [5.0, 2.0], [5.0, 6.0], [1.0, 6.0]];
    let source = SceneSource {
        scene_id: StableId(50),
        region: "cn".into(),
        font_engine_fingerprint: "ft".into(),
        raster_contract: "sdf".into(),
        layers: vec![authored_layer],
        glyphs: Vec::new(),
        semantic_commands: Vec::new(),
        interaction_regions: vec![InteractionRegionSource {
            id: StableId(5199),
            layer_id,
            role: "content".into(),
            bounds: Rect {
                x: 1.0,
                y: 2.0,
                width: 4.0,
                height: 4.0,
            },
            quad: local_quad,
            matrix: [1.0, 0.0, 0.0, 1.0, 1.0, 2.0],
            hit_geometry: local_quad,
            clip: None,
            control_bindings: Vec::new(),
            resolved_data: BTreeMap::new(),
            capabilities: vec!["inspect".into()],
        }],
        component_controls: Vec::new(),
    };
    let mut scene = Scene::new(source).unwrap();
    scene.set_render_mask(layer_id, false).unwrap();
    let region = &scene.snapshot().interaction_regions[0];
    assert_eq!(
        region.quad,
        [[12.0, 26.0], [20.0, 26.0], [20.0, 38.0], [12.0, 38.0]]
    );
    assert_eq!(
        region.bounds,
        Rect {
            x: 12.0,
            y: 26.0,
            width: 8.0,
            height: 12.0
        }
    );
    assert_eq!(region.matrix, [2.0, 0.0, 0.0, 3.0, 12.0, 26.0]);
    assert!(!region.render_mask);
}

#[test]
fn general_profile_layouts_are_shared_core_contracts() {
    use crate::profile_layout::*;
    let panels = [
        (2, &TOTAL_POWER, 5usize),
        (3, &DECK, 1),
        (4, &COMMENT, 3),
        (5, &LEADER_MEMBER, 4),
        (6, &HONORS, 4),
        (9, &MVP_SUPERSTAR, 6),
        (10, &CHALLENGE_LIVE, 5),
        (11, &CHAR_RANK, 6),
        (12, &MUSIC_CLEAR, 6),
        (13, &PLAYER_NAME, 2),
        (14, &STORY_FAVORITE, 10),
        (15, &CHAR_RANK, 6),
        (16, &MUSIC_CLEAR_TAB, 14),
    ];
    for (general_type, panel, elements) in panels {
        assert!(
            panel.w > 0.0 && panel.h > 0.0,
            "general type={general_type}"
        );
        assert_eq!(
            panel.elements.len(),
            elements,
            "general type={general_type}"
        );
    }
    assert_eq!((PLAYER_NAME.w, PLAYER_NAME.h), (610.0, 127.0));
    assert_eq!((COMMENT.w, COMMENT.h), (700.0, 251.0));
    assert_eq!((CHAR_RANK.w, CHAR_RANK.h), (967.0, 872.0));
    assert_eq!(
        (CHALLENGE_LIVE.elements[2].w, CHALLENGE_LIVE.elements[2].h),
        (92.0, 92.0),
        "the challenge avatar is a true circle; the former 87px height was a measurement error"
    );
}

#[test]
fn complete_profile_card_source_schema_lives_in_shared_core() {
    let object = serde_json::json!({
        "layer": 1, "lock": false,
        "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
        "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
        "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": true
    });
    let card: crate::profile_source::CustomProfileCard = serde_json::from_value(serde_json::json!({
        "texts": [{ "objectData": object, "colorId": 1, "fontId": 1, "lineSpacing": 0.0, "outlineColorId": 1, "outlineSize": 0.0, "size": 24.0, "text": "A", "type": 1 }],
        "shapes": [{ "objectData": object, "alpha": 1.0, "colorId": 1, "id": 1, "outlineAlpha": 1.0, "outlineColorId": 1, "outlineSize": 0.0 }],
        "cardMembers": [{ "objectData": object, "id": 1 }], "stamps": [{ "objectData": object, "id": 1 }],
        "others": [{ "objectData": object, "id": 1 }],
        "bondsHonors": [{ "objectData": object, "id": 1, "wordId": 1, "fullSize": true, "inverse": false, "useUnitVirtualSinger": false }],
        "honors": [{ "objectData": object, "id": 1, "fullSize": true }],
        "collections": [{ "objectData": object, "id": 1 }], "generals": [{ "objectData": object, "type": 4 }],
        "standMembers": [{ "objectData": object, "id": 1 }], "generalBackgrounds": [{ "objectData": object, "id": 1 }],
        "storyBackgrounds": [{ "objectData": object, "id": 1 }]
    })).unwrap();
    assert_eq!(card.element_count(), 12);
    assert_eq!(
        serde_json::to_value(&card).unwrap()["generals"][0]["type"],
        4
    );
}

#[test]
fn profile_transform_is_shared_and_uses_canonical_canvas() {
    let object = crate::profile_source::ObjectData {
        layer: 1,
        lock: false,
        position: crate::profile_source::Vec3 {
            x: 10.0,
            y: 20.0,
            z: 0.0,
        },
        rotation: crate::profile_source::Quaternion {
            w: 1.0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        scale: crate::profile_source::Vec3 {
            x: 2.0,
            y: 3.0,
            z: 1.0,
        },
        visible: true,
    };
    assert_eq!(
        crate::profile_transform::extract_transform(&object),
        (925.0, 386.0, -0.0, 2.0, 3.0)
    );
}

#[test]
fn profile_scene_normalization_preserves_authored_identity_and_game_order() {
    let object = |layer: i32| {
        serde_json::json!({
            "layer": layer, "lock": false,
            "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
            "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
            "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": true
        })
    };
    let card: crate::profile_source::CustomProfileCard = serde_json::from_value(serde_json::json!({
        "texts": [
            { "objectData": object(9), "colorId": 1, "fontId": 1, "lineSpacing": 0.0, "outlineColorId": 1, "outlineSize": 0.0, "size": 24.0, "text": "A", "type": 1 },
            { "objectData": object(2), "colorId": 1, "fontId": 1, "lineSpacing": 0.0, "outlineColorId": 1, "outlineSize": 0.0, "size": 24.0, "text": "B", "type": 1 }
        ],
        "generals": [{ "objectData": object(2), "type": 4 }]
    })).unwrap();
    let normalized = crate::profile_scene::ordered_profile_elements(&card, "document");
    assert_eq!(normalized.len(), 3);
    assert_eq!(
        normalized
            .iter()
            .map(|element| element.object().layer)
            .collect::<Vec<_>>(),
        vec![2, 2, 9]
    );
    assert_eq!(normalized[0].kind, AuthoredElementKind::Text);
    assert_eq!(normalized[0].source_index, 1);
    assert_eq!(normalized[1].kind, AuthoredElementKind::General);
    assert_ne!(normalized[0].layer_id, normalized[1].layer_id);
    assert_eq!(
        normalized,
        crate::profile_scene::ordered_profile_elements(&card, "document")
    );
}

#[test]
fn shared_profile_layer_assembly_owns_commands_and_geometry() {
    let card: crate::profile_source::CustomProfileCard =
        serde_json::from_value(serde_json::json!({
            "generals": [{
                "objectData": {
                    "layer": 7, "lock": false,
                    "position": { "x": 10.0, "y": 20.0, "z": 0.0 },
                    "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                    "scale": { "x": 2.0, "y": 3.0, "z": 1.0 }, "visible": true
                }, "type": 4
            }]
        }))
        .unwrap();
    let element = crate::profile_scene::ordered_profile_elements(&card, "document").remove(0);
    let command = SemanticCommandSource::shape(
        crate::profile_scene::semantic_command_id(&element.source_key, "background", 0),
        element.layer_id,
        "background",
        Rect {
            x: -10.0,
            y: -5.0,
            width: 20.0,
            height: 10.0,
        },
        ShapePrimitive::Rect,
    );
    let layer = crate::profile_scene::assemble_profile_layer(
        &element,
        LayerKind::Composite,
        String::new(),
        BTreeMap::new(),
        &[command],
        None,
    );
    assert_eq!(layer.game_layer, 7);
    assert_eq!(
        layer.bounds,
        Rect {
            x: -10.0,
            y: -5.0,
            width: 20.0,
            height: 10.0
        }
    );
    assert_eq!(layer.matrix, [2.0, -0.0, 0.0, 3.0, 925.0, 386.0]);
    assert_eq!(layer.authored_kind, AuthoredElementKind::General);
}

#[test]
fn identity_general_lowering_is_shared_and_region_font_one_only() {
    let snapshot = crate::profile_scene::ProfileComponentSnapshot {
        locale: "jp".into(),
        region_fonts: BTreeMap::from([(1, "FOT-Rodin".into())]),
        localized_text: BTreeMap::from([(
            "custom_profile.general.comment.title".into(),
            "ひとこと".into(),
        )]),
        user_name: "<b>Player</b>".into(),
        word: "<color=#ff0000>Bio</color>".into(),
        user_rank: 0,
        total_power: 0,
        mvp: 0,
        superstar: 0,
        challenge_score: 0,
        challenge_character_id: 0,
        challenge_avatar: None,
        music_results: None,
        story_favorites: Vec::new(),
        player_avatar: None,
        character_ranks: Vec::new(),
        deck_members: Vec::new(),
        leader_card: None,
        honor_slots: Vec::new(),
    };
    let layer_id = StableId(81);
    let name =
        crate::profile_scene::lower_identity_general(13, layer_id, "general-name", &snapshot)
            .unwrap()
            .unwrap();
    let comment =
        crate::profile_scene::lower_identity_general(4, layer_id, "general-comment", &snapshot)
            .unwrap()
            .unwrap();
    assert_eq!(name.commands.len(), 2);
    assert_eq!(comment.commands.len(), 3);
    assert_eq!(name.interaction_regions.len(), 1);
    assert_eq!(comment.interaction_regions.len(), 1);
    for command in name.commands.iter().chain(&comment.commands) {
        if let SemanticCommandPayload::Text { font_role, .. } = &command.payload {
            assert_eq!(font_role, &FontRole::RegionFontId(1));
        }
    }
    assert!(comment.commands.iter().any(|command| matches!(
        &command.payload,
        SemanticCommandPayload::Text {
            source: TextSource::Localized { key, locale, value }, ..
        } if key == "custom_profile.general.comment.title" && locale == "jp" && value == "ひとこと"
    )));
    let mut missing_font = snapshot.clone();
    missing_font.region_fonts.clear();
    assert!(matches!(
        crate::profile_scene::lower_identity_general(13, layer_id, "x", &missing_font),
        Err(crate::profile_scene::ProfileResolveError::MissingRegionFont(1))
    ));
}

#[test]
fn profile_text_regions_follow_their_command_tabs_scroll_and_viewport() {
    let snapshot = crate::profile_scene::ProfileComponentSnapshot {
        locale: "en-US".into(),
        region_fonts: BTreeMap::from([(1, "RegionFont".into())]),
        localized_text: BTreeMap::from([
            (
                "custom_profile.general.character_rank.title".into(),
                "Character rank".into(),
            ),
            (
                "custom_profile.general.character_rank.challenge".into(),
                "Challenge rank".into(),
            ),
        ]),
        character_ranks: (1..=26)
            .map(|character_id| crate::profile_scene::CharacterRankSnapshot {
                character_id,
                rank: 10 + character_id,
                challenge_rank: Some(20 + character_id),
                avatar: crate::profile_scene::ComponentImageSnapshot {
                    source_field: "userProfile.characterRanks".into(),
                    source_id: character_id.to_string(),
                    descriptor: None,
                },
            })
            .collect(),
        ..Default::default()
    };
    let layer_id = StableId(15);
    let lowering =
        crate::profile_scene::lower_identity_general(15, layer_id, "general:15", &snapshot)
            .unwrap()
            .unwrap();
    let roles = [
        "character-21-character_rank",
        "character-21-challenge_live_rank",
    ];
    for role in roles {
        let command = lowering
            .commands
            .iter()
            .find(|command| command.role == role)
            .unwrap();
        let region = lowering
            .interaction_regions
            .iter()
            .find(|region| region.role == role)
            .unwrap();
        assert_eq!(region.control_bindings, command.control_bindings);
        assert!(region.clip.is_some());
        assert_eq!(region.clip, command.clip);
    }

    let mut layer = layer(15, false);
    layer.kind = LayerKind::Composite;
    layer.authored_kind = AuthoredElementKind::General;
    let mut scene = Scene::new(SceneSource {
        scene_id: StableId(1500),
        region: "cn".into(),
        font_engine_fingerprint: "ft".into(),
        raster_contract: "sdf".into(),
        layers: vec![layer],
        glyphs: Vec::new(),
        semantic_commands: lowering.commands.clone(),
        interaction_regions: lowering.interaction_regions.clone(),
        component_controls: lowering.controls.clone(),
    })
    .unwrap();
    let find = |snapshot: &SceneSnapshot, role: &str| {
        snapshot
            .interaction_regions
            .iter()
            .find(|region| region.role == role)
            .cloned()
            .unwrap()
    };
    let before = scene.snapshot();
    let visible = roles
        .iter()
        .filter(|role| find(&before, role).render_mask)
        .count();
    assert_eq!(visible, 1);

    let scroll_id = lowering
        .controls
        .iter()
        .find(|control| {
            matches!(
                control.state,
                crate::profile_scene::ComponentControlState::Scroll { .. }
            )
        })
        .unwrap()
        .id;
    scene.scroll_by(scroll_id, 40.0).unwrap();
    let after = scene.snapshot();
    for role in roles {
        let (from, to) = (find(&before, role), find(&after, role));
        assert!(
            from.bounds.y - to.bounds.y > 0.0,
            "{role} follows the scroll"
        );
        assert_eq!(from.clip, to.clip, "{role} keeps the viewport clip");
    }
}

#[test]
fn zero_area_layer_regions_stay_at_the_layer_origin() {
    let card: crate::profile_source::CustomProfileCard =
        serde_json::from_value(serde_json::json!({
            "texts": [{
                "objectData": {
                    "layer": 1, "lock": false,
                    "position": { "x": 100.0, "y": 50.0, "z": 0.0 },
                    "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                    "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": true
                },
                "colorId": 1, "fontId": 1, "lineSpacing": 0.0, "outlineColorId": 1,
                "outlineSize": 0.0, "size": 24.0, "text": "A", "type": 1
            }]
        }))
        .unwrap();
    let snapshot = crate::profile_scene::ProfileResolveSnapshot {
        fonts: BTreeMap::from([(1, "RegionFont".into())]),
        colors: BTreeMap::from([(1, [0.0, 0.0, 0.0, 1.0])]),
        ..Default::default()
    };
    let resolved =
        crate::profile_scene::resolve_profile_scene(&card, "document", &snapshot).unwrap();
    let layer = resolved.layers[0].clone();
    assert_eq!(layer.bounds, Rect::default());
    assert_eq!(layer.hit_geometry, [[0.0; 2]; 4]);
    assert_eq!(layer.quad, [[0.0; 2]; 4]);

    let scene = Scene::new(SceneSource {
        scene_id: StableId(1600),
        region: "cn".into(),
        font_engine_fingerprint: "ft".into(),
        raster_contract: "sdf".into(),
        layers: resolved.layers,
        glyphs: Vec::new(),
        semantic_commands: resolved.commands,
        interaction_regions: resolved.interaction_regions,
        component_controls: resolved.controls,
    })
    .unwrap();
    let primary = scene
        .snapshot()
        .interaction_regions
        .into_iter()
        .find(|region| region.role == "primary")
        .unwrap();
    let origin = [layer.matrix[4], layer.matrix[5]];
    assert_eq!(primary.hit_geometry, [origin; 4]);
    assert_eq!(primary.quad, [origin; 4]);
    assert_eq!(
        primary.bounds,
        Rect {
            x: origin[0],
            y: origin[1],
            width: 0.0,
            height: 0.0,
        }
    );
}

#[test]
fn basic_general_lowering_has_profile_and_locale_provenance_without_component_layers() {
    let snapshot = crate::profile_scene::ProfileComponentSnapshot {
        locale: "ja-JP".into(),
        region_fonts: BTreeMap::from([(1, "FOT-Rodin".into())]),
        localized_text: crate::locale::GENERAL_LOCALIZATION_KEYS
            .iter()
            .map(|key| ((*key).into(), crate::locale::resolve("ja-JP", key).unwrap()))
            .collect(),
        user_name: String::new(),
        word: String::new(),
        user_rank: 100,
        total_power: 123_456,
        mvp: 12,
        superstar: 3,
        challenge_score: 765_432,
        challenge_character_id: 9,
        challenge_avatar: None,
        music_results: None,
        story_favorites: Vec::new(),
        player_avatar: None,
        character_ranks: Vec::new(),
        deck_members: Vec::new(),
        leader_card: None,
        honor_slots: Vec::new(),
    };
    for general_type in [2, 9, 10, 17] {
        let lowered = crate::profile_scene::lower_identity_general(
            general_type,
            StableId(91),
            &format!("general-{general_type}"),
            &snapshot,
        )
        .unwrap()
        .unwrap();
        assert!(lowered.commands.len() > 2);
        assert!(lowered
            .commands
            .iter()
            .all(|command| command.layer_id == StableId(91)));
        assert!(!lowered
            .commands
            .iter()
            .any(|command| matches!(command.payload, SemanticCommandPayload::Composite { .. })));
        assert!(lowered
            .commands
            .iter()
            .filter_map(|command| match &command.payload {
                SemanticCommandPayload::Text { font_role, .. } => Some(font_role),
                _ => None,
            })
            .all(|font| font == &FontRole::RegionFontId(1)));
    }
}

#[test]
fn music_general_lowering_is_typed_localized_and_preserves_append_gradient() {
    let difficulty = crate::profile_scene::MusicDifficultySnapshot {
        clear: 10,
        full_combo: 8,
        all_perfect: 3,
    };
    let snapshot = crate::profile_scene::ProfileComponentSnapshot {
        locale: "ko-KR".into(),
        region_fonts: BTreeMap::from([(1, "KoreanRegionFont".into())]),
        localized_text: crate::locale::GENERAL_LOCALIZATION_KEYS
            .iter()
            .map(|key| ((*key).into(), crate::locale::resolve("ko-KR", key).unwrap()))
            .collect(),
        user_name: String::new(),
        word: String::new(),
        user_rank: 0,
        total_power: 0,
        mvp: 0,
        superstar: 0,
        challenge_score: 0,
        challenge_character_id: 0,
        challenge_avatar: None,
        music_results: Some(crate::profile_scene::MusicResultsSnapshot {
            easy: difficulty,
            normal: difficulty,
            hard: difficulty,
            expert: difficulty,
            master: difficulty,
            append: difficulty,
        }),
        story_favorites: Vec::new(),
        player_avatar: None,
        character_ranks: Vec::new(),
        deck_members: Vec::new(),
        leader_card: None,
        honor_slots: Vec::new(),
    };
    for general_type in [12, 16] {
        let lowered = crate::profile_scene::lower_identity_general(
            general_type,
            StableId(92),
            &format!("music-{general_type}"),
            &snapshot,
        )
        .unwrap()
        .unwrap();
        assert!(lowered.commands.len() >= 24);
        assert!(!lowered
            .commands
            .iter()
            .any(|command| matches!(command.payload, SemanticCommandPayload::Composite { .. })));
        assert!(lowered
            .commands
            .iter()
            .filter_map(|command| match &command.payload {
                SemanticCommandPayload::Text { source, .. } => Some(source),
                _ => None,
            })
            .all(|source| !matches!(source, TextSource::Authored { .. })));
        if general_type == 16 {
            assert!(lowered.commands.iter().any(|command| matches!(
                command.payload,
                SemanticCommandPayload::Shape {
                    gradient: Some(_),
                    ..
                }
            )));
            assert!(matches!(
                &lowered.controls[0].state,
                crate::profile_scene::ComponentControlState::Tabs { options, active }
                    if options == &["clear", "full_combo", "all_perfect"] && active == "clear"
            ));
            assert_eq!(lowered.interaction_regions.len(), 21);
            assert_eq!(
                lowered
                    .interaction_regions
                    .iter()
                    .filter(|region| region.capabilities.contains(&"activate".into()))
                    .count(),
                3
            );
            assert_eq!(
                lowered
                    .interaction_regions
                    .iter()
                    .filter(|region| region.capabilities.contains(&"select_text".into()))
                    .count(),
                18
            );
            assert_eq!(
                lowered
                    .commands
                    .iter()
                    .filter(|command| !command.control_bindings.is_empty())
                    .count(),
                30
            );
        }
    }
}

#[test]
fn image_general_lowering_exposes_source_metadata_cover_uv_and_clip() {
    let descriptor = crate::profile_scene::ResourceDescriptor {
        resource: ResourceKey {
            namespace: "assets".into(),
            key: "fixture/wide".into(),
        },
        natural_width: 360.0,
        natural_height: 180.0,
        provenance: BTreeMap::from([("table".into(), ParameterValue::Text("cards".into()))]),
    };
    let image = crate::profile_scene::ComponentImageSnapshot {
        source_field: "userProfile.leaderCard".into(),
        source_id: "42".into(),
        descriptor: Some(descriptor.clone()),
    };
    let snapshot = crate::profile_scene::ProfileComponentSnapshot {
        locale: "en-US".into(),
        region_fonts: BTreeMap::from([(1, "RegionFont".into())]),
        localized_text: crate::locale::GENERAL_LOCALIZATION_KEYS
            .iter()
            .map(|key| ((*key).into(), crate::locale::resolve("en-US", key).unwrap()))
            .collect(),
        user_name: String::new(),
        word: String::new(),
        user_rank: 0,
        total_power: 0,
        mvp: 0,
        superstar: 0,
        challenge_score: 0,
        challenge_character_id: 0,
        challenge_avatar: None,
        music_results: None,
        story_favorites: vec![crate::profile_scene::StoryFavoriteSnapshot {
            story_id: 7,
            story_type: "event".into(),
            image: crate::profile_scene::ComponentImageSnapshot {
                source_field: "userProfile.storyFavorites".into(),
                source_id: "event:7".into(),
                descriptor: Some(descriptor.clone()),
            },
        }],
        player_avatar: Some(image),
        character_ranks: vec![crate::profile_scene::CharacterRankSnapshot {
            character_id: 21,
            rank: 50,
            challenge_rank: Some(12),
            avatar: crate::profile_scene::ComponentImageSnapshot {
                source_field: "userProfile.characterRanks".into(),
                source_id: "21".into(),
                descriptor: Some(descriptor.clone()),
            },
        }],
        deck_members: vec![crate::profile_scene::CardVisualSnapshot {
            card_id: 42,
            after_training: true,
            master_rank: 5,
            level: 60,
            rarity: "rarity_4".into(),
            attribute: "cool".into(),
            image: crate::profile_scene::ComponentImageSnapshot {
                source_field: "userProfile.deckMembers".into(),
                source_id: "42".into(),
                descriptor: Some(descriptor.clone()),
            },
        }],
        leader_card: Some(crate::profile_scene::CardVisualSnapshot {
            card_id: 42,
            after_training: true,
            master_rank: 5,
            level: 60,
            rarity: "rarity_4".into(),
            attribute: "cool".into(),
            image: crate::profile_scene::ComponentImageSnapshot {
                source_field: "userProfile.leaderCard".into(),
                source_id: "42".into(),
                descriptor: Some(descriptor.clone()),
            },
        }),
        honor_slots: vec![
            crate::profile_scene::HonorVisualSnapshot {
                source_field: "userProfile.honorSlots".into(),
                source_id: "1".into(),
                honor_id: 1,
                honor_level: 6,
                full_size: true,
                visual: crate::profile_scene::HonorVisualKind::Standard {
                    honor_type: "character".into(),
                    has_star: true,
                    is_live_master: false,
                    progress: 0,
                    background: Some(descriptor.clone()),
                    frame_candidates: vec![Some(descriptor.clone())],
                    overlay: Some(descriptor.clone()),
                    star: Some(descriptor.clone()),
                    star_high: Some(descriptor.clone()),
                    live_star_on: Some(descriptor.clone()),
                    live_star_off: Some(descriptor.clone()),
                },
            },
            crate::profile_scene::HonorVisualSnapshot {
                source_field: "userProfile.honorSlots".into(),
                source_id: "2".into(),
                honor_id: 2,
                honor_level: 3,
                full_size: false,
                visual: crate::profile_scene::HonorVisualKind::Bonds {
                    character_ids: [1, 2],
                    backgrounds: [Some(descriptor.clone()), Some(descriptor.clone())],
                    characters: [Some(descriptor.clone()), Some(descriptor.clone())],
                    mask: Some(descriptor.clone()),
                    frame: Some(descriptor.clone()),
                    word: Some(descriptor.clone()),
                    star: Some(descriptor.clone()),
                    star_high: Some(descriptor.clone()),
                },
            },
        ],
    };
    let story = crate::profile_scene::lower_identity_general(14, StableId(93), "story", &snapshot)
        .unwrap()
        .unwrap();
    let avatar =
        crate::profile_scene::lower_identity_general(18, StableId(94), "avatar", &snapshot)
            .unwrap()
            .unwrap();
    assert!(!story
        .commands
        .iter()
        .any(|command| matches!(command.payload, SemanticCommandPayload::Composite { .. })));
    let avatar_image = avatar
        .commands
        .iter()
        .find(|command| command.role == "avatar")
        .unwrap();
    assert_eq!(
        avatar_image.metadata.get("source_field"),
        Some(&ParameterValue::Text("userProfile.leaderCard".into()))
    );
    assert!(matches!(
        &avatar_image.payload,
        SemanticCommandPayload::Image {
            uv: Rect { x, width, .. },
            clip: Some(crate::ImageClip::Ellipse),
            ..
        } if (*x - 0.25).abs() < 1e-6 && (*width - 0.5).abs() < 1e-6
    ));
    let ranks = crate::profile_scene::lower_identity_general(11, StableId(95), "ranks", &snapshot)
        .unwrap()
        .unwrap();
    assert!(ranks
        .commands
        .iter()
        .any(|command| command.role == "character-21-avatar"));
    assert!(ranks.commands.iter().any(|command| matches!(
        &command.payload,
        SemanticCommandPayload::Text { source: TextSource::ProfileField { field, value }, .. }
            if field == "userProfile.characterRanks.21.rank" && value == "50"
    )));
    assert!(ranks.commands.iter().any(|command| matches!(
        &command.payload,
        SemanticCommandPayload::Text { source: TextSource::ProfileField { field, value }, .. }
            if field == "userProfile.challengeLiveSoloStages.21.rank" && value == "12"
    )));
    assert_eq!(
        ranks
            .commands
            .iter()
            .filter(|command| !command.control_bindings.is_empty())
            .count(),
        8
    );
    assert!(matches!(
        &ranks.controls[0].state,
        crate::profile_scene::ComponentControlState::Tabs { options, active }
            if options == &["character_rank", "challenge_live_rank"] && active == "character_rank"
    ));
    assert_eq!(ranks.interaction_regions.len(), 5);
    assert_eq!(
        ranks
            .interaction_regions
            .iter()
            .filter(|region| region.capabilities.contains(&"select_text".into()))
            .count(),
        2
    );
    for general_type in [3, 5] {
        let cards = crate::profile_scene::lower_identity_general(
            general_type,
            StableId(98),
            &format!("cards-{general_type}"),
            &snapshot,
        )
        .unwrap()
        .unwrap();
        assert!(cards.commands.len() >= 5);
        assert!(cards.commands.iter().any(|command| matches!(
            &command.payload,
            SemanticCommandPayload::Image { resource, .. } if resource.namespace == "static"
        )));
        assert!(cards
            .interaction_regions
            .iter()
            .all(|region| region.capabilities.contains(&"select_item".into())));
        assert!(!cards
            .commands
            .iter()
            .any(|command| matches!(command.payload, SemanticCommandPayload::Composite { .. })));
    }
    let honors = crate::profile_scene::lower_identity_general(6, StableId(99), "honors", &snapshot)
        .unwrap()
        .unwrap();
    assert_eq!(honors.interaction_regions.len(), 2);
    assert!(honors.commands.iter().any(|command| {
        command.blend_mode == crate::BlendMode::DstIn
            && matches!(
                &command.payload,
                SemanticCommandPayload::Image { resource, .. } if resource.key == "fixture/wide"
            )
    }));
    assert!(honors
        .commands
        .iter()
        .any(|command| matches!(command.payload, SemanticCommandPayload::Composite { .. })));

    let compact =
        crate::profile_scene::lower_identity_general(15, StableId(96), "compact-ranks", &snapshot)
            .unwrap()
            .unwrap();
    assert!(compact.controls.iter().any(|control| matches!(
        control.state,
        crate::profile_scene::ComponentControlState::Scroll { .. }
    )));
    assert!(compact.commands.iter().any(|command| command
        .control_bindings
        .iter()
        .any(|binding| matches!(binding, CommandControlBinding::ScrollContent { .. }))
        && command.clip.is_some()));
    let compact_background = compact
        .commands
        .iter()
        .find(|command| command.role == "character-21-background")
        .unwrap();
    assert!(compact_background.clip.is_some());
    let compact_avatar = compact
        .commands
        .iter()
        .find(|command| command.role == "character-21-avatar")
        .unwrap();
    assert!(compact_avatar.clip.is_some());
    assert!(matches!(
        compact_avatar.payload,
        SemanticCommandPayload::Image {
            clip: Some(crate::ImageClip::Ellipse),
            ..
        }
    ));
    assert!(compact
        .commands
        .iter()
        .find(|command| command.role == "character-21-character_rank")
        .unwrap()
        .clip
        .is_some());

    let mut scrolling_story_snapshot = snapshot.clone();
    let favorite = scrolling_story_snapshot.story_favorites[0].clone();
    scrolling_story_snapshot.story_favorites = (0..12)
        .map(|mut index| {
            let mut value = favorite.clone();
            index += 1;
            value.story_id = index;
            value.image.source_id = format!("event:{index}");
            value
        })
        .collect();
    let scrolling_story = crate::profile_scene::lower_identity_general(
        14,
        StableId(97),
        "scrolling-story",
        &scrolling_story_snapshot,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        scrolling_story
            .commands
            .iter()
            .filter(|command| command.role.starts_with("story-")
                && !command.role.contains("scroll"))
            .count(),
        12
    );
    assert!(scrolling_story.controls.iter().any(|control| matches!(
        control.state,
        crate::profile_scene::ComponentControlState::Scroll { max, .. } if max > 0.0
    )));
    assert!(scrolling_story.interaction_regions.iter().any(|region| {
        region.resolved_data.get("action") == Some(&ParameterValue::Text("scroll_by".into()))
    }));
}

#[test]
fn shared_profile_resolver_lowers_all_twelve_authored_types() {
    let object = serde_json::json!({
        "layer": 7, "lock": false,
        "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
        "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
        "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": true
    });
    let card: crate::profile_source::CustomProfileCard = serde_json::from_value(serde_json::json!({
        "texts": [{ "objectData": object, "colorId": 1, "fontId": 1, "lineSpacing": 0.0, "outlineColorId": 2, "outlineSize": 1.0, "size": 24.0, "text": "A", "type": 1 }],
        "shapes": [{ "objectData": object, "alpha": 0.5, "colorId": 1, "id": 1, "outlineAlpha": 1.0, "outlineColorId": 2, "outlineSize": 2.0 }],
        "cardMembers": [{ "objectData": object, "id": 1 }], "stamps": [{ "objectData": object, "id": 1 }],
        "others": [{ "objectData": object, "id": 1 }],
        "bondsHonors": [{ "objectData": object, "id": 1, "wordId": 1, "fullSize": true, "inverse": false, "useUnitVirtualSinger": false }],
        "honors": [{ "objectData": object, "id": 1, "fullSize": true }],
        "collections": [{ "objectData": object, "id": 1 }], "generals": [{ "objectData": object, "type": 9 }],
        "standMembers": [{ "objectData": object, "id": 1 }], "generalBackgrounds": [{ "objectData": object, "id": 1 }],
        "storyBackgrounds": [{ "objectData": object, "id": 1 }]
    })).unwrap();
    let mut snapshot = crate::profile_scene::ProfileResolveSnapshot {
        fonts: BTreeMap::from([(1, "RegionFont".into())]),
        colors: BTreeMap::from([(1, [1.0, 0.0, 0.0, 1.0]), (2, [0.0, 0.0, 0.0, 1.0])]),
        resources: [
            ("shape", "", "shape/resolved", 1024.0, 1024.0),
            ("card-member", "2:normal", "card/resolved", 156.0, 156.0),
            ("stamp", "", "stamp/resolved", 100.0, 100.0),
            ("other", "", "other/resolved", 120.0, 80.0),
            ("collection", "", "collection/resolved", 100.0, 100.0),
            ("stand-member", "", "standing/resolved", 300.0, 700.0),
            (
                "general-background",
                "",
                "general-bg/resolved",
                1830.0,
                812.0,
            ),
            ("story-background", "", "story-bg/resolved", 1830.0, 812.0),
        ]
        .into_iter()
        .map(|(kind, variant, key, natural_width, natural_height)| {
            (
                crate::profile_scene::resource_lookup_key(kind, 1, variant),
                crate::profile_scene::ResourceDescriptor {
                    resource: ResourceKey {
                        namespace: "assets".into(),
                        key: key.into(),
                    },
                    natural_width,
                    natural_height,
                    provenance: BTreeMap::from([(
                        "table".into(),
                        ParameterValue::Text(kind.into()),
                    )]),
                },
            )
        })
        .collect(),
        ..Default::default()
    };
    let honor_asset = crate::profile_scene::ResourceDescriptor {
        resource: ResourceKey {
            namespace: "static".into(),
            key: "honor/fixture".into(),
        },
        natural_width: 380.0,
        natural_height: 80.0,
        provenance: BTreeMap::new(),
    };
    for element in crate::profile_scene::ordered_profile_elements(&card, "document") {
        let visual = match element.value {
            crate::profile_scene::ProfileElementRef::Honor(value) => {
                Some(crate::profile_scene::HonorVisualSnapshot {
                    source_field: "customProfile.honors".into(),
                    source_id: value.id.to_string(),
                    honor_id: value.id,
                    honor_level: value.honor_level,
                    full_size: value.full_size,
                    visual: crate::profile_scene::HonorVisualKind::Standard {
                        honor_type: "achievement".into(),
                        has_star: true,
                        is_live_master: false,
                        progress: 0,
                        background: Some(honor_asset.clone()),
                        frame_candidates: vec![Some(honor_asset.clone())],
                        overlay: Some(honor_asset.clone()),
                        star: Some(honor_asset.clone()),
                        star_high: Some(honor_asset.clone()),
                        live_star_on: Some(honor_asset.clone()),
                        live_star_off: Some(honor_asset.clone()),
                    },
                })
            }
            crate::profile_scene::ProfileElementRef::BondsHonor(value) => {
                Some(crate::profile_scene::HonorVisualSnapshot {
                    source_field: "customProfile.bondsHonors".into(),
                    source_id: value.id.to_string(),
                    honor_id: value.id,
                    honor_level: value.honor_level,
                    full_size: value.full_size,
                    visual: crate::profile_scene::HonorVisualKind::Bonds {
                        character_ids: [1, 2],
                        backgrounds: [Some(honor_asset.clone()), Some(honor_asset.clone())],
                        characters: [Some(honor_asset.clone()), Some(honor_asset.clone())],
                        mask: Some(honor_asset.clone()),
                        frame: Some(honor_asset.clone()),
                        word: Some(honor_asset.clone()),
                        star: Some(honor_asset.clone()),
                        star_high: Some(honor_asset.clone()),
                    },
                })
            }
            _ => None,
        };
        if let Some(visual) = visual {
            snapshot.honor_visuals.insert(element.source_key, visual);
        }
    }
    let resolved =
        crate::profile_scene::resolve_profile_scene(&card, "document", &snapshot).unwrap();
    assert_eq!(resolved.layers.len(), 12);
    assert!(resolved.commands.len() > 12);
    assert!(resolved.interaction_regions.len() > 12);
    assert!(resolved.layers.iter().all(|layer| layer.game_layer == 7));
    assert!(resolved.commands.iter().all(|command| resolved
        .layers
        .iter()
        .any(|layer| layer.id == command.layer_id)));
    assert!(resolved.commands.iter().any(|command| matches!(
        &command.payload,
        SemanticCommandPayload::Image { resource, .. } if resource.key == "other/resolved"
    )));
    let honor_layer = resolved
        .layers
        .iter()
        .find(|layer| layer.authored_kind == AuthoredElementKind::Honor)
        .unwrap();
    assert!(resolved
        .commands
        .iter()
        .filter(|command| command.layer_id == honor_layer.id)
        .all(|command| !matches!(command.payload, SemanticCommandPayload::Composite { .. })));
    let bonds_layer = resolved
        .layers
        .iter()
        .find(|layer| layer.authored_kind == AuthoredElementKind::BondsHonor)
        .unwrap();
    let bonds_composites = resolved
        .commands
        .iter()
        .filter(|command| {
            command.layer_id == bonds_layer.id
                && matches!(command.payload, SemanticCommandPayload::Composite { .. })
        })
        .collect::<Vec<_>>();
    assert_eq!(bonds_composites.len(), 2);
    assert!(matches!(
        bonds_composites[0].payload,
        SemanticCommandPayload::Composite {
            operation: CompositeOperation::BeginIsolation,
            ..
        }
    ));
    assert!(matches!(
        bonds_composites[1].payload,
        SemanticCommandPayload::Composite {
            operation: CompositeOperation::EndIsolation,
            ..
        }
    ));
    let other_layer = resolved
        .layers
        .iter()
        .find(|layer| layer.authored_kind == AuthoredElementKind::Other)
        .unwrap();
    assert_eq!(other_layer.bounds.width, 120.0);
    assert_eq!(
        other_layer.resolved_parameters.get("resource_source.table"),
        Some(&ParameterValue::Text("other".into()))
    );
    let text = resolved
        .commands
        .iter()
        .find(|command| matches!(command.payload, SemanticCommandPayload::Text { .. }))
        .unwrap();
    assert!(matches!(
        &text.payload,
        SemanticCommandPayload::Text { font_role: FontRole::RegionFontId(1), color, outline_color, .. }
            if *color == [1.0, 0.0, 0.0, 1.0] && *outline_color == [0.0, 0.0, 0.0, 1.0]
    ));
}

#[test]
fn semantic_command_for_unknown_layer_fails_closed() {
    let source = SceneSource {
        scene_id: StableId(42),
        region: "cn".into(),
        font_engine_fingerprint: "ft".into(),
        raster_contract: "sdf".into(),
        layers: vec![layer(1, false)],
        glyphs: Vec::new(),
        semantic_commands: vec![SemanticCommandSource::text(
            StableId(4201),
            StableId(99),
            "orphan",
            "text",
            FontRole::RegionFontId(1),
        )],
        interaction_regions: Vec::new(),
        component_controls: Vec::new(),
    };
    assert!(matches!(
        Scene::new(source),
        Err(CoreError::UnknownCommandLayer { .. })
    ));
}

#[test]
fn warm_glyph_cache_does_not_regenerate() {
    let mut cache = SessionGlyphCache::default();
    let key = GlyphCacheKey {
        region: "cn".into(),
        font_digest: [7; 32],
        font_engine_fingerprint: "ft".into(),
        raster_contract: "sdf".into(),
        glyph_index: 12,
    };
    cache.get_or_insert_with(key.clone(), || GlyphCacheEntry {
        metrics: [0; 6],
        pixels: vec![1, 2, 3],
    });
    cache.get_or_insert_with(key, || panic!("warm glyph regenerated"));
    assert_eq!(
        cache.stats(),
        &GlyphCacheStats {
            hits: 1,
            misses: 1,
            generations: 1,
            bytes: 3
        }
    );
}

#[test]
fn materialized_dynamic_matches_reference_formula() {
    for percent in [0.0, 25.0, 50.0, 99.0, 100.0] {
        let source = LineIndentSource {
            percent,
            line_advances_tmp: vec![vec![17.5, 31.25, 22.0, 19.75]],
            rotation_deg: 0.0,
            scale_x: 1.0,
            alignment: 1,
        };
        let max_frames = if (0.0..100.0).contains(&percent) {
            20_000
        } else {
            1_800
        };
        let actual = materialize_line_indent(source.clone(), max_frames).unwrap();
        let expected = reference_materialize(&source, max_frames);
        assert_eq!(actual.looped, expected.0, "percent={percent}");
        assert_eq!(actual.frames.len(), expected.1.len(), "percent={percent}");
        for (actual, expected) in actual.frames.iter().zip(expected.1) {
            assert!(
                (actual.dx_local - expected).abs() <= 1e-5,
                "percent={percent}"
            );
        }
    }
    let divergent = materialize_line_indent(
        LineIndentSource {
            percent: 150.0,
            line_advances_tmp: vec![vec![17.5, 31.25, 22.0, 19.75]],
            rotation_deg: 0.0,
            scale_x: 1.0,
            alignment: 1,
        },
        1_800,
    )
    .unwrap();
    assert!(divergent
        .frames
        .iter()
        .all(|frame| frame.dx_local.is_finite()));
    assert!(divergent.frames.len() < 1_800);
}

#[test]
fn line_indent_shift_follows_horizontal_alignment() {
    // Around the centre pivot the first glyph sits at (pct - 0.5) * w when
    // left aligned, (pct * w - natural) / 2 when centred and w / 2 - natural
    // when right aligned, so a width change moves it by pct - 0.5, pct / 2
    // and 0.5 of that change respectively.
    let pct = 0.75;
    for (alignment, coefficient) in [(1, pct - 0.5), (0, pct - 0.5), (2, pct / 2.0), (4, 0.5)] {
        let mut runtime = LineIndentRuntime::new(LineIndentSource {
            percent: pct * 100.0,
            line_advances_tmp: vec![vec![20.0, 20.0, 20.0]],
            rotation_deg: 0.0,
            scale_x: 1.0,
            alignment,
        })
        .unwrap();
        for tick in 0..4 {
            assert_eq!(runtime.state.status, DynamicStatus::Running);
            let width = runtime.width_tmp;
            runtime.advance_one();
            let expected = coefficient * (width - runtime.static_width_tmp);
            assert!(
                (runtime.state.transform.dx - expected).abs() <= 1e-4,
                "alignment={alignment} tick={tick}: {} != {expected}",
                runtime.state.transform.dx
            );
            assert!(expected.abs() > CONVERGENCE_EPSILON);
        }
    }
}

#[test]
fn contracting_negative_percent_programs_settle_at_their_limit() {
    for (percent, line_advances_tmp) in [
        (-25.0, vec![vec![20.0, 20.0, 20.0]]),
        (-25.0, vec![vec![30.0, 10.0], vec![5.0, 5.0, 5.0, 50.0]]),
        (-50.0, vec![vec![48.0; 6]]),
        (-95.0, vec![vec![80.0, 80.0, 10.0, 80.0]]),
    ] {
        let source = LineIndentSource {
            percent,
            line_advances_tmp,
            rotation_deg: 0.0,
            scale_x: 1.0,
            alignment: 1,
        };
        let mut runtime = LineIndentRuntime::new(source.clone()).unwrap();
        for _ in 0..2_000 {
            if runtime.state.status != DynamicStatus::Running {
                break;
            }
            runtime.advance_one();
        }
        assert_eq!(
            runtime.state.status,
            DynamicStatus::Settled,
            "percent={percent}"
        );
        let mut reference = LineIndentRuntime::new(source).unwrap();
        for _ in 0..5_000 {
            reference.state.status = DynamicStatus::Running;
            reference.advance_one();
        }
        assert!(
            (runtime.state.transform.dx - reference.state.transform.dx).abs()
                <= CONVERGENCE_EPSILON,
            "percent={percent}: settled at {} but the loop ends at {}",
            runtime.state.transform.dx,
            reference.state.transform.dx
        );
    }
}

#[test]
fn two_cycle_reached_after_a_transient_is_periodic_from_its_first_repeat() {
    let mut runtime = LineIndentRuntime::new(LineIndentSource {
        percent: -100.0,
        line_advances_tmp: vec![vec![60.0, 30.0, 20.0, 20.0]],
        rotation_deg: 0.0,
        scale_x: 1.0,
        alignment: 1,
    })
    .unwrap();
    for _ in 0..16 {
        runtime.advance_one();
    }
    assert_eq!(runtime.state.status, DynamicStatus::Periodic);
    assert_eq!(
        runtime.state.timeline,
        Some(TimelineDescriptor {
            loop_start_tick: 1,
            period_ticks: 2,
        })
    );
}

#[test]
fn multiline_line_indent_feedback_uses_global_preferred_width() {
    let multiline = materialize_line_indent(
        LineIndentSource {
            percent: 50.0,
            line_advances_tmp: vec![vec![10.0], vec![20.0, 20.0]],
            rotation_deg: 0.0,
            scale_x: 1.0,
            alignment: 1,
        },
        512,
    )
    .unwrap();
    let widest_line = materialize_line_indent(
        LineIndentSource {
            percent: 50.0,
            line_advances_tmp: vec![vec![20.0, 20.0]],
            rotation_deg: 0.0,
            scale_x: 1.0,
            alignment: 1,
        },
        512,
    )
    .unwrap();

    assert_eq!(multiline, widest_line);
}

fn reference_materialize(source: &LineIndentSource, max_frames: usize) -> (bool, Vec<f32>) {
    let pct = source.percent / 100.0;
    let natural = source.line_advances_tmp[0].iter().sum::<f32>();
    let last = *source.line_advances_tmp[0].last().unwrap();
    let static_width = if pct < 1.0 {
        (natural + TMP_PAD) / (1.0 - pct)
    } else {
        natural + TMP_PAD
    };
    let mut width = TMP_SEED_WIDTH;
    let mut frames = Vec::new();
    for frame in 0..(WARMUP_TICKS as usize + max_frames) {
        let x_advance = natural + pct * width;
        let preferred = if x_advance >= last {
            x_advance
        } else {
            2.0 * last - x_advance
        };
        if frame >= WARMUP_TICKS as usize {
            let local = (pct - 0.5) * (width - static_width);
            frames.push(local);
            if (0.0..1.0).contains(&pct) && frames.len() >= 2 && local.abs() <= CONVERGENCE_EPSILON
            {
                break;
            }
        }
        width = preferred + TMP_PAD;
    }
    if frames.len() >= 4
        && frames
            .iter()
            .take(32)
            .enumerate()
            .all(|(index, value)| (*value - frames[index % 2]).abs() <= TWO_CYCLE_EPSILON)
    {
        return (true, vec![frames[0], frames[1]]);
    }
    (false, frames)
}

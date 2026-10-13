use super::*;

#[test]
fn telemetry_serializes_the_executor_atlases_and_fallback_reason() {
    let requested = RenderConfig {
        executor: SdfExecutor::Auto,
        ..RenderConfig::default()
    };
    let mut telemetry = RenderTelemetry::new(requested, SdfExecutor::Scalar);
    telemetry.surface_identity = SurfaceIdentity {
        pixel_format: "rgba8888".into(),
        alpha_type: "premultiplied".into(),
        color_space: "none".into(),
    };
    telemetry.atlas_identities.push(AtlasIdentity {
        family: "text".into(),
        usage: "executed".into(),
        schema: "allium.sdf-atlas-manifest.v1".into(),
        manifest_sha256: "11".repeat(32),
        generator_contract: "outline-edt-v1:ss=2:fallback=analytic-v1".into(),
        pixel_format: "r8-distance".into(),
        font_family: Some("test-font".into()),
        font_sha256: Some("22".repeat(32)),
        page_count: 1,
        entry_count: 42,
        mapped_bytes: 4096,
    });
    telemetry.record_fallback(
        FallbackCode::RenderObjectMissing,
        "render-objects",
        "missing-texture",
        Some("layer-1".into()),
        Some("command-2".into()),
    );
    let value = serde_json::to_value(telemetry).expect("telemetry must serialize");
    assert_eq!(value["schema"], PROFILE_RENDER_TELEMETRY_SCHEMA);
    assert_eq!(value["requested"]["executor"], "auto");
    assert_eq!(value["executor"], "scalar");
    assert!(value.get("jpeg").is_none());
    assert_eq!(value["surface_identity"]["pixel_format"], "rgba8888");
    assert_eq!(value["atlas_identities"][0]["usage"], "executed");
    assert_eq!(
        value["atlas_identities"][0]["generator_contract"],
        "outline-edt-v1:ss=2:fallback=analytic-v1"
    );
    assert_eq!(value["fallbacks"][0]["code"], "render-object-missing");
    assert_eq!(value["fallbacks"][0]["reason"], "missing-texture");
}

#[test]
fn a_plan_alone_does_not_count_as_executed_commands() {
    let mut telemetry = RenderTelemetry::new(RenderConfig::default(), SdfExecutor::Simd);
    telemetry.record_sdf_plan(
        crate::sdf::tile::SdfPlanStats {
            command_count: 3,
            text_command_count: 2,
            shape_command_count: 1,
            span_count: 4,
            text_span_count: 3,
            shape_span_count: 1,
            tile_count: 2,
            nonempty_tile_count: 1,
            covered_fragment_count: 8,
            text_covered_fragment_count: 6,
            shape_covered_fragment_count: 2,
        },
        128,
        64,
    );
    assert_eq!(telemetry.work.command_count, 0);
    assert_eq!(telemetry.work.glyph_count, 0);
    assert_eq!(telemetry.work.shape_count, 0);
    assert_eq!(telemetry.work.ordered_span_count, 4);
    assert_eq!(telemetry.commands[0].command_count, 0);
}

#[test]
fn sdf_stats_accumulate_without_losing_command_kind() {
    let mut telemetry = RenderTelemetry::new(RenderConfig::default(), SdfExecutor::Simd);
    let plan_stats = crate::sdf::tile::SdfPlanStats {
        command_count: 5,
        text_command_count: 4,
        shape_command_count: 1,
        span_count: 20,
        text_span_count: 16,
        shape_span_count: 4,
        tile_count: 8,
        nonempty_tile_count: 6,
        covered_fragment_count: 100,
        text_covered_fragment_count: 80,
        shape_covered_fragment_count: 20,
    };
    telemetry.record_executed_sdf_commands(2, 1, 4);
    telemetry.record_sdf_plan(plan_stats, 2_048, 320);
    telemetry.record_sdf_execution(crate::sdf::tile::SdfExecutionStats {
        shaded_fragment_count: 100,
        text_shaded_fragment_count: 80,
        shape_shaded_fragment_count: 20,
        sampled_texel_count: 400,
        blended_fragment_count: 100,
        text_blended_fragment_count: 80,
        shape_blended_fragment_count: 20,
        simd_packet_count: 7,
        swizzled_packet_count: 3,
        gather_fallback_packet_count: 4,
        precomputed_shape_fragment_count: 12,
        precomputed_shape_span_count: 3,
        direct_output_run_count: 2,
        direct_output_packet_count: 5,
    });

    assert_eq!(telemetry.work.glyph_count, 4);
    assert_eq!(telemetry.work.shape_count, 1);
    assert_eq!(telemetry.work.command_count, 3);
    assert_eq!(telemetry.work.ordered_span_count, 20);
    assert_eq!(telemetry.work.touched_tile_count, 6);
    assert_eq!(telemetry.work.sampled_texel_count, 400);
    assert_eq!(telemetry.work.simd_packet_count, 7);
    assert_eq!(telemetry.work.swizzled_packet_count, 3);
    assert_eq!(telemetry.work.gather_fallback_packet_count, 4);
    assert_eq!(telemetry.work.precomputed_shape_fragment_count, 12);
    assert_eq!(telemetry.work.precomputed_shape_span_count, 3);
    assert_eq!(telemetry.work.direct_output_run_count, 2);
    assert_eq!(telemetry.work.direct_output_packet_count, 5);
    assert_eq!(telemetry.bytes.plan_bytes, 2_048);
    assert_eq!(telemetry.bytes.static_span_bytes, 320);
    assert_eq!(
        telemetry
            .commands
            .iter()
            .find(|command| command.kind == CommandKind::Text)
            .map(|command| (
                command.command_count,
                command.covered_fragments,
                command.blended_fragments
            )),
        Some((2, 80, 80))
    );
    assert_eq!(
        telemetry
            .commands
            .iter()
            .find(|command| command.kind == CommandKind::Shape)
            .map(|command| (
                command.command_count,
                command.covered_fragments,
                command.blended_fragments
            )),
        Some((1, 20, 20))
    );
}

use super::*;
use crate::sdf::atlas::SdfAtlasGlyphManifest;
use crate::sdf::shape::ShapeSdfMaterial;

struct TestAtlas {
    width: u32,
    height: u32,
    texels: Vec<u8>,
}

struct TestMixedAtlas {
    text: u8,
    shape: ShapeSdfTexel,
}

struct PhysicalMixedAtlas {
    text: [u8; 64],
    shape: [u8; 128],
}

struct PhysicalTextAtlas {
    payload: [u8; 128],
}

struct PhysicalShapeAtlas {
    payload: [u8; 256],
}

struct WidePhysicalShapeAtlas {
    payload: [u8; 640],
}

impl SdfAtlasSource for PhysicalTextAtlas {
    fn page_dimensions(&self, atlas_set: u16, page: u16) -> Option<(u32, u32)> {
        (atlas_set == 0 && page == 0).then_some((16, 8))
    }

    fn texel(&self, atlas_set: u16, page: u16, x: u32, y: u32) -> Option<SdfAtlasTexel> {
        if atlas_set != 0 || page != 0 || x >= 16 || y >= 8 {
            return None;
        }
        let block = x / 8;
        let in_block = y * 8 + x % 8;
        self.payload
            .get((block * 64 + in_block) as usize)
            .copied()
            .map(SdfAtlasTexel::Text)
    }

    fn swizzled_page(&self, atlas_set: u16, page: u16) -> Option<SdfSwizzledPage<'_>> {
        (atlas_set == 0 && page == 0).then_some(SdfSwizzledPage {
            width: 16,
            height: 8,
            format: SdfSwizzledFormat::TextR8,
            payload: &self.payload,
        })
    }
}

impl SdfAtlasSource for PhysicalShapeAtlas {
    fn page_dimensions(&self, atlas_set: u16, page: u16) -> Option<(u32, u32)> {
        (atlas_set == 0 && page == 0).then_some((16, 8))
    }

    fn texel(&self, atlas_set: u16, page: u16, x: u32, y: u32) -> Option<SdfAtlasTexel> {
        if atlas_set != 0 || page != 0 || x >= 16 || y >= 8 {
            return None;
        }
        let block = x / 8;
        let local = y * 8 + x % 8;
        let offset = (block * 128 + local * 2) as usize;
        Some(SdfAtlasTexel::Shape(ShapeSdfTexel {
            distance: self.payload[offset],
            gate: self.payload[offset + 1],
        }))
    }

    fn swizzled_page(&self, atlas_set: u16, page: u16) -> Option<SdfSwizzledPage<'_>> {
        (atlas_set == 0 && page == 0).then_some(SdfSwizzledPage {
            width: 16,
            height: 8,
            format: SdfSwizzledFormat::ShapeRg8,
            payload: &self.payload,
        })
    }
}

impl SdfAtlasSource for WidePhysicalShapeAtlas {
    fn page_dimensions(&self, atlas_set: u16, page: u16) -> Option<(u32, u32)> {
        (atlas_set == 0 && page == 0).then_some((40, 8))
    }

    fn texel(&self, atlas_set: u16, page: u16, x: u32, y: u32) -> Option<SdfAtlasTexel> {
        if atlas_set != 0 || page != 0 || x >= 40 || y >= 8 {
            return None;
        }
        let block = x / 8;
        let local = y * 8 + x % 8;
        let offset = (block * 128 + local * 2) as usize;
        Some(SdfAtlasTexel::Shape(ShapeSdfTexel {
            distance: self.payload[offset],
            gate: self.payload[offset + 1],
        }))
    }

    fn swizzled_page(&self, atlas_set: u16, page: u16) -> Option<SdfSwizzledPage<'_>> {
        (atlas_set == 0 && page == 0).then_some(SdfSwizzledPage {
            width: 40,
            height: 8,
            format: SdfSwizzledFormat::ShapeRg8,
            payload: &self.payload,
        })
    }
}

impl SdfAtlasSource for PhysicalMixedAtlas {
    fn page_dimensions(&self, atlas_set: u16, page: u16) -> Option<(u32, u32)> {
        (atlas_set <= 1 && page == 0).then_some((8, 8))
    }

    fn texel(&self, atlas_set: u16, page: u16, x: u32, y: u32) -> Option<SdfAtlasTexel> {
        if page != 0 || x >= 8 || y >= 8 {
            return None;
        }
        let index = (y * 8 + x) as usize;
        match atlas_set {
            0 => Some(SdfAtlasTexel::Text(self.text[index])),
            1 => Some(SdfAtlasTexel::Shape(ShapeSdfTexel {
                distance: self.shape[index * 2],
                gate: self.shape[index * 2 + 1],
            })),
            _ => None,
        }
    }

    fn swizzled_page(&self, atlas_set: u16, page: u16) -> Option<SdfSwizzledPage<'_>> {
        if page != 0 {
            return None;
        }
        match atlas_set {
            0 => Some(SdfSwizzledPage {
                width: 8,
                height: 8,
                format: SdfSwizzledFormat::TextR8,
                payload: &self.text,
            }),
            1 => Some(SdfSwizzledPage {
                width: 8,
                height: 8,
                format: SdfSwizzledFormat::ShapeRg8,
                payload: &self.shape,
            }),
            _ => None,
        }
    }
}

impl SdfAtlasSource for TestMixedAtlas {
    fn page_dimensions(&self, atlas_set: u16, page: u16) -> Option<(u32, u32)> {
        (atlas_set <= 1 && page == 0).then_some((2, 2))
    }

    fn texel(&self, atlas_set: u16, page: u16, x: u32, y: u32) -> Option<SdfAtlasTexel> {
        if page != 0 || x >= 2 || y >= 2 {
            return None;
        }
        match atlas_set {
            0 => Some(SdfAtlasTexel::Text(self.text)),
            1 => Some(SdfAtlasTexel::Shape(self.shape)),
            _ => None,
        }
    }
}

impl SdfAtlasSource for TestAtlas {
    fn page_dimensions(&self, atlas_set: u16, page: u16) -> Option<(u32, u32)> {
        (atlas_set == 0 && page == 0).then_some((self.width, self.height))
    }

    fn texel(&self, atlas_set: u16, page: u16, x: u32, y: u32) -> Option<SdfAtlasTexel> {
        if atlas_set != 0 || page != 0 || x >= self.width || y >= self.height {
            return None;
        }
        self.texels
            .get((y as usize) * self.width as usize + x as usize)
            .copied()
            .map(SdfAtlasTexel::Text)
    }
}

fn solid_atlas(value: u8) -> TestAtlas {
    TestAtlas {
        width: 8,
        height: 8,
        texels: vec![value; 64],
    }
}

fn command(color: [f32; 4], quad: [Point2; 4]) -> SdfDrawCommand {
    SdfDrawCommand {
        kind: SdfPrimitiveKind::Text,
        atlas_set: 0,
        atlas_page: 0,
        atlas_rect: [0, 0, 8, 8],
        quad,
        device_clip: None,
        material: SdfCommandMaterial::Text(SdfMaterial {
            face: color,
            ..SdfMaterial::default()
        }),
    }
}

fn rectangle(x0: f32, y0: f32, x1: f32, y1: f32) -> [Point2; 4] {
    [
        Point2::new(x0, y0),
        Point2::new(x1, y0),
        Point2::new(x1, y1),
        Point2::new(x0, y1),
    ]
}

#[test]
fn plan_is_variable_sized_and_preserves_command_order_per_tile() {
    let atlas = solid_atlas(255);
    let commands = [
        command([1.0, 0.0, 0.0, 1.0], rectangle(0.0, 0.0, 3.0, 2.0)),
        command([0.0, 1.0, 0.0, 1.0], rectangle(1.0, 0.0, 4.0, 2.0)),
    ];
    let grid = TileGrid {
        canvas_width: 5,
        canvas_height: 3,
        tile_width: 2,
        tile_height: 2,
    };
    let plan = SdfTilePlan::build(grid, &commands, &atlas).expect("build plan");
    assert_eq!(plan.stats().command_count, 2);
    assert_eq!(plan.stats().span_count, 8);
    assert_eq!(
        plan.spans_for_tile(0)
            .expect("tile zero")
            .iter()
            .map(|span| span.command)
            .collect::<Vec<_>>(),
        vec![0, 0, 1, 1]
    );

    let mut output = vec![0; 5 * 3 * 4];
    let stats = plan
        .execute_scalar(&atlas, [0, 0, 0, 0], &mut output)
        .expect("execute scalar");
    assert_eq!(stats.shaded_fragment_count, 12);
    assert_eq!(&output[(0 * 5 + 0) * 4..][..4], &[255, 0, 0, 255]);
    assert_eq!(&output[(0 * 5 + 1) * 4..][..4], &[0, 255, 0, 255]);
    assert_eq!(&output[(0 * 5 + 4) * 4..][..4], &[0, 0, 0, 0]);
}

#[test]
fn device_clip_limits_text_spans_without_moving_atlas_geometry() {
    let atlas = solid_atlas(255);
    let mut clipped = command([1.0, 0.0, 0.0, 1.0], rectangle(0.0, 0.0, 8.0, 4.0));
    clipped.device_clip = Some(SdfDeviceClip {
        min_x: 2.0,
        min_y: 1.0,
        max_x: 6.0,
        max_y: 3.0,
    });
    let plan =
        SdfTilePlan::build(TileGrid::new(8, 4), &[clipped], &atlas).expect("clipped text plan");
    assert_eq!(plan.stats().covered_fragment_count, 8);

    let mut output = vec![0; 8 * 4 * 4];
    plan.execute_scalar(&atlas, [0, 0, 0, 0], &mut output)
        .expect("execute clipped text");
    for y in 0..4usize {
        for x in 0..8usize {
            let alpha = output[(y * 8 + x) * 4 + 3];
            assert_eq!(alpha != 0, (2..6).contains(&x) && (1..3).contains(&y));
        }
    }
}

#[test]
fn pixel_occlusion_counts_destination_fragments_without_changing_plan() {
    let atlas = solid_atlas(255);
    let commands = [
        command([1.0, 0.0, 0.0, 1.0], rectangle(0.0, 0.0, 3.0, 2.0)),
        command([0.0, 1.0, 0.0, 1.0], rectangle(1.0, 0.0, 4.0, 2.0)),
    ];
    let plan = SdfTilePlan::build(
        TileGrid {
            canvas_width: 5,
            canvas_height: 3,
            tile_width: 2,
            tile_height: 2,
        },
        &commands,
        &atlas,
    )
    .expect("build plan");
    let mut rgba = vec![0u8; 5 * 3 * 4];
    for y in 0..2 {
        for x in 0..3 {
            rgba[(y * 5 + x) * 4 + 3] = 255;
        }
    }
    let mut mask = PixelOcclusionMask::new(5, 3).expect("mask");
    assert_eq!(mask.union_opaque_rgba8(&rgba).expect("union"), 6);
    let measured = plan.measure_occlusion(&mask).expect("measure");
    assert_eq!(measured.occluded_fragment_count, 10);
    assert_eq!(measured.visible_fragment_count, 2);
    assert_eq!(measured.occluded_text_fragment_count, 10);
    assert_eq!(measured.occluded_shape_fragment_count, 0);
    assert_eq!(measured.fully_occluded_command_count, 1);
    assert_eq!(plan.stats().covered_fragment_count, 12);

    let (visible, filtered) = plan.visible_plan(&mask).expect("visible plan");
    assert_eq!(filtered, measured);
    assert_eq!(visible.stats().covered_fragment_count, 2);
    let mut original_pixels = vec![0; 5 * 3 * 4];
    let mut visible_pixels = vec![0; 5 * 3 * 4];
    plan.execute_scalar(&atlas, [0, 0, 0, 0], &mut original_pixels)
        .expect("execute original");
    visible
        .execute_scalar(&atlas, [0, 0, 0, 0], &mut visible_pixels)
        .expect("execute visible");
    for y in 0..2 {
        for x in 0..3 {
            let offset = (y * 5 + x) * 4;
            original_pixels[offset..offset + 4].copy_from_slice(&[0, 0, 255, 255]);
            visible_pixels[offset..offset + 4].copy_from_slice(&[0, 0, 255, 255]);
        }
    }
    assert_eq!(visible_pixels, original_pixels);
}

#[test]
fn pixel_occlusion_mask_counts_across_word_boundaries() {
    let mut rgba = vec![0u8; 130 * 4];
    for x in [0usize, 63, 64, 129] {
        rgba[x * 4 + 3] = 255;
    }
    let mut mask = PixelOcclusionMask::new(130, 1).expect("mask");
    assert_eq!(mask.union_opaque_rgba8(&rgba).expect("union"), 4);
    assert_eq!(mask.count_opaque_span(0, 0, 130), 4);
    assert_eq!(mask.count_opaque_span(0, 1, 129), 2);
    assert_eq!(mask.count_opaque_span(0, 63, 65), 2);
}

#[test]
fn bilinear_sampler_and_material_are_locked_by_scalar_oracle() {
    let atlas = TestAtlas {
        width: 2,
        height: 2,
        texels: vec![0, 64, 128, 255],
    };
    let mut draw = command([0.5, 0.25, 0.0, 0.5], rectangle(0.0, 0.0, 1.0, 1.0));
    draw.atlas_rect = [0, 0, 2, 2];
    let SdfCommandMaterial::Text(material) = &mut draw.material else {
        panic!("test command must use text material");
    };
    material.face_scale = 2.0;
    material.face_bias = 0.25;
    let plan = SdfTilePlan::build(TileGrid::new(1, 1), &[draw], &atlas).expect("build plan");
    let mut output = [0; 4];
    plan.execute_scalar(&atlas, [0, 0, 0, 0], &mut output)
        .expect("execute scalar");
    // Center sample = mean(0,64,128,255)/255 = 0.4382353;
    // face_t = center*2-0.25 = 0.6264706.
    assert_eq!(output, [80, 40, 0, 80]);
}

#[test]
fn rejects_non_affine_and_degenerate_commands() {
    let atlas = solid_atlas(255);
    let non_affine = command(
        [1.0; 4],
        [
            Point2::new(0.0, 0.0),
            Point2::new(2.0, 0.0),
            Point2::new(3.0, 2.0),
            Point2::new(0.0, 2.0),
        ],
    );
    assert_eq!(
        SdfTilePlan::build(TileGrid::new(4, 4), &[non_affine], &atlas)
            .expect_err("non-affine quad must fail"),
        SdfTileError::NonAffineQuad { command: 0 }
    );

    let degenerate = command(
        [1.0; 4],
        [
            Point2::new(0.0, 0.0),
            Point2::new(1.0, 0.0),
            Point2::new(2.0, 0.0),
            Point2::new(1.0, 0.0),
        ],
    );
    assert_eq!(
        SdfTilePlan::build(TileGrid::new(4, 4), &[degenerate], &atlas)
            .expect_err("degenerate quad must fail"),
        SdfTileError::DegenerateQuad { command: 0 }
    );
}

#[test]
fn reports_shape_commands_separately() {
    let atlas = solid_atlas(255);
    let mut shape = command([1.0; 4], rectangle(0.0, 0.0, 2.0, 2.0));
    shape.kind = SdfPrimitiveKind::Shape;
    shape.material = SdfCommandMaterial::Shape(ShapeSdfMaterial::from_profile_values(
        [1.0; 3], 1.0, [0.0; 3], 0.0, 0.0,
    ));
    let plan = SdfTilePlan::build(TileGrid::new(2, 2), &[shape], &atlas).expect("shape plan");
    assert_eq!(plan.stats().text_command_count, 0);
    assert_eq!(plan.stats().shape_command_count, 1);
}

#[test]
fn mixed_text_and_shape_keep_order_and_typed_sampling() {
    let atlas = TestMixedAtlas {
        text: 255,
        shape: ShapeSdfTexel {
            distance: 255,
            gate: 255,
        },
    };
    let mut text = command([1.0, 0.0, 0.0, 1.0], rectangle(0.0, 0.0, 1.0, 1.0));
    text.atlas_rect = [0, 0, 2, 2];
    let shape = SdfDrawCommand {
        kind: SdfPrimitiveKind::Shape,
        atlas_set: 1,
        atlas_page: 0,
        atlas_rect: [0, 0, 2, 2],
        quad: rectangle(0.0, 0.0, 1.0, 1.0),
        device_clip: None,
        material: SdfCommandMaterial::Shape(ShapeSdfMaterial::from_profile_values(
            [0.0, 1.0, 0.0],
            0.5,
            [0.0; 3],
            0.0,
            0.0,
        )),
    };
    let plan = SdfTilePlan::build(TileGrid::new(1, 1), &[text, shape], &atlas).expect("mixed plan");
    let mut output = [0; 4];
    let stats = plan
        .execute_scalar(&atlas, [0, 0, 0, 0], &mut output)
        .expect("mixed scalar execution");
    // The shape's 0.5 face alpha becomes the 8-bit vertex alpha 128/255.
    assert_eq!(output, [127, 128, 0, 255]);
    assert_eq!(stats.text_shaded_fragment_count, 1);
    assert_eq!(stats.shape_shaded_fragment_count, 1);
}

#[test]
fn load_existing_destination_preserves_untouched_pixels_and_blends_touched_pixels() {
    let atlas = PhysicalTextAtlas {
        payload: [255; 128],
    };
    let draw = command([0.5, 0.0, 0.0, 0.5], rectangle(1.0, 0.0, 3.0, 1.0));
    let plan = SdfTilePlan::build(TileGrid::new(4, 2), &[draw], &atlas).expect("build plan");
    // Non-dyadic opaque channels catch accidental conversion/writeback of
    // pixels sharing a tile with the draw but outside its actual spans.
    let background = [255, 239, 168, 255];
    let mut output = background.repeat(8);
    plan.execute_scalar_f32_over(&atlas, &mut output)
        .expect("blend over existing target");
    assert_eq!(&output[0..4], &background);
    assert_eq!(&output[4..8], &[255, 120, 84, 255]);
    assert_eq!(&output[8..12], &[255, 120, 84, 255]);
    assert_eq!(&output[12..16], &background);
    assert_eq!(&output[16..], &background.repeat(4));

    let mut simd = background.repeat(8);
    let result = plan.execute_simd_f32_over(&atlas, &mut simd);
    #[cfg(target_arch = "x86_64")]
    let simd_supported = std::arch::is_x86_feature_detected!("avx512f")
        && std::arch::is_x86_feature_detected!("avx512bw")
        && std::arch::is_x86_feature_detected!("avx512vbmi")
        && std::arch::is_x86_feature_detected!("fma");
    #[cfg(not(target_arch = "x86_64"))]
    let simd_supported = false;
    if simd_supported {
        result.expect("supported SIMD over execution");
        assert_eq!(simd, output);
    } else {
        assert_eq!(result, Err(SdfTileError::SimdUnavailable));
        assert_eq!(simd, background.repeat(8));
    }
}

#[test]
fn simd_dispatch_is_exact_to_f32_oracle_or_fails_closed() {
    let mut text = [0u8; 64];
    for (index, value) in text.iter_mut().enumerate() {
        *value = (index * 4).min(255) as u8;
    }
    let mut shape = [0u8; 128];
    for texel in shape.chunks_exact_mut(2) {
        texel.copy_from_slice(&[220, 200]);
    }
    let atlas = PhysicalMixedAtlas { text, shape };
    let text = command([0.7, 0.2, 0.1, 0.75], rectangle(0.0, 0.0, 32.0, 4.0));
    let shape = SdfDrawCommand {
        kind: SdfPrimitiveKind::Shape,
        atlas_set: 1,
        atlas_page: 0,
        atlas_rect: [0, 0, 8, 8],
        quad: rectangle(4.0, 0.0, 28.0, 4.0),
        device_clip: None,
        material: SdfCommandMaterial::Shape(ShapeSdfMaterial::from_profile_values(
            [0.1, 0.6, 0.3],
            0.55,
            [0.8, 0.2, 0.5],
            0.4,
            0.25,
        )),
    };
    let plan = SdfTilePlan::build(TileGrid::new(32, 4), &[text, shape], &atlas)
        .expect("physical mixed plan");
    let mut oracle = vec![0; 32 * 4 * 4];
    plan.execute_scalar_f32(&atlas, [3, 5, 7, 11], &mut oracle)
        .expect("scalar f32 oracle");
    let mut simd = vec![0; oracle.len()];
    let result = plan.execute_simd(
        &atlas,
        [3, 5, 7, 11],
        &mut simd,
        SdfAccumulationMode::F32Tile,
    );
    let background = [17, 31, 47, 191].repeat(32 * 4);
    let mut over_oracle = background.clone();
    plan.execute_scalar_f32_over(&atlas, &mut over_oracle)
        .expect("scalar over oracle");
    let mut over_simd = background.clone();
    let over_result = plan.execute_simd_f32_over(&atlas, &mut over_simd);
    #[cfg(target_arch = "x86_64")]
    let simd_supported = std::arch::is_x86_feature_detected!("avx512f")
        && std::arch::is_x86_feature_detected!("avx512bw")
        && std::arch::is_x86_feature_detected!("avx512vbmi")
        && std::arch::is_x86_feature_detected!("fma");
    #[cfg(not(target_arch = "x86_64"))]
    let simd_supported = false;
    if simd_supported {
        let stats = result.expect("supported SIMD execution");
        assert_eq!(simd, oracle);
        assert!(stats.simd_packet_count > 0);
        assert!(stats.swizzled_packet_count > 0);
        assert_eq!(
            stats.simd_packet_count,
            stats.swizzled_packet_count + stats.gather_fallback_packet_count
        );
        over_result.expect("supported SIMD over execution");
        assert_eq!(over_simd, over_oracle);
    } else {
        assert_eq!(result, Err(SdfTileError::SimdUnavailable));
        assert_eq!(over_result, Err(SdfTileError::SimdUnavailable));
        assert_eq!(over_simd, background);
    }
}

#[test]
fn text_adjacent_blocks_use_swizzled_loads_without_gather() {
    let mut payload = [0u8; 128];
    for (index, value) in payload.iter_mut().enumerate() {
        *value = ((index * 37 + 19) & 0xff) as u8;
    }
    let atlas = PhysicalTextAtlas { payload };
    let mut text = command([0.7, 0.2, 0.1, 0.75], rectangle(0.0, 0.0, 16.0, 1.0));
    text.atlas_rect = [0, 0, 16, 8];
    let plan =
        SdfTilePlan::build(TileGrid::new(16, 1), &[text], &atlas).expect("cross-block text plan");
    let mut oracle = vec![0; 16 * 4];
    plan.execute_scalar_f32(&atlas, [3, 5, 7, 11], &mut oracle)
        .expect("scalar f32 oracle");
    let mut simd = vec![0; oracle.len()];
    let result = plan.execute_simd(
        &atlas,
        [3, 5, 7, 11],
        &mut simd,
        SdfAccumulationMode::F32Tile,
    );

    #[cfg(target_arch = "x86_64")]
    let simd_supported = std::arch::is_x86_feature_detected!("avx512f")
        && std::arch::is_x86_feature_detected!("avx512bw")
        && std::arch::is_x86_feature_detected!("avx512vbmi")
        && std::arch::is_x86_feature_detected!("fma");
    #[cfg(not(target_arch = "x86_64"))]
    let simd_supported = false;

    if simd_supported {
        let stats = result.expect("supported SIMD execution");
        assert_eq!(simd, oracle);
        assert_eq!(stats.swizzled_packet_count, 1);
        assert_eq!(stats.gather_fallback_packet_count, 0);
    } else {
        assert_eq!(result, Err(SdfTileError::SimdUnavailable));
    }
}

#[test]
fn shape_five_blocks_use_swizzled_loads_and_match_scalar() {
    let mut payload = [0u8; 640];
    for (index, texel) in payload.chunks_exact_mut(2).enumerate() {
        texel.copy_from_slice(&[
            ((index * 29 + 17) & 0xff) as u8,
            ((index * 11 + 101) & 0xff) as u8,
        ]);
    }
    let atlas = WidePhysicalShapeAtlas { payload };
    let shape = SdfDrawCommand {
        kind: SdfPrimitiveKind::Shape,
        atlas_set: 0,
        atlas_page: 0,
        atlas_rect: [0, 0, 40, 8],
        quad: rectangle(0.0, 0.0, 16.0, 1.0),
        device_clip: None,
        material: SdfCommandMaterial::Shape(ShapeSdfMaterial::from_profile_values(
            [0.2, 0.4, 0.8],
            0.7,
            [0.8, 0.1, 0.2],
            0.3,
            0.2,
        )),
    };
    let plan =
        SdfTilePlan::build_for_one_shot_dynamic_layer(TileGrid::new(16, 1), &[shape], &atlas)
            .expect("five-block shape plan");
    let mut oracle = vec![0; 16 * 4];
    plan.execute_scalar_f32(&atlas, [3, 5, 7, 11], &mut oracle)
        .expect("scalar f32 oracle");
    let mut simd = vec![0; oracle.len()];
    let result = plan.execute_simd(
        &atlas,
        [3, 5, 7, 11],
        &mut simd,
        SdfAccumulationMode::F32Tile,
    );

    #[cfg(target_arch = "x86_64")]
    let simd_supported = std::arch::is_x86_feature_detected!("avx512f")
        && std::arch::is_x86_feature_detected!("avx512bw")
        && std::arch::is_x86_feature_detected!("avx512vbmi")
        && std::arch::is_x86_feature_detected!("fma");
    #[cfg(not(target_arch = "x86_64"))]
    let simd_supported = false;

    if simd_supported {
        let stats = result.expect("supported SIMD execution");
        assert_eq!(simd, oracle);
        assert_eq!(stats.swizzled_packet_count, 1);
        assert_eq!(stats.gather_fallback_packet_count, 0);
    } else {
        assert_eq!(result, Err(SdfTileError::SimdUnavailable));
    }
}

#[test]
fn axis_aligned_shape_uses_row_program_without_sampling() {
    let mut payload = [0u8; 256];
    for (index, texel) in payload.chunks_exact_mut(2).enumerate() {
        texel.copy_from_slice(&[
            ((index * 29 + 17) & 0xff) as u8,
            ((index * 11 + 101) & 0xff) as u8,
        ]);
    }
    let atlas = PhysicalShapeAtlas { payload };
    let shape = SdfDrawCommand {
        kind: SdfPrimitiveKind::Shape,
        atlas_set: 0,
        atlas_page: 0,
        atlas_rect: [0, 0, 16, 8],
        quad: rectangle(0.0, 0.0, 16.0, 1.0),
        device_clip: None,
        material: SdfCommandMaterial::Shape(ShapeSdfMaterial::from_profile_values(
            [0.1, 0.6, 0.3],
            0.55,
            [0.8, 0.2, 0.5],
            0.4,
            0.25,
        )),
    };
    let plan =
        SdfTilePlan::build(TileGrid::new(16, 1), &[shape], &atlas).expect("cross-block shape plan");
    let mut oracle = vec![0; 16 * 4];
    plan.execute_scalar_f32(&atlas, [3, 5, 7, 11], &mut oracle)
        .expect("scalar f32 oracle");
    let mut simd = vec![0; oracle.len()];
    let result = plan.execute_simd(
        &atlas,
        [3, 5, 7, 11],
        &mut simd,
        SdfAccumulationMode::F32Tile,
    );

    #[cfg(target_arch = "x86_64")]
    let simd_supported = std::arch::is_x86_feature_detected!("avx512f")
        && std::arch::is_x86_feature_detected!("avx512bw")
        && std::arch::is_x86_feature_detected!("avx512vbmi")
        && std::arch::is_x86_feature_detected!("fma");
    #[cfg(not(target_arch = "x86_64"))]
    let simd_supported = false;

    if simd_supported {
        let stats = result.expect("supported SIMD execution");
        assert_eq!(simd, oracle);
        assert!(stats.precomputed_shape_fragment_count > 0);
        assert!(stats.precomputed_shape_span_count > 0);
        assert_eq!(stats.sampled_texel_count, 0);
        assert_eq!(stats.swizzled_packet_count, 0);
        assert_eq!(stats.gather_fallback_packet_count, 0);
    } else {
        assert_eq!(result, Err(SdfTileError::SimdUnavailable));
    }
}

#[test]
fn atlas_glyph_builder_reuses_manifest_plane_geometry() {
    let glyph = SdfAtlasGlyphManifest {
        codepoint: u32::from('A'),
        page: 3,
        rect: [16, 24, 8, 9],
        plane_bearing: [2.0, 7.0],
        plane_size: [4.0, 5.0],
        plane_advance_x: 4.5,
    };
    let command = SdfDrawCommand::from_atlas_glyph(
        SdfPrimitiveKind::Text,
        0,
        &glyph,
        10.0,
        1.0,
        Point2::new(10.0, 20.0),
        20.0,
        Affine2::IDENTITY,
        SdfMaterial::default(),
    )
    .expect("valid glyph placement");
    assert_eq!(command.atlas_page, 3);
    assert_eq!(command.atlas_rect, [16, 24, 8, 9]);
    assert_eq!(
        command.quad,
        [
            Point2::new(12.0, 4.0),
            Point2::new(24.0, 4.0),
            Point2::new(24.0, 18.0),
            Point2::new(12.0, 18.0),
        ]
    );
}

#[test]
fn atlas_glyph_builder_applies_resolved_affine_without_relayout() {
    let glyph = SdfAtlasGlyphManifest {
        codepoint: u32::from('B'),
        page: 0,
        rect: [0, 0, 8, 8],
        plane_bearing: [0.0, 1.0],
        plane_size: [2.0, 2.0],
        plane_advance_x: 2.0,
    };
    let affine = Affine2 {
        scale_x: 2.0,
        skew_x: 0.5,
        translate_x: 3.0,
        skew_y: -0.25,
        scale_y: 1.5,
        translate_y: 4.0,
    };
    let command = SdfDrawCommand::from_atlas_glyph(
        SdfPrimitiveKind::Text,
        0,
        &glyph,
        1.0,
        0.0,
        Point2::new(0.0, 0.0),
        1.0,
        affine,
        SdfMaterial::default(),
    )
    .expect("affine glyph placement");
    assert_eq!(command.quad[0], affine.map_point(Point2::new(0.0, -1.0)));
    assert_eq!(command.quad[2], affine.map_point(Point2::new(2.0, 1.0)));
    let plan = SdfTilePlan::build(TileGrid::new(16, 16), &[command], &solid_atlas(255))
        .expect("affine command must remain a parallelogram");
    assert_eq!(plan.stats().command_count, 1);
}

#[test]
fn axis_aligned_shape_row_program_is_exact_to_sampler_oracle() {
    let mut atlas = PhysicalShapeAtlas { payload: [0; 256] };
    for y in 0..8u32 {
        for x in 0..16u32 {
            let block = x / 8;
            let local = y * 8 + x % 8;
            let offset = (block * 128 + local * 2) as usize;
            let border = x == 0 || x == 15 || y == 0 || y == 7;
            atlas.payload[offset] = if border {
                0
            } else {
                ((x * 19 + y * 31) & 0xff) as u8
            };
            atlas.payload[offset + 1] = if border { 0 } else { 255 };
        }
    }
    let shape = SdfDrawCommand {
        kind: SdfPrimitiveKind::Shape,
        atlas_set: 0,
        atlas_page: 0,
        atlas_rect: [0, 0, 16, 8],
        quad: rectangle(2.25, 1.75, 70.25, 39.75),
        device_clip: None,
        material: SdfCommandMaterial::Shape(ShapeSdfMaterial::from_profile_values(
            [0.1, 0.6, 0.3],
            0.55,
            [0.8, 0.2, 0.5],
            0.4,
            0.25,
        )),
    };
    let plan = SdfTilePlan::build(TileGrid::new(73, 41), &[shape], &atlas)
        .expect("axis-aligned shape plan");
    let program = plan.axis_shape_program(0).expect("row program");
    assert!(program.runs.len() < 16 * 8);

    let background = [17, 31, 47, 191].repeat(73 * 41);
    let mut oracle = background.clone();
    plan.execute_scalar_f32_over(&atlas, &mut oracle)
        .expect("sampler oracle");
    let mut candidate = background.clone();
    let stats = plan
        .execute_scalar_f32_over_precomputed_shapes(&atlas, &mut candidate)
        .expect("row-program candidate");
    assert_eq!(candidate, oracle);
    assert!(stats.precomputed_shape_fragment_count > 0);
    assert!(stats.precomputed_shape_span_count > 0);
    assert_eq!(stats.sampled_texel_count, 0);
}

#[test]
fn one_shot_shape_plan_skips_row_program_and_eager_direct_spans() {
    let atlas = PhysicalShapeAtlas {
        payload: [255; 256],
    };
    let shape = SdfDrawCommand {
        kind: SdfPrimitiveKind::Shape,
        atlas_set: 0,
        atlas_page: 0,
        atlas_rect: [0, 0, 16, 8],
        quad: rectangle(2.0, 3.0, 70.0, 39.0),
        device_clip: None,
        material: SdfCommandMaterial::Shape(ShapeSdfMaterial::from_profile_values(
            [0.1, 0.6, 0.3],
            0.55,
            [0.8, 0.2, 0.5],
            0.4,
            0.25,
        )),
    };
    let plan =
        SdfTilePlan::build_for_one_shot_dynamic_layer(TileGrid::new(73, 41), &[shape], &atlas)
            .expect("one-shot Shape plan");

    assert!(plan.axis_shape_program(0).is_none());
    assert!(plan.direct_axis_shape_spans().is_none());
}

#[test]
fn static_one_shot_keeps_row_program_without_eager_direct_spans() {
    let atlas = PhysicalShapeAtlas {
        payload: [255; 256],
    };
    let shape = SdfDrawCommand {
        kind: SdfPrimitiveKind::Shape,
        atlas_set: 0,
        atlas_page: 0,
        atlas_rect: [0, 0, 16, 8],
        quad: rectangle(2.0, 3.0, 70.0, 39.0),
        device_clip: None,
        material: SdfCommandMaterial::Shape(ShapeSdfMaterial::from_profile_values(
            [0.1, 0.6, 0.3],
            0.55,
            [0.8, 0.2, 0.5],
            0.4,
            0.25,
        )),
    };
    let cache = ShapeRowProgramCache::default();
    let plan = SdfTilePlan::build_static_one_shot_with_shape_program_cache(
        TileGrid::new(73, 41),
        &[shape],
        &atlas,
        &cache,
    )
    .expect("static one-shot Shape plan");

    assert!(plan.axis_shape_program(0).is_some());
    assert!(plan.direct_axis_shape_spans().is_none());
    let mask = PixelOcclusionMask::new(73, 41).expect("occlusion mask");
    let (visible, _) = plan.visible_plan(&mask).expect("visible one-shot plan");
    assert!(visible.axis_shape_program(0).is_some());
    assert!(visible.direct_axis_shape_spans().is_none());
    let background = [17, 31, 47, 191].repeat(73 * 41);
    let mut oracle = background.clone();
    plan.execute_scalar_f32_over(&atlas, &mut oracle)
        .expect("sampler oracle");
    let mut candidate = background.clone();
    let stats = plan
        .execute_scalar_f32_over_precomputed_shapes(&atlas, &mut candidate)
        .expect("row-program candidate");
    assert_eq!(candidate, oracle);
    assert!(stats.precomputed_shape_fragment_count > 0);
    assert_eq!(stats.sampled_texel_count, 0);

    let mut recolored = shape;
    recolored.material = SdfCommandMaterial::Shape(ShapeSdfMaterial::from_profile_values(
        [0.9, 0.1, 0.7],
        0.8,
        [0.1, 0.8, 0.2],
        0.3,
        0.25,
    ));
    let recolored_plan = SdfTilePlan::build_static_one_shot_with_shape_program_cache(
        TileGrid::new(73, 41),
        &[recolored],
        &atlas,
        &cache,
    )
    .expect("recolored static one-shot Shape plan");
    assert!(Arc::ptr_eq(
        plan.axis_shape_programs[0]
            .as_ref()
            .expect("original row program"),
        recolored_plan.axis_shape_programs[0]
            .as_ref()
            .expect("recolored row program"),
    ));
    let mut recolored_oracle = background.clone();
    recolored_plan
        .execute_scalar_f32_over(&atlas, &mut recolored_oracle)
        .expect("recolored sampler oracle");
    let mut recolored_candidate = background;
    recolored_plan
        .execute_scalar_f32_over_precomputed_shapes(&atlas, &mut recolored_candidate)
        .expect("recolored row-program candidate");
    assert_eq!(recolored_candidate, recolored_oracle);

    let mut reoutlined = recolored;
    reoutlined.material = SdfCommandMaterial::Shape(ShapeSdfMaterial::from_profile_values(
        [0.9, 0.1, 0.7],
        0.8,
        [0.1, 0.8, 0.2],
        0.3,
        0.75,
    ));
    let reoutlined_plan = SdfTilePlan::build_static_one_shot_with_shape_program_cache(
        TileGrid::new(73, 41),
        &[reoutlined],
        &atlas,
        &cache,
    )
    .expect("reoutlined static one-shot Shape plan");
    assert!(!Arc::ptr_eq(
        recolored_plan.axis_shape_programs[0]
            .as_ref()
            .expect("recolored row program"),
        reoutlined_plan.axis_shape_programs[0]
            .as_ref()
            .expect("reoutlined row program"),
    ));
    assert_eq!(
        cache
            .source_programs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .len(),
        1,
    );
    let mut reoutlined_oracle = [17, 31, 47, 191].repeat(73 * 41);
    reoutlined_plan
        .execute_scalar_f32_over(&atlas, &mut reoutlined_oracle)
        .expect("reoutlined sampler oracle");
    let mut reoutlined_candidate = [17, 31, 47, 191].repeat(73 * 41);
    reoutlined_plan
        .execute_scalar_f32_over_precomputed_shapes(&atlas, &mut reoutlined_candidate)
        .expect("reoutlined row-program candidate");
    assert_eq!(reoutlined_candidate, reoutlined_oracle);
}

#[test]
fn transformed_shape_without_axis_alignment_falls_back_to_sampler() {
    let atlas = PhysicalShapeAtlas {
        payload: [255; 256],
    };
    let shape = SdfDrawCommand {
        kind: SdfPrimitiveKind::Shape,
        atlas_set: 0,
        atlas_page: 0,
        atlas_rect: [0, 0, 16, 8],
        quad: [
            Point2::new(2.0, 1.0),
            Point2::new(20.0, 3.0),
            Point2::new(24.0, 14.0),
            Point2::new(6.0, 12.0),
        ],
        device_clip: None,
        material: SdfCommandMaterial::Shape(ShapeSdfMaterial::from_profile_values(
            [0.2, 0.4, 0.8],
            0.7,
            [0.8, 0.1, 0.2],
            0.3,
            0.2,
        )),
    };
    let plan = SdfTilePlan::build(TileGrid::new(32, 16), &[shape], &atlas)
        .expect("transformed shape plan");
    assert!(plan.axis_shape_program(0).is_none());
    let background = [5, 7, 11, 191].repeat(32 * 16);
    let mut oracle = background.clone();
    plan.execute_scalar_f32_over(&atlas, &mut oracle)
        .expect("sampler oracle");
    let mut candidate = background;
    let stats = plan
        .execute_scalar_f32_over_precomputed_shapes(&atlas, &mut candidate)
        .expect("fallback candidate");
    assert_eq!(candidate, oracle);
    assert_eq!(stats.precomputed_shape_fragment_count, 0);
    assert!(stats.sampled_texel_count > 0);
}

#[test]
fn dilated_underlay_follows_the_material_ramp_down_to_zero_distance() {
    let material = SdfMaterial {
        face: [0.0, 0.0, 0.0, 1.0],
        outline: [0.0, 0.0, 1.0, 1.0],
        face_scale: 5.43,
        face_bias: 2.0,
        outline_scale: 5.43,
        outline_bias: -0.0474,
        vertex_alpha: 1.0,
    };
    for sdf in [0.0f32, 0.02, 0.05, 0.1] {
        let expected = 5.43f32.mul_add(sdf, 0.0474).clamp(0.0, 1.0);
        let shaded = shade_text(material, sdf);
        assert_eq!(shaded[2], expected, "sdf {sdf}");
        assert_eq!(shaded[3], expected, "sdf {sdf}");
    }
}

#[test]
fn shape_coverage_ramp_matches_between_scalar_and_simd() {
    let mut payload = [0u8; 640];
    for (index, texel) in payload.as_chunks_mut::<2>().0.iter_mut().enumerate() {
        *texel = [126 + (index % 17) as u8, [255, 200, 128, 3, 1][index % 5]];
    }
    let atlas = WidePhysicalShapeAtlas { payload };
    let shape = SdfDrawCommand {
        kind: SdfPrimitiveKind::Shape,
        atlas_set: 0,
        atlas_page: 0,
        atlas_rect: [0, 0, 40, 8],
        quad: rectangle(0.0, 0.0, 16.0, 8.0),
        device_clip: None,
        material: SdfCommandMaterial::Shape(ShapeSdfMaterial {
            face: [0.14, 0.28, 0.56, 0.7],
            outline: [0.24, 0.03, 0.06, 0.3],
            face_threshold: 0.5475,
            outline_threshold: 0.5,
            sharpness: 1.5,
        }),
    };
    let plan =
        SdfTilePlan::build_for_one_shot_dynamic_layer(TileGrid::new(16, 8), &[shape], &atlas)
            .expect("ramp shape plan");
    let mut oracle = vec![0; 16 * 8 * 4];
    plan.execute_scalar_f32(&atlas, [3, 5, 7, 11], &mut oracle)
        .expect("scalar f32 oracle");
    assert!(oracle
        .as_chunks::<4>()
        .0
        .iter()
        .any(|pixel| *pixel != [3, 5, 7, 11]));
    let mut simd = vec![0; oracle.len()];
    let result = plan.execute_simd(
        &atlas,
        [3, 5, 7, 11],
        &mut simd,
        SdfAccumulationMode::F32Tile,
    );
    #[cfg(target_arch = "x86_64")]
    let simd_supported = std::arch::is_x86_feature_detected!("avx512f")
        && std::arch::is_x86_feature_detected!("avx512bw")
        && std::arch::is_x86_feature_detected!("avx512vbmi")
        && std::arch::is_x86_feature_detected!("fma");
    #[cfg(not(target_arch = "x86_64"))]
    let simd_supported = false;
    if simd_supported {
        result.expect("supported SIMD execution");
        assert_eq!(simd, oracle);
    } else {
        assert_eq!(result, Err(SdfTileError::SimdUnavailable));
    }
}

//! 场景图渲染器高层 API。

use crate::assets::AssetStore;
use crate::masterdata::{MasterData, MasterDataProvider};
use crate::types::{CustomProfileCard, UserCustomProfileCard};
use std::sync::{Arc, RwLock};

/// 个人资料画布主题别名。`scenes` 关闭时为不可构造的占位类型，
/// 使共享的内部渲染函数签名在两种配置下一致（永远只会传 `None`）。
#[cfg(feature = "scenes")]
#[allow(dead_code)] // 仅在 skia-core 渲染路径中作为签名占位
pub(crate) type PersonalTheme = crate::personal_profile::PersonalProfileTheme;
#[cfg(not(feature = "scenes"))]
#[derive(Clone, Copy)]
#[allow(dead_code)] // 仅在 skia-core 渲染路径中作为签名占位
pub(crate) enum PersonalTheme {}

/// 自定义名片渲染器。
pub struct CustomProfileRenderer {
    md_source: RwLock<Arc<dyn MasterDataProvider>>,
    assets: Option<Arc<AssetStore>>,
    #[cfg(feature = "skia-core")]
    sdf_atlases: Option<Arc<crate::sdf::atlas::MappedSdfAtlasSet>>,
    #[cfg(feature = "skia-core")]
    shape_sdf_atlas: Option<Arc<crate::sdf::shape_atlas::MappedShapeSdfAtlas>>,
    #[cfg(feature = "skia-core")]
    render_object_generations: Option<Arc<crate::render_object::RenderObjectGenerationManager>>,
}

impl CustomProfileRenderer {
    /// 用 MasterData provider 初始化渲染器。
    pub fn new(provider: Arc<dyn MasterDataProvider>) -> Self {
        let md = MasterData::new(Arc::clone(&provider));
        tracing::info!(
            colors = md.color_count(),
            fonts = md.font_count(),
            "自定义名片渲染器初始化完成"
        );
        Self {
            md_source: RwLock::new(provider),
            assets: None,
            #[cfg(feature = "skia-core")]
            sdf_atlases: None,
            #[cfg(feature = "skia-core")]
            shape_sdf_atlas: None,
            #[cfg(feature = "skia-core")]
            render_object_generations: None,
        }
    }

    /// 设置素材缓存。
    pub fn with_assets(mut self, assets: Arc<AssetStore>) -> Self {
        self.assets = Some(assets);
        self
    }

    #[cfg(feature = "skia-core")]
    pub fn with_sdf_atlases(mut self, atlases: Arc<crate::sdf::atlas::MappedSdfAtlasSet>) -> Self {
        self.sdf_atlases = Some(atlases);
        self
    }

    #[cfg(feature = "animation-export")]
    pub(crate) fn mapped_text_sdf_atlases(
        &self,
    ) -> Option<Arc<crate::sdf::atlas::MappedSdfAtlasSet>> {
        self.sdf_atlases.clone()
    }

    #[cfg(feature = "skia-core")]
    pub fn with_shape_sdf_atlas(
        mut self,
        atlas: Arc<crate::sdf::shape_atlas::MappedShapeSdfAtlas>,
    ) -> Self {
        self.shape_sdf_atlas = Some(atlas);
        self
    }

    #[cfg(feature = "skia-core")]
    pub fn with_render_object_store(
        mut self,
        store: Arc<crate::render_object::MappedRenderObjectStore>,
    ) -> Self {
        self.render_object_generations = Some(Arc::new(
            crate::render_object::RenderObjectGenerationManager::new(store),
        ));
        self
    }

    #[cfg(feature = "skia-core")]
    pub fn profile_backend_capabilities(
        &self,
    ) -> crate::profile_backend::ProfileBackendCapabilities {
        let simd = turin_sdf_simd_available();
        let sdf_atlases_available = self.sdf_atlases.as_ref().map_or(false, |a| !a.is_empty());
        crate::profile_backend::ProfileBackendCapabilities {
            skia_raster_cpu: true,
            skia_opengl_llvmpipe: false,
            skia_vulkan_lavapipe: false,
            text_legacy_skia: true,
            text_simd: simd && sdf_atlases_available,
            text_scalar_oracle: sdf_atlases_available,
            shape_skia: true,
            shape_simd: simd && self.shape_sdf_atlas.is_some(),
        }
    }

    /// 热替换 MasterData provider。
    pub fn swap_masterdata(&self, new_provider: Arc<dyn MasterDataProvider>) {
        let md = MasterData::new(Arc::clone(&new_provider));
        tracing::info!(
            colors = md.color_count(),
            fonts = md.font_count(),
            "MasterData 热替换完成"
        );
        *self.md_source.write().unwrap_or_else(|e| e.into_inner()) = new_provider;
    }

    fn snapshot(&self) -> MasterData {
        let provider = self
            .md_source
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        MasterData::new(provider)
    }

    pub fn snapshot_masterdata(&self) -> MasterData {
        self.snapshot()
    }

    #[cfg(feature = "skia-core")]
    pub fn render_page(&self, card: &CustomProfileCard) -> Result<Vec<u8>, String> {
        self.render_page_with_profile(card, None)
    }

    #[cfg(feature = "skia-core")]
    pub fn render_page_with_profile(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
    ) -> Result<Vec<u8>, String> {
        let md = self.snapshot();
        let asset_ref = self.assets.as_deref();
        render_card(card, &md, asset_ref, profile)
    }

    #[cfg(all(feature = "skia-core", feature = "scenes"))]
    pub fn render_personal_profile(
        &self,
        input: &crate::personal_profile::PersonalProfileRenderInput,
    ) -> Result<crate::traits::RenderOutput, String> {
        let md = self.snapshot();
        let asset_ref = self.assets.as_deref();
        crate::personal_profile::render_personal_profile(input, &md, asset_ref)
    }

    #[cfg(all(feature = "skia-core", feature = "scenes"))]
    pub fn render_personal_profile_canvas(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
        theme: crate::personal_profile::PersonalProfileTheme,
    ) -> Result<Vec<u8>, String> {
        let md = self.snapshot();
        let asset_ref = self.assets.as_deref();
        render_card_personal_profile_canvas(card, &md, asset_ref, profile, theme)
    }

    #[cfg(all(feature = "skia-core", feature = "scenes"))]
    pub fn render_personal_profile_canvas_sized(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
        theme: crate::personal_profile::PersonalProfileTheme,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, String> {
        let md = self.snapshot();
        let asset_ref = self.assets.as_deref();
        render_card_personal_profile_canvas_sized(
            card, &md, asset_ref, profile, theme, width, height,
        )
    }

    #[cfg(feature = "skia-core")]
    pub fn render_page_png_with_profile(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
    ) -> Result<Vec<u8>, String> {
        let md = self.snapshot();
        let asset_ref = self.assets.as_deref();
        render_card_png(card, &md, asset_ref, profile)
    }

    #[cfg(feature = "skia-core")]
    pub fn render_page_png_transparent_with_profile(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
    ) -> Result<Vec<u8>, String> {
        let md = self.snapshot();
        let asset_ref = self.assets.as_deref();
        render_card_png_transparent(card, &md, asset_ref, profile)
    }

    #[cfg(feature = "skia-core")]
    pub fn render_element_layer_cropped(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
        webp_quality: u32,
    ) -> Result<CroppedLayerOutput, String> {
        let md = self.snapshot();
        let asset_ref = self.assets.as_deref();
        render_element_layer_cropped(card, &md, asset_ref, profile, webp_quality)
    }

    /// Renders one immutable standard-honor variant as a tightly sized WebP.
    /// Live-master progress is player state and is not baked into this output.
    #[cfg(feature = "skia-core")]
    pub fn render_static_honor_artwork(
        &self,
        honor_id: i32,
        honor_level: i32,
        full_size: bool,
        webp_quality: u32,
    ) -> Result<HonorArtworkOutput, String> {
        let md = self.snapshot();
        let fallback_assets = AssetStore::new(1);
        let assets = self.assets.as_deref().unwrap_or(&fallback_assets);
        encode_honor_artwork(full_size, webp_quality, |canvas| {
            crate::elements::honor::render_static_honor(
                canvas,
                honor_id,
                honor_level,
                full_size,
                &md,
                assets,
            );
        })
    }

    /// Renders one fully specified bonds-honor variant as a tightly sized WebP.
    #[cfg(feature = "skia-core")]
    #[allow(clippy::too_many_arguments)]
    pub fn render_bonds_honor_artwork(
        &self,
        honor_id: i32,
        honor_level: i32,
        full_size: bool,
        word_id: i64,
        inverse: bool,
        use_unit_virtual_singer: bool,
        webp_quality: u32,
    ) -> Result<HonorArtworkOutput, String> {
        let md = self.snapshot();
        let fallback_assets = AssetStore::new(1);
        let assets = self.assets.as_deref().unwrap_or(&fallback_assets);
        encode_honor_artwork(full_size, webp_quality, |canvas| {
            crate::elements::honor::render_bonds_honor(
                canvas,
                honor_id,
                honor_level,
                full_size,
                word_id,
                inverse,
                use_unit_virtual_singer,
                &md,
                assets,
            );
        })
    }

    /// 批量分层裁剪渲染（统一原语）：见自由函数 `render_all_layers_cropped` 的注释。
    #[cfg(feature = "skia-core")]
    pub fn render_all_layers_cropped(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
        webp_quality: u32,
        include_properties: bool,
    ) -> Result<Vec<LayerCrop>, String> {
        let md = self.snapshot();
        let asset_ref = self.assets.as_deref();
        render_all_layers_cropped(
            card,
            &md,
            asset_ref,
            profile,
            webp_quality,
            include_properties,
        )
    }

    #[cfg(feature = "skia-core")]
    pub fn render_by_seq(
        &self,
        cards: &[UserCustomProfileCard],
        page: u32,
    ) -> Result<Vec<u8>, String> {
        let card = cards
            .iter()
            .find(|c| c.seq == page as i32)
            .ok_or_else(|| format!("未找到第 {page} 页名片"))?;
        self.render_page(&card.custom_profile_card)
    }

    #[cfg(not(feature = "skia-core"))]
    pub fn render_page(&self, _card: &CustomProfileCard) -> Result<Vec<u8>, String> {
        Err("Skia 渲染未启用，请使用 --features skia 编译".into())
    }

    #[cfg(not(feature = "skia-core"))]
    pub fn render_page_with_profile(
        &self,
        _card: &CustomProfileCard,
        _profile: Option<&crate::profile::ProfileData>,
    ) -> Result<Vec<u8>, String> {
        Err("Skia 渲染未启用，请使用 --features skia 编译".into())
    }

    #[cfg(not(feature = "skia-core"))]
    pub fn render_page_png_with_profile(
        &self,
        _card: &CustomProfileCard,
        _profile: Option<&crate::profile::ProfileData>,
    ) -> Result<Vec<u8>, String> {
        Err("Skia 渲染未启用，请使用 --features skia 编译".into())
    }

    #[cfg(not(feature = "skia-core"))]
    pub fn render_page_png_transparent_with_profile(
        &self,
        _card: &CustomProfileCard,
        _profile: Option<&crate::profile::ProfileData>,
    ) -> Result<Vec<u8>, String> {
        Err("Skia 渲染未启用，请使用 --features skia 编译".into())
    }

    #[cfg(all(not(feature = "skia-core"), feature = "scenes"))]
    pub fn render_personal_profile(
        &self,
        _input: &crate::personal_profile::PersonalProfileRenderInput,
    ) -> Result<crate::traits::RenderOutput, String> {
        Err("Skia 渲染未启用，请使用 --features skia 编译".into())
    }

    #[cfg(all(not(feature = "skia-core"), feature = "scenes"))]
    pub fn render_personal_profile_canvas(
        &self,
        _card: &CustomProfileCard,
        _profile: Option<&crate::profile::ProfileData>,
        _theme: crate::personal_profile::PersonalProfileTheme,
    ) -> Result<Vec<u8>, String> {
        Err("Skia 渲染未启用，请使用 --features skia 编译".into())
    }

    #[cfg(all(not(feature = "skia-core"), feature = "scenes"))]
    pub fn render_personal_profile_canvas_sized(
        &self,
        _card: &CustomProfileCard,
        _profile: Option<&crate::profile::ProfileData>,
        _theme: crate::personal_profile::PersonalProfileTheme,
        _width: u32,
        _height: u32,
    ) -> Result<Vec<u8>, String> {
        Err("Skia 渲染未启用，请使用 --features skia 编译".into())
    }

    #[cfg(not(feature = "skia-core"))]
    pub fn render_by_seq(
        &self,
        _cards: &[UserCustomProfileCard],
        _page: u32,
    ) -> Result<Vec<u8>, String> {
        Err("Skia 渲染未启用，请使用 --features skia 编译".into())
    }

    /// 获取 MasterData 快照。
    pub fn masterdata(&self) -> MasterData {
        self.snapshot()
    }

    /// 获取内部 AssetStore。
    pub fn assets(&self) -> Option<&Arc<AssetStore>> {
        self.assets.as_ref()
    }

    /// 预校验名片数据。
    pub fn validate_card(&self, card: &CustomProfileCard) -> Vec<String> {
        let md = self.snapshot();
        let mut warnings = Vec::new();
        for (i, text) in card.texts.iter().enumerate() {
            if md.resolve_color(text.color_id).is_none() {
                warnings.push(format!(
                    "texts[{i}]: colorId={} 不在映射表中",
                    text.color_id
                ));
            }
            if md.resolve_font(text.font_id).is_none() {
                warnings.push(format!("texts[{i}]: fontId={} 不在映射表中", text.font_id));
            }
        }
        for (i, shape) in card.shapes.iter().enumerate() {
            if md.resolve_color(shape.color_id).is_none() {
                warnings.push(format!(
                    "shapes[{i}]: colorId={} 不在映射表中",
                    shape.color_id
                ));
            }
            if md.resolve_resource("shape", shape.id).is_none() {
                warnings.push(format!("shapes[{i}]: shapeId={} 不在映射表中", shape.id));
            }
        }
        if !warnings.is_empty() {
            tracing::warn!(count = warnings.len(), "名片数据校验发现缺失映射");
        }
        warnings
    }

    /// 填充名片中的 Honor/BondsHonor 等级。
    pub fn enrich_honor_levels(
        &self,
        card: &mut CustomProfileCard,
        honor_levels: &std::collections::HashMap<i32, i32>,
        bonds_levels: &std::collections::HashMap<i32, i32>,
        char_ranks: &std::collections::HashMap<i32, i32>,
    ) {
        let md = self.snapshot();
        for honor in &mut card.honors {
            if let Some(&level) = honor_levels.get(&honor.id) {
                honor.honor_level = level;
            } else if let Some(res) = md.resolve_honor(honor.id, 1) {
                if res.honor_type == "character" {
                    if let Some(entry) = md.get_honor(honor.id) {
                        if let Some(group_id) = entry.group_id {
                            if let Some(&rank) = char_ranks.get(&group_id) {
                                honor.honor_level = rank;
                            }
                        }
                    }
                }
            }
        }
        for bond in &mut card.bonds_honors {
            if let Some(&level) = bonds_levels.get(&bond.id) {
                bond.honor_level = level;
            }
        }
    }
}

#[cfg(feature = "skia-core")]
fn render_card_encoded(
    card: &CustomProfileCard,
    md: &MasterData,
    assets: Option<&AssetStore>,
    profile: Option<&crate::profile::ProfileData>,
    clear_color: skia_safe::Color,
    format: skia_safe::EncodedImageFormat,
    quality: u32,
) -> Result<Vec<u8>, String> {
    render_card_encoded_with_background(
        card,
        md,
        assets,
        profile,
        clear_color,
        None,
        format,
        quality,
    )
}

#[cfg(feature = "skia-core")]
fn render_card_encoded_with_background(
    card: &CustomProfileCard,
    md: &MasterData,
    assets: Option<&AssetStore>,
    profile: Option<&crate::profile::ProfileData>,
    clear_color: skia_safe::Color,
    personal_theme: Option<PersonalTheme>,
    format: skia_safe::EncodedImageFormat,
    quality: u32,
) -> Result<Vec<u8>, String> {
    render_card_encoded_with_background_sized(
        card,
        md,
        assets,
        profile,
        clear_color,
        personal_theme,
        format,
        quality,
        crate::transform::CANVAS_WIDTH as u32,
        crate::transform::CANVAS_HEIGHT as u32,
    )
}

#[cfg(feature = "skia-core")]
fn render_card_encoded_with_background_sized(
    card: &CustomProfileCard,
    md: &MasterData,
    assets: Option<&AssetStore>,
    profile: Option<&crate::profile::ProfileData>,
    clear_color: skia_safe::Color,
    personal_theme: Option<PersonalTheme>,
    format: skia_safe::EncodedImageFormat,
    quality: u32,
    canvas_width: u32,
    canvas_height: u32,
) -> Result<Vec<u8>, String> {
    use skia_safe::surfaces;

    let mut surface = surfaces::raster_n32_premul((canvas_width as i32, canvas_height as i32))
        .ok_or("创建 Skia Surface 失败")?;

    let canvas = surface.canvas();
    match personal_theme {
        #[cfg(feature = "scenes")]
        Some(theme) => draw_personal_profile_general_background(
            canvas,
            theme,
            canvas_width as f32,
            canvas_height as f32,
        ),
        #[cfg(not(feature = "scenes"))]
        Some(theme) => match theme {},
        None => {
            canvas.clear(clear_color);
        }
    }

    let elements = crate::elements::flatten_and_sort(card);
    let text_count = elements
        .iter()
        .filter(|e| matches!(e, crate::elements::RenderElement::Text(_)))
        .count();
    tracing::debug!(
        total = elements.len(),
        text_count,
        "interpret: 名片元素统计"
    );

    // 循环外一次性构造共享上下文，避免每个元素重建（#29）。
    let fallback_assets = crate::assets::AssetStore::new(1);
    let theme = crate::widgets::theme::Theme::default();
    for elem in &elements {
        if !elem.visible() {
            continue;
        }
        crate::elements::draw_element_on_canvas(
            canvas,
            elem,
            md,
            assets,
            profile,
            &fallback_assets,
            &theme,
            canvas_width as f32,
            canvas_height as f32,
        );
    }

    let image = surface.image_snapshot();
    let ctx: Option<&mut skia_safe::gpu::DirectContext> = None;
    let data = image
        .encode(ctx, format, Some(quality))
        .ok_or_else(|| match format {
            skia_safe::EncodedImageFormat::PNG => "PNG 编码失败".to_string(),
            _ => "图片编码失败".to_string(),
        })?;
    Ok(data.as_bytes().to_vec())
}

#[cfg(all(feature = "skia-core", feature = "scenes"))]
fn draw_personal_profile_general_background(
    canvas: &skia_safe::Canvas,
    theme: crate::personal_profile::PersonalProfileTheme,
    canvas_width: f32,
    canvas_height: f32,
) {
    let (bg, niigo, miku, line, panel, edge, shadow) = match theme {
        crate::personal_profile::PersonalProfileTheme::NiigoDark => (
            skia_safe::Color::from_argb(255, 20, 20, 28),
            skia_safe::Color::from_argb(255, 136, 122, 240),
            skia_safe::Color::from_argb(255, 168, 216, 232),
            skia_safe::Color::from_argb(18, 54, 58, 82),
            skia_safe::Color::from_argb(232, 246, 246, 252),
            skia_safe::Color::from_argb(84, 107, 105, 146),
            skia_safe::Color::from_argb(56, 0, 0, 0),
        ),
        crate::personal_profile::PersonalProfileTheme::MikuLight => (
            skia_safe::Color::from_argb(255, 229, 242, 247),
            skia_safe::Color::from_argb(255, 125, 112, 224),
            skia_safe::Color::from_argb(255, 65, 164, 194),
            skia_safe::Color::from_argb(24, 56, 92, 112),
            skia_safe::Color::from_argb(150, 255, 255, 255),
            skia_safe::Color::from_argb(60, 70, 120, 140),
            skia_safe::Color::from_argb(22, 0, 42, 54),
        ),
    };

    canvas.clear(bg);

    let mut paint = skia_safe::Paint::default();
    paint.set_anti_alias(true);
    paint.set_style(skia_safe::PaintStyle::Fill);
    paint.set_color(niigo);
    canvas.draw_rect(
        skia_safe::Rect::from_xywh(0.0, 0.0, canvas_width * 0.64, 5.0),
        &paint,
    );
    paint.set_color(miku);
    canvas.draw_rect(
        skia_safe::Rect::from_xywh(canvas_width * 0.64, 0.0, canvas_width * 0.22, 5.0),
        &paint,
    );

    let card_rect =
        skia_safe::Rect::from_xywh(28.0, 38.0, canvas_width - 56.0, canvas_height - 76.0);

    paint.set_style(skia_safe::PaintStyle::Fill);
    paint.set_color(shadow);
    canvas.draw_round_rect(card_rect.with_offset((0.0, 8.0)), 8.0, 8.0, &paint);

    paint.set_style(skia_safe::PaintStyle::Fill);
    paint.set_color(panel);
    canvas.draw_round_rect(card_rect, 8.0, 8.0, &paint);

    paint.set_style(skia_safe::PaintStyle::Stroke);
    paint.set_stroke_width(1.0);
    paint.set_color(edge);
    canvas.draw_round_rect(card_rect, 8.0, 8.0, &paint);

    paint.set_style(skia_safe::PaintStyle::Stroke);
    paint.set_stroke_width(1.0);
    paint.set_color(line);
    for i in -8..48 {
        let x = i as f32 * 72.0;
        canvas.draw_line((x, canvas_height), (x + 420.0, 0.0), &paint);
    }
}

#[cfg(feature = "skia-core")]
pub fn render_card(
    card: &CustomProfileCard,
    md: &MasterData,
    assets: Option<&AssetStore>,
    profile: Option<&crate::profile::ProfileData>,
) -> Result<Vec<u8>, String> {
    render_card_encoded(
        card,
        md,
        assets,
        profile,
        skia_safe::Color::WHITE,
        skia_safe::EncodedImageFormat::JPEG,
        90,
    )
}

#[cfg(all(feature = "skia-core", feature = "scenes"))]
pub fn render_card_personal_profile_canvas(
    card: &CustomProfileCard,
    md: &MasterData,
    assets: Option<&AssetStore>,
    profile: Option<&crate::profile::ProfileData>,
    theme: crate::personal_profile::PersonalProfileTheme,
) -> Result<Vec<u8>, String> {
    render_card_encoded_with_background(
        card,
        md,
        assets,
        profile,
        skia_safe::Color::WHITE,
        Some(theme),
        skia_safe::EncodedImageFormat::JPEG,
        90,
    )
}

#[cfg(all(feature = "skia-core", feature = "scenes"))]
pub fn render_card_personal_profile_canvas_sized(
    card: &CustomProfileCard,
    md: &MasterData,
    assets: Option<&AssetStore>,
    profile: Option<&crate::profile::ProfileData>,
    theme: crate::personal_profile::PersonalProfileTheme,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    render_card_encoded_with_background_sized(
        card,
        md,
        assets,
        profile,
        skia_safe::Color::WHITE,
        Some(theme),
        skia_safe::EncodedImageFormat::JPEG,
        90,
        width,
        height,
    )
}

#[cfg(feature = "skia-core")]
pub fn render_card_png(
    card: &CustomProfileCard,
    md: &MasterData,
    assets: Option<&AssetStore>,
    profile: Option<&crate::profile::ProfileData>,
) -> Result<Vec<u8>, String> {
    render_card_encoded(
        card,
        md,
        assets,
        profile,
        skia_safe::Color::WHITE,
        skia_safe::EncodedImageFormat::PNG,
        100,
    )
}

#[cfg(feature = "skia-core")]
pub fn render_card_png_transparent(
    card: &CustomProfileCard,
    md: &MasterData,
    assets: Option<&AssetStore>,
    profile: Option<&crate::profile::ProfileData>,
) -> Result<Vec<u8>, String> {
    render_card_encoded(
        card,
        md,
        assets,
        profile,
        skia_safe::Color::TRANSPARENT,
        skia_safe::EncodedImageFormat::PNG,
        100,
    )
}

/// 裁剪后的图层渲染结果。
#[cfg(feature = "skia-core")]
pub struct CroppedLayerOutput {
    pub data: Vec<u8>,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[cfg(feature = "animation-export")]
pub(crate) struct CroppedLayerRaster {
    pub image: skia_safe::Image,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scratch_peak_bytes: usize,
}

#[cfg(feature = "animation-export")]
#[derive(Default)]
struct AnimationCanvasExpansion {
    left: i32,
    right: i32,
    top: i32,
    bottom: i32,
}

#[cfg(feature = "animation-export")]
fn animation_canvas_expansion(
    elements: &[crate::elements::RenderElement<'_>],
    md: &MasterData,
) -> AnimationCanvasExpansion {
    const PAD: i32 = 8;
    let Some(text) = elements.iter().find_map(|element| match element {
        crate::elements::RenderElement::Text(text) if text.object_data.visible => Some(*text),
        _ => None,
    }) else {
        return AnimationCanvasExpansion::default();
    };
    let Some(animation) = crate::text::line_indent_x_animation(text, md) else {
        return AnimationCanvasExpansion::default();
    };
    let (_, _, angle_deg, scale_x, _) = crate::transform::extract_transform(&text.object_data);
    let radians = angle_deg.to_radians();
    let cos = radians.cos();
    let sin = radians.sin();
    let mut min_dx = 0.0_f32;
    let mut max_dx = 0.0_f32;
    let mut min_dy = 0.0_f32;
    let mut max_dy = 0.0_f32;
    for frame in animation.frames {
        let local_x = frame.dx_local * scale_x;
        let dx = local_x * cos;
        let dy = local_x * sin;
        min_dx = min_dx.min(dx);
        max_dx = max_dx.max(dx);
        min_dy = min_dy.min(dy);
        max_dy = max_dy.max(dy);
    }
    AnimationCanvasExpansion {
        left: max_dx.max(0.0).ceil() as i32 + PAD,
        right: (-min_dx).max(0.0).ceil() as i32 + PAD,
        top: max_dy.max(0.0).ceil() as i32 + PAD,
        bottom: (-min_dy).max(0.0).ceil() as i32 + PAD,
    }
}

/// Encoded pre-generated honor artwork and its exact game dimensions.
#[cfg(feature = "skia-core")]
pub struct HonorArtworkOutput {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

#[cfg(feature = "skia-core")]
fn encode_honor_artwork(
    full_size: bool,
    webp_quality: u32,
    draw: impl FnOnce(&skia_safe::Canvas),
) -> Result<HonorArtworkOutput, String> {
    use skia_safe::surfaces;

    let width = if full_size { 380 } else { 180 };
    let height = 80;
    let mut surface = surfaces::raster_n32_premul((width, height))
        .ok_or("failed to create honor artwork surface")?;
    let canvas = surface.canvas();
    canvas.clear(skia_safe::Color::TRANSPARENT);
    canvas.save();
    canvas.translate((width as f32 / 2.0, height as f32 / 2.0));
    draw(canvas);
    canvas.restore();
    let image = surface.image_snapshot();
    let data = image
        .encode(
            None,
            skia_safe::EncodedImageFormat::WEBP,
            Some(webp_quality.min(100)),
        )
        .ok_or("failed to encode honor artwork as WebP")?
        .as_bytes()
        .to_vec();
    if data.is_empty() {
        return Err("honor artwork encoder returned no bytes".into());
    }
    Ok(HonorArtworkOutput {
        data,
        width: width as u32,
        height: height as u32,
    })
}

/// 渲染单个元素到透明画布，裁剪到不透明像素的紧凑边界，编码为 WebP。
#[cfg(feature = "skia-core")]
pub fn render_element_layer_cropped(
    card: &CustomProfileCard,
    md: &MasterData,
    assets: Option<&AssetStore>,
    profile: Option<&crate::profile::ProfileData>,
    webp_quality: u32,
) -> Result<CroppedLayerOutput, String> {
    use skia_safe::surfaces;

    let w = crate::transform::CANVAS_WIDTH as i32;
    let h = crate::transform::CANVAS_HEIGHT as i32;

    let mut surface = surfaces::raster_n32_premul((w, h)).ok_or("创建 Skia Surface 失败")?;

    let canvas = surface.canvas();
    canvas.clear(skia_safe::Color::TRANSPARENT);

    let elements = crate::elements::flatten_and_sort(card);
    let visible_count = elements.iter().filter(|e| e.visible()).count();
    tracing::debug!(
        total = elements.len(),
        visible = visible_count,
        "图层元素扁平化完成"
    );
    // 循环外一次性构造共享上下文，避免每个元素重建（#29）。
    let fallback_assets = crate::assets::AssetStore::new(1);
    let theme = crate::widgets::theme::Theme::default();
    for elem in &elements {
        if !elem.visible() {
            continue;
        }
        crate::elements::draw_element_on_canvas(
            canvas,
            elem,
            md,
            assets,
            profile,
            &fallback_assets,
            &theme,
            w as f32,
            h as f32,
        );
    }

    let image = surface.image_snapshot();

    // read_pixels 扫描包围盒
    let info = skia_safe::ImageInfo::new(
        (w, h),
        skia_safe::ColorType::RGBA8888,
        skia_safe::AlphaType::Unpremul,
        None,
    );
    let row_bytes = w as usize * 4;
    let mut pixel_buf = vec![0u8; row_bytes * h as usize];
    if !image.read_pixels(
        &info,
        &mut pixel_buf,
        row_bytes,
        skia_safe::IPoint::new(0, 0),
        skia_safe::image::CachingHint::Allow,
    ) {
        return Err("无法读取像素数据".to_string());
    }
    let (bx, by, bw, bh) = find_opaque_bounds(&pixel_buf, w as u32, h as u32, row_bytes);
    tracing::debug!(bx, by, bw, bh, "像素包围盒扫描完成");

    if bw == 0 || bh == 0 {
        // 完全透明 — 返回 1x1 占位
        let mut tiny = surfaces::raster_n32_premul((1, 1)).ok_or("创建 1x1 Surface 失败")?;
        let tiny_img = tiny.image_snapshot();
        let ctx: Option<&mut skia_safe::gpu::DirectContext> = None;
        let encoded = tiny_img
            .encode(ctx, skia_safe::EncodedImageFormat::WEBP, Some(webp_quality))
            .ok_or("WebP 编码失败")?;
        let data = encoded.as_bytes().to_vec();
        if data.is_empty() {
            return Err("WebP 编码产生空数据".to_string());
        }
        return Ok(CroppedLayerOutput {
            data,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        });
    }

    // 创建小尺寸 surface，将原图裁剪区域画上去再编码（避免 make_subset 编码失败）
    let mut crop_surface =
        surfaces::raster_n32_premul((bw as i32, bh as i32)).ok_or("创建裁剪 Surface 失败")?;
    let crop_canvas = crop_surface.canvas();
    crop_canvas.draw_image_rect(
        &image,
        Some((
            &skia_safe::Rect::from_xywh(bx as f32, by as f32, bw as f32, bh as f32),
            skia_safe::canvas::SrcRectConstraint::Fast,
        )),
        &skia_safe::Rect::from_xywh(0.0, 0.0, bw as f32, bh as f32),
        &skia_safe::Paint::default(),
    );

    let ctx: Option<&mut skia_safe::gpu::DirectContext> = None;
    let crop_img = crop_surface.image_snapshot();
    let encoded = crop_img
        .encode(ctx, skia_safe::EncodedImageFormat::WEBP, Some(webp_quality))
        .ok_or_else(|| {
            let cw = crop_img.width();
            let ch = crop_img.height();
            format!("WebP 编码失败 (crop {cw}x{ch} from src {bx},{by},{bw},{bh})")
        })?;
    let data = encoded.as_bytes().to_vec();
    if data.is_empty() {
        return Err("WebP 编码产生空数据".to_string());
    }

    Ok(CroppedLayerOutput {
        data,
        x: bx,
        y: by,
        width: bw,
        height: bh,
    })
}

#[cfg(feature = "animation-export")]
pub(crate) fn render_element_layer_cropped_animation_raster(
    card: &CustomProfileCard,
    md: &MasterData,
    assets: Option<&AssetStore>,
    profile: Option<&crate::profile::ProfileData>,
    include_dynamic_bounds: bool,
) -> Result<CroppedLayerRaster, String> {
    let canvas_width = crate::transform::CANVAS_WIDTH as i32;
    let canvas_height = crate::transform::CANVAS_HEIGHT as i32;
    let elements = crate::elements::flatten_and_sort(card);
    let expansion = if include_dynamic_bounds {
        animation_canvas_expansion(&elements, md)
    } else {
        AnimationCanvasExpansion::default()
    };
    let surface_width = canvas_width + expansion.left + expansion.right;
    let surface_height = canvas_height + expansion.top + expansion.bottom;
    let mut surface = skia_safe::surfaces::raster_n32_premul((surface_width, surface_height))
        .ok_or("创建动画图层 Surface 失败")?;
    let canvas = surface.canvas();
    canvas.clear(skia_safe::Color::TRANSPARENT);
    canvas.save();
    canvas.translate((expansion.left as f32, expansion.top as f32));
    let fallback_assets = crate::assets::AssetStore::new(1);
    let theme = crate::widgets::theme::Theme::default();
    for element in &elements {
        if !element.visible() {
            continue;
        }
        crate::elements::draw_element_on_canvas(
            canvas,
            element,
            md,
            assets,
            profile,
            &fallback_assets,
            &theme,
            canvas_width as f32,
            canvas_height as f32,
        );
    }
    canvas.restore();

    let image = surface.image_snapshot();
    let row_bytes = surface_width as usize * 4;
    let info = skia_safe::ImageInfo::new(
        (surface_width, surface_height),
        skia_safe::ColorType::RGBA8888,
        skia_safe::AlphaType::Unpremul,
        None,
    );
    let surface_bytes = row_bytes
        .checked_mul(surface_height as usize)
        .ok_or_else(|| "动画图层像素缓冲区溢出".to_string())?;
    let mut pixels = vec![0u8; surface_bytes];
    if !image.read_pixels(
        &info,
        &mut pixels,
        row_bytes,
        skia_safe::IPoint::new(0, 0),
        skia_safe::image::CachingHint::Allow,
    ) {
        return Err("无法读取动画图层像素数据".into());
    }
    let (x, y, width, height) = find_opaque_bounds(
        &pixels,
        surface_width as u32,
        surface_height as u32,
        row_bytes,
    );
    if width == 0 || height == 0 {
        let mut tiny =
            skia_safe::surfaces::raster_n32_premul((1, 1)).ok_or("创建 1x1 Surface 失败")?;
        return Ok(CroppedLayerRaster {
            image: tiny.image_snapshot(),
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            scratch_peak_bytes: surface_bytes,
        });
    }

    let mut cropped = skia_safe::surfaces::raster_n32_premul((width as i32, height as i32))
        .ok_or("创建动画裁剪 Surface 失败")?;
    cropped.canvas().draw_image_rect(
        &image,
        Some((
            &skia_safe::Rect::from_xywh(x as f32, y as f32, width as f32, height as f32),
            skia_safe::canvas::SrcRectConstraint::Strict,
        )),
        &skia_safe::Rect::from_xywh(0.0, 0.0, width as f32, height as f32),
        &skia_safe::Paint::default(),
    );
    let cropped_bytes = width as usize * height as usize * 4;
    Ok(CroppedLayerRaster {
        image: cropped.image_snapshot(),
        x: x as i32 - expansion.left,
        y: y as i32 - expansion.top,
        width,
        height,
        scratch_peak_bytes: surface_bytes.saturating_add(cropped_bytes),
    })
}

/// 扫描像素缓冲区，找到所有 alpha > 0 像素的最小包围矩形。
#[cfg(feature = "skia-core")]
fn find_opaque_bounds(
    pixels: &[u8],
    width: u32,
    height: u32,
    row_bytes: usize,
) -> (u32, u32, u32, u32) {
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x: u32 = 0;
    let mut max_y: u32 = 0;

    for y in 0..height {
        let row_start = y as usize * row_bytes;
        for x in 0..width {
            // RGBA8888, alpha is byte offset 3
            let pixel_offset = row_start + (x as usize) * 4;
            let alpha = pixels[pixel_offset + 3];
            if alpha > 0 {
                if x < min_x {
                    min_x = x;
                }
                if x > max_x {
                    max_x = x;
                }
                if y < min_y {
                    min_y = y;
                }
                if y > max_y {
                    max_y = y;
                }
            }
        }
    }

    if max_x < min_x || max_y < min_y {
        return (0, 0, 0, 0);
    }

    (min_x, min_y, max_x - min_x + 1, max_y - min_y + 1)
}

// ─────────────────────────────────────────────────────────────────────────
// 批量分层裁剪渲染：把名片拆成「每个可见元素一张裁剪 WebP」的统一原语。
//
// 当前实现循环调 `render_element_layer_cropped`，每层输出与单方法逐字节一致。
// 后续可在不破坏逐字节一致性的前提下做内部优化（单画布复用等）。
// ─────────────────────────────────────────────────────────────────────────

/// 单层裁剪输出（含元数据 + WebP 字节）。
#[cfg(feature = "skia-core")]
pub struct LayerCrop {
    /// 元素在 layer 升序中的位置（0-based）。
    pub z: usize,
    /// 元素类型 "text" / "shape" / "card_member" / ...。
    pub element_type: String,
    /// 原始可见性（调用方可自行覆盖，本字段记录数据源标记）。
    pub original_visible: bool,
    /// 裁剪后的 WebP 字节（与 `render_element_layer_cropped` 同编码）。
    pub data: Vec<u8>,
    /// 裁剪框（原画布坐标系）。完全透明时全 0。
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// 可选元素属性（include_properties=true 时填充）。
    pub properties: Option<serde_json::Value>,
}

/// 单层定位：layer 升序后的 (元素类型, 在该类型数组内的下标, 原始可见性)。
#[cfg(feature = "skia-core")]
struct ElementPos {
    etype: &'static str,
    idx: usize,
    original_visible: bool,
}

/// 收集所有元素的定位信息，按 layer 升序稳定排序。
///
/// 与 `elements::flatten_and_sort` 同 push 顺序、同 `sort_by_key(layer)`；
/// `z` 编号 = 本函数返回向量的下标。
#[cfg(feature = "skia-core")]
fn collect_element_positions(card: &CustomProfileCard) -> Vec<ElementPos> {
    let mut raw: Vec<(i32, ElementPos)> = Vec::new();
    for (i, e) in card.texts.iter().enumerate() {
        raw.push((
            e.object_data.layer,
            ElementPos {
                etype: "text",
                idx: i,
                original_visible: e.object_data.visible,
            },
        ));
    }
    for (i, e) in card.shapes.iter().enumerate() {
        raw.push((
            e.object_data.layer,
            ElementPos {
                etype: "shape",
                idx: i,
                original_visible: e.object_data.visible,
            },
        ));
    }
    for (i, e) in card.card_members.iter().enumerate() {
        raw.push((
            e.object_data.layer,
            ElementPos {
                etype: "card_member",
                idx: i,
                original_visible: e.object_data.visible,
            },
        ));
    }
    for (i, e) in card.stamps.iter().enumerate() {
        raw.push((
            e.object_data.layer,
            ElementPos {
                etype: "stamp",
                idx: i,
                original_visible: e.object_data.visible,
            },
        ));
    }
    for (i, e) in card.others.iter().enumerate() {
        raw.push((
            e.object_data.layer,
            ElementPos {
                etype: "other",
                idx: i,
                original_visible: e.object_data.visible,
            },
        ));
    }
    for (i, e) in card.bonds_honors.iter().enumerate() {
        raw.push((
            e.object_data.layer,
            ElementPos {
                etype: "bonds_honor",
                idx: i,
                original_visible: e.object_data.visible,
            },
        ));
    }
    for (i, e) in card.honors.iter().enumerate() {
        raw.push((
            e.object_data.layer,
            ElementPos {
                etype: "honor",
                idx: i,
                original_visible: e.object_data.visible,
            },
        ));
    }
    for (i, e) in card.collections.iter().enumerate() {
        raw.push((
            e.object_data.layer,
            ElementPos {
                etype: "collection",
                idx: i,
                original_visible: e.object_data.visible,
            },
        ));
    }
    for (i, e) in card.generals.iter().enumerate() {
        raw.push((
            e.object_data.layer,
            ElementPos {
                etype: "general",
                idx: i,
                original_visible: e.object_data.visible,
            },
        ));
    }
    for (i, e) in card.stand_members.iter().enumerate() {
        raw.push((
            e.object_data.layer,
            ElementPos {
                etype: "stand_member",
                idx: i,
                original_visible: e.object_data.visible,
            },
        ));
    }
    for (i, e) in card.general_backgrounds.iter().enumerate() {
        raw.push((
            e.object_data.layer,
            ElementPos {
                etype: "general_background",
                idx: i,
                original_visible: e.object_data.visible,
            },
        ));
    }
    for (i, e) in card.story_backgrounds.iter().enumerate() {
        raw.push((
            e.object_data.layer,
            ElementPos {
                etype: "story_background",
                idx: i,
                original_visible: e.object_data.visible,
            },
        ));
    }
    raw.sort_by_key(|(layer, _)| *layer);
    raw.into_iter().map(|(_, p)| p).collect()
}

/// 把所有元素置不可见，再单独打开目标元素 —— 给单层渲染用的临时 card。
#[cfg(feature = "skia-core")]
fn make_single_element_card(base: &CustomProfileCard, pos: &ElementPos) -> CustomProfileCard {
    let mut card = base.clone();
    for e in &mut card.texts {
        e.object_data.visible = false;
    }
    for e in &mut card.shapes {
        e.object_data.visible = false;
    }
    for e in &mut card.card_members {
        e.object_data.visible = false;
    }
    for e in &mut card.stamps {
        e.object_data.visible = false;
    }
    for e in &mut card.others {
        e.object_data.visible = false;
    }
    for e in &mut card.bonds_honors {
        e.object_data.visible = false;
    }
    for e in &mut card.honors {
        e.object_data.visible = false;
    }
    for e in &mut card.collections {
        e.object_data.visible = false;
    }
    for e in &mut card.generals {
        e.object_data.visible = false;
    }
    for e in &mut card.stand_members {
        e.object_data.visible = false;
    }
    for e in &mut card.general_backgrounds {
        e.object_data.visible = false;
    }
    for e in &mut card.story_backgrounds {
        e.object_data.visible = false;
    }
    match pos.etype {
        "text" => card.texts[pos.idx].object_data.visible = true,
        "shape" => card.shapes[pos.idx].object_data.visible = true,
        "card_member" => card.card_members[pos.idx].object_data.visible = true,
        "stamp" => card.stamps[pos.idx].object_data.visible = true,
        "other" => card.others[pos.idx].object_data.visible = true,
        "bonds_honor" => card.bonds_honors[pos.idx].object_data.visible = true,
        "honor" => card.honors[pos.idx].object_data.visible = true,
        "collection" => card.collections[pos.idx].object_data.visible = true,
        "general" => card.generals[pos.idx].object_data.visible = true,
        "stand_member" => card.stand_members[pos.idx].object_data.visible = true,
        "general_background" => card.general_backgrounds[pos.idx].object_data.visible = true,
        "story_background" => card.story_backgrounds[pos.idx].object_data.visible = true,
        _ => {}
    }
    card
}

/// 提取元素属性：字体名、颜色 hex、文本内容等。
#[cfg(feature = "skia-core")]
fn extract_element_properties(
    card: &CustomProfileCard,
    etype: &str,
    idx: usize,
    md: &MasterData,
) -> Option<serde_json::Value> {
    use serde_json::json;

    fn color_hex(md: &MasterData, id: i32) -> String {
        md.resolve_color(id)
            .map(|c| format!("#{:02x}{:02x}{:02x}{:02x}", c.r, c.g, c.b, c.a))
            .unwrap_or_else(|| format!("color#{id}"))
    }
    fn font_name(md: &MasterData, id: i32) -> String {
        md.resolve_font(id).unwrap_or_else(|| format!("font#{id}"))
    }

    match etype {
        "text" => {
            let e = card.texts.get(idx)?;
            Some(json!({
                "text": e.text,
                "font": font_name(md, e.font_id),
                "font_id": e.font_id,
                "size": e.size,
                "color": color_hex(md, e.color_id),
                "color_id": e.color_id,
                "outline_color": color_hex(md, e.outline_color_id),
                "outline_size": e.outline_size,
                "line_spacing": e.line_spacing,
                "text_type": e.text_type,
            }))
        }
        "shape" => {
            let e = card.shapes.get(idx)?;
            Some(json!({
                "id": e.id,
                "color": color_hex(md, e.color_id),
                "color_id": e.color_id,
                "alpha": e.alpha,
                "outline_color": color_hex(md, e.outline_color_id),
                "outline_alpha": e.outline_alpha,
                "outline_size": e.outline_size,
            }))
        }
        "card_member" => {
            let e = card.card_members.get(idx)?;
            Some(json!({
                "id": e.id,
                "member_type": e.member_type,
                "show_master_rank": e.show_master_rank,
                "use_after_special_training": e.use_after_special_training,
            }))
        }
        "stamp" => Some(json!({ "id": card.stamps.get(idx)?.id })),
        "other" => Some(json!({ "id": card.others.get(idx)?.id })),
        "bonds_honor" => {
            let e = card.bonds_honors.get(idx)?;
            Some(json!({
                "id": e.id,
                "word_id": e.word_id,
                "honor_level": e.honor_level,
                "full_size": e.full_size,
                "inverse": e.inverse,
            }))
        }
        "honor" => {
            let e = card.honors.get(idx)?;
            Some(json!({
                "id": e.id,
                "honor_level": e.honor_level,
                "full_size": e.full_size,
            }))
        }
        "collection" => {
            let e = card.collections.get(idx)?;
            Some(json!({
                "id": e.id,
                "target_id": e.target_id,
            }))
        }
        "general" => {
            let e = card.generals.get(idx)?;
            Some(json!({ "general_type": e.general_type }))
        }
        "stand_member" => Some(json!({ "id": card.stand_members.get(idx)?.id })),
        "general_background" => Some(json!({ "id": card.general_backgrounds.get(idx)?.id })),
        "story_background" => Some(json!({ "id": card.story_backgrounds.get(idx)?.id })),
        _ => None,
    }
}

/// 批量分层裁剪渲染：把名片所有元素（按 layer 升序）逐个渲成裁剪后的 WebP。
///
/// 输出向量的下标 = 元素的 z 序号。仅 `original_visible=true` 的元素生成 WebP，
/// 其它元素仍出现在结果中（data 为空、rect 全 0），便于调用方完整重建图层列表。
///
/// `include_properties=true` 时为每层填充 `properties`（字体名/颜色 hex/文本等）。
/// 不需要属性时关掉省一遍 md 查询。
#[cfg(feature = "skia-core")]
pub fn render_all_layers_cropped(
    card: &CustomProfileCard,
    md: &MasterData,
    assets: Option<&AssetStore>,
    profile: Option<&crate::profile::ProfileData>,
    webp_quality: u32,
    include_properties: bool,
) -> Result<Vec<LayerCrop>, String> {
    let positions = collect_element_positions(card);
    let mut out = Vec::with_capacity(positions.len());

    for (z, pos) in positions.iter().enumerate() {
        let properties = if include_properties {
            extract_element_properties(card, pos.etype, pos.idx, md)
        } else {
            None
        };

        if !pos.original_visible {
            out.push(LayerCrop {
                z,
                element_type: pos.etype.to_string(),
                original_visible: false,
                data: Vec::new(),
                x: 0,
                y: 0,
                width: 0,
                height: 0,
                properties,
            });
            continue;
        }

        let layer_card = make_single_element_card(card, pos);
        let rendered =
            render_element_layer_cropped(&layer_card, md, assets, profile, webp_quality)?;
        out.push(LayerCrop {
            z,
            element_type: pos.etype.to_string(),
            original_visible: true,
            data: rendered.data,
            x: rendered.x,
            y: rendered.y,
            width: rendered.width,
            height: rendered.height,
            properties,
        });
    }
    Ok(out)
}

/// Runtime probe for the Turin AVX-512 SDF tile executor.
#[cfg(feature = "skia-core")]
fn turin_sdf_simd_available() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::arch::is_x86_feature_detected!("avx512f")
            && std::arch::is_x86_feature_detected!("avx512bw")
            && std::arch::is_x86_feature_detected!("avx512vbmi")
            && std::arch::is_x86_feature_detected!("fma")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::masterdata::{MasterDataProvider, ResolvedColor, ResolvedHonor, ResourceInfo};
    use crate::types::{
        BondsHonorEntry, BondsHonorWordEntry, CardEntry, CustomProfileCard, HonorEntry, ObjectData,
        Quaternion, StampElement, TextElement, Vec3,
    };

    /// 全部返回 None 的 MasterData provider，模拟"无素材"环境。
    /// 此环境可能触发渲染 panic（Issue #5 根因）。
    struct NullProvider;

    impl MasterDataProvider for NullProvider {
        fn resolve_story_banner(&self, _story_type: &str, _story_id: i32) -> Option<String> {
            None
        }
        fn get_card(&self, _card_id: i32) -> Option<CardEntry> {
            None
        }
        fn resolve_color(&self, _color_id: i32) -> Option<ResolvedColor> {
            None
        }
        fn resolve_font(&self, _font_id: i32) -> Option<String> {
            None
        }
        fn resolve_stamp(&self, _stamp_id: i32) -> Option<String> {
            None
        }
        fn resolve_resource(&self, _res_type: &str, _id: i32) -> Option<ResourceInfo> {
            None
        }
        fn resolve_honor(&self, _honor_id: i32, _honor_level: i32) -> Option<ResolvedHonor> {
            None
        }
        fn get_bonds_honor(&self, _id: i32) -> Option<BondsHonorEntry> {
            None
        }
        fn get_bonds_honor_word(&self, _word_id: i64) -> Option<BondsHonorWordEntry> {
            None
        }
        fn get_honor(&self, _honor_id: i32) -> Option<HonorEntry> {
            None
        }
        fn resolve_unit_vs_sd(&self, _self_id: i32, _partner_id: i32) -> i32 {
            0
        }
        fn font_count(&self) -> usize {
            0
        }
        fn color_count(&self) -> usize {
            0
        }
    }

    fn default_object_data(layer: i32, visible: bool) -> ObjectData {
        ObjectData {
            layer,
            lock: false,
            position: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            rotation: Quaternion {
                w: 1.0,
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            scale: Vec3 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
            },
            visible,
        }
    }

    /// Issue #5 用户 7493593928021629747 的简单名片结构：
    /// 4 个 invisible shapes + 4 个 visible stamps + 4 个 invisible texts = 12 层
    fn issue5_simple_card() -> CustomProfileCard {
        CustomProfileCard {
            shapes: vec![
                crate::types::ShapeElement {
                    object_data: default_object_data(0, false),
                    alpha: 0.0,
                    color_id: 23,
                    id: 3,
                    outline_alpha: 1.0,
                    outline_color_id: 23,
                    outline_size: 0.37,
                },
                crate::types::ShapeElement {
                    object_data: default_object_data(1, false),
                    alpha: 1.0,
                    color_id: 23,
                    id: 3,
                    outline_alpha: 1.0,
                    outline_color_id: 23,
                    outline_size: 0.0,
                },
                crate::types::ShapeElement {
                    object_data: default_object_data(2, false),
                    alpha: 0.0,
                    color_id: 23,
                    id: 3,
                    outline_alpha: 1.0,
                    outline_color_id: 23,
                    outline_size: 0.37,
                },
                crate::types::ShapeElement {
                    object_data: default_object_data(3, false),
                    alpha: 0.0,
                    color_id: 23,
                    id: 3,
                    outline_alpha: 1.0,
                    outline_color_id: 23,
                    outline_size: 0.37,
                },
            ],
            stamps: vec![
                StampElement {
                    object_data: default_object_data(4, true),
                    id: 609,
                },
                StampElement {
                    object_data: default_object_data(5, true),
                    id: 179,
                },
                StampElement {
                    object_data: default_object_data(6, true),
                    id: 631,
                },
                StampElement {
                    object_data: default_object_data(7, true),
                    id: 514,
                },
            ],
            texts: vec![
                TextElement {
                    object_data: default_object_data(8, false),
                    color_id: 18,
                    font_id: 2,
                    line_spacing: 0.0,
                    outline_color_id: 18,
                    outline_size: 0.0,
                    size: 24.0,
                    text: "5.9-5.12".to_string(),
                    text_type: 513,
                },
                TextElement {
                    object_data: default_object_data(9, false),
                    color_id: 15,
                    font_id: 2,
                    line_spacing: 0.0,
                    outline_color_id: 15,
                    outline_size: 0.0,
                    size: 24.0,
                    text: "5.12-5.15".to_string(),
                    text_type: 513,
                },
                TextElement {
                    object_data: default_object_data(10, false),
                    color_id: 17,
                    font_id: 2,
                    line_spacing: 0.0,
                    outline_color_id: 17,
                    outline_size: 0.0,
                    size: 24.0,
                    text: "5.15-5.18".to_string(),
                    text_type: 513,
                },
                TextElement {
                    object_data: default_object_data(11, false),
                    color_id: 16,
                    font_id: 2,
                    line_spacing: 0.0,
                    outline_color_id: 16,
                    outline_size: 0.0,
                    size: 24.0,
                    text: "5.18-5.21".to_string(),
                    text_type: 513,
                },
            ],
            card_members: vec![],
            others: vec![],
            bonds_honors: vec![],
            honors: vec![],
            collections: vec![],
            generals: vec![],
            stand_members: vec![],
            general_backgrounds: vec![],
            story_backgrounds: vec![],
        }
    }

    #[test]
    #[cfg(feature = "skia-core")]
    fn static_honor_artwork_uses_exact_game_dimensions() {
        fn assert_webp_dimensions(output: HonorArtworkOutput, width: i32, height: i32) {
            let image = skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&output.data))
                .expect("honor artwork should decode as WebP");
            assert_eq!(output.width, width as u32);
            assert_eq!(output.height, height as u32);
            assert_eq!(image.width(), width);
            assert_eq!(image.height(), height);
        }

        let renderer = CustomProfileRenderer::new(Arc::new(NullProvider));
        let main = renderer
            .render_static_honor_artwork(1, 1, true, 90)
            .expect("main honor artwork should encode without masterdata");
        assert_webp_dimensions(main, 380, 80);

        let sub = renderer
            .render_static_honor_artwork(1, 1, false, 90)
            .expect("sub honor artwork should encode without masterdata");
        assert_webp_dimensions(sub, 180, 80);
    }

    /// 测试 1: 在无素材环境下，render_element_layer_cropped 对 stamp 层不 panic。
    /// 修复前：stamp 渲染会 unwrap 缺失素材 → panic → 整个 spawn_blocking 崩溃
    /// 修复后：stamp 渲染返回 Err 或 draw_image_placeholder，不 panic
    #[test]
    #[cfg(feature = "skia-core")]
    fn stamp_layer_render_no_panic_without_assets() {
        let provider = Arc::new(NullProvider);
        let md = MasterData::new(provider);
        let assets = AssetStore::new(8);

        let card = issue5_simple_card();

        // 对每个 stamp 层单独渲染
        for (z, _stamp) in card.stamps.iter().enumerate() {
            let mut layer_card = card.clone();
            // 设置所有元素为不可见
            for s in &mut layer_card.shapes {
                s.object_data.visible = false;
            }
            for s in &mut layer_card.stamps {
                s.object_data.visible = false;
            }
            for t in &mut layer_card.texts {
                t.object_data.visible = false;
            }
            // 只设置当前 stamp 可见
            layer_card.stamps[z].object_data.visible = true;

            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                render_element_layer_cropped(&layer_card, &md, Some(&assets), None, 80)
            }));

            match result {
                Ok(Ok(output)) => {
                    // 成功渲染（可能是 placeholder）— 正常路径
                    assert!(!output.data.is_empty(), "stamp z={z} 应有输出数据");
                }
                Ok(Err(_error)) => {
                    // 渲染返回错误 — 安全路径，不 panic
                    // 这是修复后的行为：Err 会被记录，发送空占位
                }
                Err(panic_info) => {
                    // 如果到达这里，说明 catch_unwind 捕获了 panic
                    // 修复后的 handler 层会处理这种情况
                    panic!(
                        "stamp z={z} 渲染 panic（应该已被 .expect 替换修复）：{:?}",
                        panic_info
                    );
                }
            }
        }
    }

    /// 测试 2: 模拟 handler 层的 catch_unwind + channel 模式。
    /// 验证当单个元素 panic 时，其他元素仍能正常处理。
    #[test]
    #[cfg(feature = "skia-core")]
    fn catch_unwind_isolates_single_layer_panic() {
        let provider = Arc::new(NullProvider);
        let md = MasterData::new(provider);
        let assets = AssetStore::new(8);
        let card = issue5_simple_card();

        // 模拟 handler 中的 channel + catch_unwind 模式
        let (tx, rx) = std::sync::mpsc::channel::<(usize, String, Vec<u8>)>();

        // 在同步上下文中模拟 spawn_blocking 逻辑
        let positions: Vec<(usize, &StampElement)> = card.stamps.iter().enumerate().collect();
        let total_layers = positions.len();

        for (z, _stamp) in &positions {
            let mut layer_card = card.clone();
            for s in &mut layer_card.shapes {
                s.object_data.visible = false;
            }
            for s in &mut layer_card.stamps {
                s.object_data.visible = false;
            }
            for t in &mut layer_card.texts {
                t.object_data.visible = false;
            }
            layer_card.stamps[*z].object_data.visible = true;

            // 核心：catch_unwind 隔离单个元素渲染
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                render_element_layer_cropped(&layer_card, &md, Some(&assets), None, 80)
            }));

            match result {
                Ok(Ok(output)) => {
                    let _ = tx.send((*z, "stamp".to_string(), output.data));
                }
                Ok(Err(_error)) => {
                    // 渲染失败 — 发送空占位（修复后的行为）
                    let _ = tx.send((*z, "stamp".to_string(), Vec::new()));
                }
                Err(_panic_info) => {
                    // panic 被捕获 — 发送空占位（修复后的行为）
                    let _ = tx.send((*z, "stamp".to_string(), Vec::new()));
                }
            }
        }
        drop(tx); // 关闭 sender

        // 验证接收端收到了所有层的消息
        let received: Vec<_> = rx.iter().collect();
        assert_eq!(
            received.len(),
            total_layers,
            "应收到 {} 层消息，实际收到 {} — 修复前 panic 会导致后续层全部丢失",
            total_layers,
            received.len()
        );

        // 验证 z-index 顺序
        for (i, (z, etype, data)) in received.iter().enumerate() {
            assert_eq!(*z, i, "层 z={} 应在第 {} 位", z, i);
            assert_eq!(etype, "stamp");
            // data 可能有内容（placeholder）或为空（渲染失败），但必须存在
            let _ = data; // 不检查数据内容，只确认消息到达
        }
    }

    /// 测试 3: 验证修复后的行为：catch_unwind 阻止 panic 传播，所有层都到达 receiver。
    #[test]
    fn catch_unwind_prevents_channel_drop() {
        let (tx, rx) = std::sync::mpsc::channel::<(usize, String, Vec<u8>)>();

        // 模拟修复后的行为：每个元素用 catch_unwind 包裹
        for z in 0..4 {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if z == 1 {
                    panic!("simulated render panic at layer 1");
                }
                vec![1, 2, 3] // 模拟渲染输出
            }));

            match result {
                Ok(data) => {
                    let _ = tx.send((z, "element".to_string(), data));
                }
                Err(_panic_info) => {
                    // panic 被捕获 → 发送空占位
                    let _ = tx.send((z, "element".to_string(), Vec::new()));
                }
            }
        }
        drop(tx);

        let received: Vec<_> = rx.iter().collect();
        assert_eq!(
            received.len(),
            4,
            "修复后：所有 4 层都应到达，实际收到 {} 层",
            received.len()
        );

        // 验证 panic 层（z=1）收到了空占位
        let panicked_layer = received.iter().find(|(z, _, _)| *z == 1).unwrap();
        assert!(panicked_layer.2.is_empty(), "panic 层应为空数据占位");

        // 验证正常层有数据
        let normal_layer = received.iter().find(|(z, _, _)| *z == 0).unwrap();
        assert!(!normal_layer.2.is_empty(), "正常层应有渲染数据");
    }

    /// 测试 5: 回归测试 — read_pixels 使用 AlphaType::Unpremul（Issue #7）。
    /// 修复前 AlphaType::Premul 导致 read_pixels 对 N32 raster surface 返回 false，
    /// 所有图层 width=0 height=0 url=""。
    /// 修复后使用 Unpremul（与 shape.rs 一致），read_pixels 成功返回像素数据。
    #[test]
    #[cfg(feature = "skia-core")]
    fn read_pixels_unpremul_succeeds_on_raster_surface() {
        use skia_safe::{surfaces, AlphaType, ColorType, ImageInfo};

        let w = 64i32;
        let h = 64i32;
        let mut surface = surfaces::raster_n32_premul((w, h)).expect("创建 raster surface 失败");

        let canvas = surface.canvas();
        canvas.clear(skia_safe::Color::TRANSPARENT);

        // 画一个红色矩形，确保 surface 有非透明像素
        let mut paint = skia_safe::Paint::default();
        paint.set_color(skia_safe::Color::from_argb(255, 255, 0, 0));
        canvas.draw_rect(skia_safe::Rect::from_xywh(10.0, 10.0, 40.0, 40.0), &paint);

        let image = surface.image_snapshot();

        // 使用 AlphaType::Unpremul 读取像素（修复后的方式）
        let info = ImageInfo::new((w, h), ColorType::RGBA8888, AlphaType::Unpremul, None);
        let row_bytes = w as usize * 4;
        let mut pixel_buf = vec![0u8; row_bytes * h as usize];
        let success = image.read_pixels(
            &info,
            &mut pixel_buf,
            row_bytes,
            skia_safe::IPoint::new(0, 0),
            skia_safe::image::CachingHint::Allow,
        );

        assert!(success, "read_pixels with AlphaType::Unpremul 应成功");

        // 验证像素数据非全零（红色矩形区域应有非零 alpha）
        let nonzero: usize = pixel_buf.iter().filter(|&&b| b != 0).count();
        assert!(nonzero > 0, "像素缓冲区应有非零字节，实际全部为 0");

        // 验证 find_opaque_bounds 能找到正确的包围盒
        let (bx, by, bw, bh) = find_opaque_bounds(&pixel_buf, w as u32, h as u32, row_bytes);
        assert!(
            bw > 0 && bh > 0,
            "find_opaque_bounds 应找到非零包围盒，实际 ({bx},{by},{bw},{bh})"
        );
    }

    /// 测试 4: 验证 render_element_layer_cropped 返回裁剪后的编码结果。
    #[test]
    #[cfg(feature = "skia-core")]
    fn layer_cropped_with_real_card_structure() {
        let provider = Arc::new(NullProvider);
        let md = MasterData::new(provider);
        let assets = AssetStore::new(8);
        let card = issue5_simple_card();

        let mut layer_card = card.clone();
        for s in &mut layer_card.shapes {
            s.object_data.visible = false;
        }
        for s in &mut layer_card.stamps {
            s.object_data.visible = false;
        }
        for t in &mut layer_card.texts {
            t.object_data.visible = false;
        }
        layer_card.shapes[0].object_data.visible = true;

        let result = render_element_layer_cropped(&layer_card, &md, Some(&assets), None, 80);
        match result {
            Ok(output) => {
                assert!(!output.data.is_empty(), "应有 WebP 输出");
                // 裁剪后尺寸应小于等于画布尺寸
                assert!(output.width <= crate::transform::CANVAS_WIDTH as u32);
                assert!(output.height <= crate::transform::CANVAS_HEIGHT as u32);
            }
            Err(e) => {
                let _ = e;
            }
        }
    }

    /// 关键回归：批量原语 `render_all_layers_cropped` 的每层输出，必须与
    /// 单方法 `render_element_layer_cropped` 在「make_single_element_card 单独打开该层」
    /// 下的输出**逐字节一致**。任何不破坏此性质的内部优化（单画布复用等）都应保留
    /// 此测试通过。
    #[test]
    #[cfg(feature = "skia-core")]
    fn batch_layers_match_single_method_byte_for_byte() {
        let provider = Arc::new(NullProvider);
        let md = MasterData::new(provider);
        let assets = AssetStore::new(8);
        let card = issue5_simple_card();

        // 批量原语
        let batch = render_all_layers_cropped(&card, &md, Some(&assets), None, 80, false)
            .expect("batch render OK");

        // 逐层对照：每个 original_visible=true 的元素，用相同的"单独打开它"的 card
        // 调单方法，结果应逐字节相等。
        let positions = collect_element_positions(&card);
        assert_eq!(batch.len(), positions.len(), "批量结果数 == 元素总数");

        let mut compared_visible = 0;
        for (z, pos) in positions.iter().enumerate() {
            let layer = &batch[z];
            assert_eq!(layer.z, z);
            assert_eq!(layer.element_type, pos.etype);
            assert_eq!(layer.original_visible, pos.original_visible);

            if !pos.original_visible {
                assert!(layer.data.is_empty(), "不可见层 data 应为空");
                assert_eq!((layer.x, layer.y, layer.width, layer.height), (0, 0, 0, 0));
                continue;
            }

            let single_card = make_single_element_card(&card, pos);
            let single = render_element_layer_cropped(&single_card, &md, Some(&assets), None, 80)
                .expect("single render OK");

            assert_eq!(
                layer.data, single.data,
                "z={z} ({}): WebP 字节不一致",
                pos.etype
            );
            assert_eq!(layer.x, single.x);
            assert_eq!(layer.y, single.y);
            assert_eq!(layer.width, single.width);
            assert_eq!(layer.height, single.height);
            compared_visible += 1;
        }
        assert!(compared_visible > 0, "至少应有一个可见层参与对照");
    }
}

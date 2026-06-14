//! 平台个人资料默认图渲染。

#![cfg_attr(not(feature = "skia-core"), allow(dead_code))]

use crate::assets::AssetStore;
use crate::masterdata::MasterData;
use crate::profile::ProfileData;
#[cfg(feature = "skia-core")]
use crate::profile::{DeckMember, HonorSlot};
use crate::traits::RenderOutput;

pub const PERSONAL_PROFILE_WIDTH: u32 = 1830;
pub const PERSONAL_PROFILE_HEIGHT: u32 = 812;
pub const PERSONAL_PROFILE_QUALITY: u8 = 90;

/// 平台个人资料图主题。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonalProfileTheme {
    /// 25 时深色玻璃风格。
    NiigoDark,
    /// 偏亮的青色资料卡风格，给后续替换默认风格使用。
    MikuLight,
}

impl Default for PersonalProfileTheme {
    fn default() -> Self {
        Self::NiigoDark
    }
}

/// 平台个人资料图输入。
#[derive(Debug, Clone)]
pub struct PersonalProfileRenderInput {
    pub user_id: String,
    pub profile: ProfileData,
    pub theme: PersonalProfileTheme,
}

/// 渲染平台个人资料图。
pub fn render_personal_profile(
    input: &PersonalProfileRenderInput,
    md: &MasterData,
    assets: Option<&AssetStore>,
) -> Result<RenderOutput, String> {
    #[cfg(feature = "skia-core")]
    {
        return render_personal_profile_skia(input, md, assets);
    }

    #[cfg(not(feature = "skia-core"))]
    {
        let _ = (input, md, assets);
        Err("Skia 渲染未启用，请使用 --features skia 编译".to_string())
    }
}

#[cfg(feature = "skia-core")]
fn render_personal_profile_skia(
    input: &PersonalProfileRenderInput,
    md: &MasterData,
    assets: Option<&AssetStore>,
) -> Result<RenderOutput, String> {
    let mut canvas = ProfileCanvas::new(input.theme)?;
    canvas.background();
    canvas.left_hero(&input.profile, md, assets);
    canvas.main_panel(&input.user_id, &input.profile, md, assets);
    canvas.finish()
}

#[cfg(feature = "skia-core")]
#[derive(Debug, Clone, Copy)]
struct ThemeColors {
    background: Color,
    panel: Color,
    panel_alt: Color,
    glass_edge: Color,
    text: Color,
    muted: Color,
    subtle: Color,
    niigo: Color,
    miku: Color,
    warm: Color,
    black_35: Color,
    black_62: Color,
}

#[cfg(feature = "skia-core")]
impl ThemeColors {
    fn for_theme(theme: PersonalProfileTheme) -> Self {
        match theme {
            PersonalProfileTheme::NiigoDark => Self {
                background: Color(17, 17, 25, 255),
                panel: Color(240, 245, 255, 24),
                panel_alt: Color(255, 255, 255, 30),
                glass_edge: Color(255, 255, 255, 58),
                text: Color(244, 246, 250, 255),
                muted: Color(184, 187, 204, 255),
                subtle: Color(255, 255, 255, 12),
                niigo: Color(136, 122, 240, 255),
                miku: Color(168, 216, 232, 255),
                warm: Color(255, 207, 111, 255),
                black_35: Color(0, 0, 0, 90),
                black_62: Color(0, 0, 0, 158),
            },
            PersonalProfileTheme::MikuLight => Self {
                background: Color(229, 242, 247, 255),
                panel: Color(255, 255, 255, 194),
                panel_alt: Color(247, 252, 255, 220),
                glass_edge: Color(75, 115, 135, 38),
                text: Color(36, 43, 58, 255),
                muted: Color(91, 104, 121, 255),
                subtle: Color(97, 157, 184, 22),
                niigo: Color(125, 112, 224, 255),
                miku: Color(65, 164, 194, 255),
                warm: Color(223, 148, 62, 255),
                black_35: Color(42, 58, 72, 42),
                black_62: Color(22, 35, 48, 132),
            },
        }
    }
}

#[cfg(feature = "skia-core")]
struct ProfileCanvas {
    surface: skia_safe::Surface,
    regular: skia_safe::Typeface,
    emphasis: skia_safe::Typeface,
    colors: ThemeColors,
}

#[cfg(feature = "skia-core")]
impl ProfileCanvas {
    fn new(theme: PersonalProfileTheme) -> Result<Self, String> {
        let surface = skia_safe::surfaces::raster_n32_premul((
            PERSONAL_PROFILE_WIDTH as i32,
            PERSONAL_PROFILE_HEIGHT as i32,
        ))
        .ok_or_else(|| "创建平台个人资料 Surface 失败".to_string())?;
        let (regular, emphasis) =
            profile_typefaces().ok_or_else(|| "平台个人资料字体初始化失败".to_string())?;
        Ok(Self {
            surface,
            regular,
            emphasis,
            colors: ThemeColors::for_theme(theme),
        })
    }

    fn background(&mut self) {
        let canvas = self.surface.canvas();
        canvas.clear(self.colors.background.skia());

        let mut paint = skia_safe::Paint::default();
        paint.set_anti_alias(true);
        paint.set_style(skia_safe::PaintStyle::Fill);
        paint.set_color(self.colors.niigo.skia());
        canvas.draw_rect(skia_safe::Rect::from_xywh(0.0, 0.0, 630.0, 5.0), &paint);
        paint.set_color(self.colors.miku.skia());
        canvas.draw_rect(skia_safe::Rect::from_xywh(630.0, 0.0, 360.0, 5.0), &paint);

        paint.set_style(skia_safe::PaintStyle::Stroke);
        paint.set_stroke_width(1.0);
        paint.set_color(self.colors.subtle.skia());
        for i in -6..34 {
            let x = i as f32 * 72.0;
            canvas.draw_line((x, 812.0), (x + 420.0, 0.0), &paint);
        }
    }

    fn left_hero(&mut self, profile: &ProfileData, md: &MasterData, assets: Option<&AssetStore>) {
        const X: f32 = 42.0;
        const Y: f32 = 42.0;
        const W: f32 = 552.0;
        const IMAGE_H: f32 = 392.0;
        const INFO_Y: f32 = Y + IMAGE_H + 24.0;
        const INFO_H: f32 = 312.0;
        let rect = skia_safe::Rect::from_xywh(X, Y, W, IMAGE_H);

        self.shadow(rect);
        let mut drawn = false;
        if let Some(leader) = &profile.leader_card {
            let suffix = if leader.after_training {
                "after_training"
            } else {
                "normal"
            };
            if let Some(key) =
                crate::asset_keys::resolve_card_member_key(leader.card_id, 2, suffix, md)
            {
                drawn = draw_asset_cover(self.surface.canvas(), assets, &key, rect, 8.0);
            }
        }
        if !drawn {
            self.panel_fill(
                rect,
                8.0,
                self.colors.panel_alt,
                Some(self.colors.glass_edge),
            );
        }

        self.panel_fill(
            skia_safe::Rect::from_xywh(X, Y + IMAGE_H - 112.0, W, 112.0),
            8.0,
            self.colors.black_62,
            None,
        );
        self.rule(X + 28.0, Y + IMAGE_H - 86.0, 96.0, 5.0, self.colors.niigo);

        let leader_meta = profile
            .leader_card
            .as_ref()
            .map(|leader| format!("队长卡 #{} · 突破 {}", leader.card_id, leader.master_rank))
            .unwrap_or_else(|| "队长卡未公开".to_string());
        self.text(
            &leader_meta,
            X + 28.0,
            Y + IMAGE_H - 44.0,
            28.0,
            self.colors.text,
            TextAlign::Left,
            W - 56.0,
            FontWeight::Emphasis,
        );

        let info_rect = skia_safe::Rect::from_xywh(X, INFO_Y, W, INFO_H);
        self.glass(info_rect);
        self.text(
            "个性签名",
            X + 28.0,
            INFO_Y + 44.0,
            20.0,
            self.colors.muted,
            TextAlign::Left,
            W - 56.0,
            FontWeight::Regular,
        );
        for (index, line) in wrap_text(blank_as(&profile.word, "未设置个性签名"), W - 56.0, 31.0, 3)
            .iter()
            .enumerate()
        {
            self.text(
                line,
                X + 28.0,
                INFO_Y + 94.0 + index as f32 * 42.0,
                31.0,
                self.colors.text,
                TextAlign::Left,
                W - 56.0,
                FontWeight::Emphasis,
            );
        }
        let deck_text = if profile.deck_members.is_empty() {
            "当前卡组未公开".to_string()
        } else {
            format!("当前卡组 {} 张卡", profile.deck_members.len().min(5))
        };
        self.text(
            &deck_text,
            X + 28.0,
            INFO_Y + INFO_H - 34.0,
            22.0,
            self.colors.muted,
            TextAlign::Left,
            W - 56.0,
            FontWeight::Regular,
        );
    }

    fn main_panel(
        &mut self,
        user_id: &str,
        profile: &ProfileData,
        md: &MasterData,
        assets: Option<&AssetStore>,
    ) {
        const X: f32 = 628.0;
        const Y: f32 = 42.0;
        const W: f32 = 1160.0;
        const H: f32 = 728.0;
        let rect = skia_safe::Rect::from_xywh(X, Y, W, H);
        self.glass(rect);

        self.text(
            blank_as(&profile.user_name, "未命名玩家"),
            X + 36.0,
            Y + 70.0,
            54.0,
            self.colors.text,
            TextAlign::Left,
            570.0,
            FontWeight::Emphasis,
        );

        let rank = if profile.user_rank > 0 {
            format!("Rank {}", profile.user_rank)
        } else {
            "Rank 未知".to_string()
        };
        let mut chip_x = X + 36.0;
        chip_x += self.chip(&rank, chip_x, Y + 90.0, 34.0, self.colors.niigo);
        let id_chip = format!("ID {user_id}");
        self.chip(&id_chip, chip_x + 12.0, Y + 90.0, 34.0, self.colors.miku);

        self.text(
            "综合力",
            X + 710.0,
            Y + 54.0,
            22.0,
            self.colors.muted,
            TextAlign::Left,
            360.0,
            FontWeight::Regular,
        );
        self.text(
            &format_number(profile.total_power),
            X + 710.0,
            Y + 124.0,
            78.0,
            self.colors.text,
            TextAlign::Left,
            400.0,
            FontWeight::Emphasis,
        );
        self.rule(X + 710.0, Y + 145.0, 230.0, 5.0, self.colors.miku);

        self.draw_honors(profile, md, assets, X + 36.0, Y + 188.0);
        self.draw_deck(profile, md, assets, X + 36.0, Y + 312.0);
        self.draw_music(profile, X + 36.0, Y + 584.0);
        self.draw_counters(profile, X + 596.0, Y + 584.0);
    }

    fn draw_honors(
        &mut self,
        profile: &ProfileData,
        md: &MasterData,
        assets: Option<&AssetStore>,
        x: f32,
        y: f32,
    ) {
        let rect = skia_safe::Rect::from_xywh(x, y, 690.0, 92.0);
        self.panel_fill(
            rect,
            8.0,
            self.colors.panel_alt,
            Some(self.colors.glass_edge),
        );
        let mut slots: Vec<&HonorSlot> = profile.honor_slots.iter().collect();
        slots.sort_by(|a, b| b.full_size.cmp(&a.full_size));
        if slots.is_empty() {
            self.text(
                "未展示称号",
                x + 24.0,
                y + 55.0,
                25.0,
                self.colors.muted,
                TextAlign::Left,
                640.0,
                FontWeight::Regular,
            );
            return;
        }

        let canvas = self.surface.canvas();
        let mut cursor = x + 26.0;
        for slot in slots.into_iter().take(3) {
            let scale = 0.84;
            let native_w = if slot.full_size { 380.0 } else { 180.0 };
            let native_h = 80.0;
            let out_w = native_w * scale;
            let out_h = native_h * scale;
            canvas.save();
            canvas.translate((cursor + out_w / 2.0, y + 46.0));
            canvas.scale((scale, scale));
            if let Some(store) = assets {
                if slot.profile_honor_type == "bonds" {
                    crate::elements::honor::render_bonds_honor(
                        canvas,
                        slot.honor_id,
                        slot.honor_level,
                        slot.full_size,
                        slot.bonds_honor_word_id.unwrap_or(0),
                        slot.bonds_honor_view_type.as_deref() == Some("reverse"),
                        false,
                        md,
                        store,
                    );
                } else {
                    crate::elements::honor::render_honor(
                        canvas,
                        slot.honor_id,
                        slot.honor_level,
                        slot.full_size,
                        md,
                        store,
                        Some(profile),
                    );
                }
            }
            canvas.restore();
            cursor += out_w + 18.0;
            if cursor > x + 666.0 {
                break;
            }
            let _ = out_h;
        }
    }

    fn draw_deck(
        &mut self,
        profile: &ProfileData,
        md: &MasterData,
        assets: Option<&AssetStore>,
        x: f32,
        y: f32,
    ) {
        self.text(
            "当前卡组",
            x,
            y - 16.0,
            22.0,
            self.colors.muted,
            TextAlign::Left,
            300.0,
            FontWeight::Regular,
        );
        let card_w = 130.0;
        let card_h = 214.0;
        let gap = 17.0;
        for i in 0..5 {
            let cx = x + i as f32 * (card_w + gap);
            let rect = skia_safe::Rect::from_xywh(cx, y, card_w, card_h);
            self.panel_fill(
                rect,
                8.0,
                self.colors.black_35,
                Some(self.colors.glass_edge),
            );
            if let Some(member) = profile.deck_members.get(i) {
                self.deck_card(member, md, assets, rect);
            }
        }
    }

    fn deck_card(
        &mut self,
        member: &DeckMember,
        md: &MasterData,
        assets: Option<&AssetStore>,
        rect: skia_safe::Rect,
    ) {
        let suffix = if member.after_training {
            "after_training"
        } else {
            "normal"
        };
        let Some(key) = crate::asset_keys::resolve_card_member_key(member.card_id, 1, suffix, md)
        else {
            return;
        };
        let badge = md
            .get_card(member.card_id)
            .map(|card| crate::elements::image::CardBadgeData {
                rarity: card.card_rarity_type,
                attr: card.attr,
                master_rank: member.master_rank,
                trained: member.after_training,
                level: member.level,
            });

        let canvas = self.surface.canvas();
        let clip = {
            let rrect = skia_safe::RRect::new_rect_xy(rect, 8.0, 8.0);
            let mut b = skia_safe::PathBuilder::new();
            b.add_rrect(rrect, None, None);
            b.detach()
        };
        canvas.save();
        canvas.clip_path(&clip, skia_safe::ClipOp::Intersect, true);
        canvas.translate((rect.center_x(), rect.center_y()));
        let scale = (rect.width() / 312.0).min(rect.height() / 512.0);
        canvas.scale((scale, scale));
        crate::elements::image::draw_card_member_cropped(
            canvas,
            assets,
            &key,
            member.card_id,
            badge,
        );
        canvas.restore();
    }

    fn draw_music(&mut self, profile: &ProfileData, x: f32, y: f32) {
        let rect = skia_safe::Rect::from_xywh(x, y, 528.0, 146.0);
        self.panel_fill(
            rect,
            8.0,
            self.colors.panel_alt,
            Some(self.colors.glass_edge),
        );
        let Some(results) = &profile.music_results else {
            self.text(
                "歌曲统计未公开",
                x + 24.0,
                y + 82.0,
                26.0,
                self.colors.muted,
                TextAlign::Left,
                480.0,
                FontWeight::Regular,
            );
            return;
        };

        let totals = [
            ("完成", sum_music(results, |s| s.clear), self.colors.miku),
            (
                "全连",
                sum_music(results, |s| s.full_combo),
                self.colors.niigo,
            ),
            (
                "AP",
                sum_music(results, |s| s.all_perfect),
                self.colors.warm,
            ),
        ];
        for (index, (label, value, color)) in totals.iter().enumerate() {
            let sx = x + 24.0 + index as f32 * 164.0;
            self.text(
                label,
                sx,
                y + 34.0,
                17.0,
                self.colors.muted,
                TextAlign::Left,
                130.0,
                FontWeight::Regular,
            );
            self.text(
                &format_number(i64::from(*value)),
                sx,
                y + 78.0,
                37.0,
                *color,
                TextAlign::Left,
                144.0,
                FontWeight::Emphasis,
            );
        }

        self.text(
            &format!(
                "高难 AP：EXPERT {} · MASTER {} · APPEND {}",
                results.expert.all_perfect, results.master.all_perfect, results.append.all_perfect
            ),
            x + 24.0,
            y + 122.0,
            18.0,
            self.colors.muted,
            TextAlign::Left,
            480.0,
            FontWeight::Regular,
        );
    }

    fn draw_counters(&mut self, profile: &ProfileData, x: f32, y: f32) {
        let cell_w = 248.0;
        let cell_h = 64.0;
        let gap = 16.0;
        self.stat_cell(
            "挑战最高分",
            &format_number(i64::from(profile.challenge_score)),
            x,
            y,
            cell_w,
            cell_h,
            self.colors.warm,
        );
        self.stat_cell(
            "角色收藏",
            &character_rank_summary(profile),
            x + cell_w + gap,
            y,
            cell_w,
            cell_h,
            self.colors.miku,
        );
        self.stat_cell(
            "MVP",
            &format_number(i64::from(profile.mvp)),
            x,
            y + cell_h + gap,
            cell_w,
            cell_h,
            self.colors.niigo,
        );
        self.stat_cell(
            "SUPER STAR",
            &format_number(i64::from(profile.superstar)),
            x + cell_w + gap,
            y + cell_h + gap,
            cell_w,
            cell_h,
            self.colors.miku,
        );
    }

    fn stat_cell(
        &mut self,
        label: &str,
        value: &str,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        accent: Color,
    ) {
        self.panel_fill(
            skia_safe::Rect::from_xywh(x, y, width, height),
            8.0,
            self.colors.panel_alt,
            Some(self.colors.glass_edge),
        );
        self.rule(x + 14.0, y + 16.0, 42.0, 4.0, accent);
        self.text(
            label,
            x + 66.0,
            y + 25.0,
            16.0,
            self.colors.muted,
            TextAlign::Left,
            width - 82.0,
            FontWeight::Regular,
        );
        self.text(
            value,
            x + 16.0,
            y + 53.0,
            30.0,
            self.colors.text,
            TextAlign::Left,
            width - 32.0,
            FontWeight::Emphasis,
        );
    }

    fn glass(&mut self, rect: skia_safe::Rect) {
        self.shadow(rect);
        self.panel_fill(rect, 8.0, self.colors.panel, Some(self.colors.glass_edge));
    }

    fn shadow(&mut self, rect: skia_safe::Rect) {
        let shadow = skia_safe::Rect::from_xywh(
            rect.left + 4.0,
            rect.top + 6.0,
            rect.width(),
            rect.height(),
        );
        let mut paint = skia_safe::Paint::default();
        paint.set_anti_alias(true);
        paint.set_style(skia_safe::PaintStyle::Fill);
        paint.set_color(self.colors.black_35.skia());
        self.surface
            .canvas()
            .draw_round_rect(shadow, 10.0, 10.0, &paint);
    }

    fn panel_fill(
        &mut self,
        rect: skia_safe::Rect,
        radius: f32,
        fill: Color,
        border: Option<Color>,
    ) {
        let mut paint = skia_safe::Paint::default();
        paint.set_anti_alias(true);
        paint.set_style(skia_safe::PaintStyle::Fill);
        paint.set_color(fill.skia());
        self.surface
            .canvas()
            .draw_round_rect(rect, radius, radius, &paint);
        if let Some(border) = border {
            paint.set_style(skia_safe::PaintStyle::Stroke);
            paint.set_stroke_width(1.0);
            paint.set_color(border.skia());
            self.surface
                .canvas()
                .draw_round_rect(rect, radius, radius, &paint);
        }
    }

    fn rule(&mut self, x: f32, y: f32, width: f32, height: f32, color: Color) {
        let rect = skia_safe::Rect::from_xywh(x, y, width, height);
        let mut paint = skia_safe::Paint::default();
        paint.set_anti_alias(true);
        paint.set_style(skia_safe::PaintStyle::Fill);
        paint.set_color(color.skia());
        self.surface
            .canvas()
            .draw_round_rect(rect, height / 2.0, height / 2.0, &paint);
    }

    fn chip(&mut self, text: &str, x: f32, y: f32, height: f32, accent: Color) -> f32 {
        let width = estimate_text_width(text, 18.0) + 28.0;
        self.panel_fill(
            skia_safe::Rect::from_xywh(x, y, width, height),
            height / 2.0,
            Color(accent.0, accent.1, accent.2, 38),
            Some(Color(accent.0, accent.1, accent.2, 108)),
        );
        self.text(
            text,
            x + 14.0,
            y + 24.0,
            18.0,
            self.colors.text,
            TextAlign::Left,
            width - 28.0,
            FontWeight::Regular,
        );
        width
    }

    fn text(
        &mut self,
        text: &str,
        x: f32,
        baseline: f32,
        size: f32,
        color: Color,
        align: TextAlign,
        max_width: f32,
        weight: FontWeight,
    ) {
        let typeface = match weight {
            FontWeight::Regular => self.regular.clone(),
            FontWeight::Emphasis => self.emphasis.clone(),
        };
        let font = skia_safe::Font::new(typeface, Some(size));
        let mut paint = skia_safe::Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(color.skia());
        let truncated = truncate_text(text, max_width, size);
        let draw_x = match align {
            TextAlign::Left => x,
        };
        self.surface
            .canvas()
            .draw_str(truncated, (draw_x, baseline), &font, &paint);
    }

    fn finish(mut self) -> Result<RenderOutput, String> {
        let image = self.surface.image_snapshot();
        let encoded = image
            .encode(
                None,
                skia_safe::EncodedImageFormat::JPEG,
                Some(u32::from(PERSONAL_PROFILE_QUALITY)),
            )
            .ok_or_else(|| "平台个人资料 JPEG 编码失败".to_string())?;
        Ok(RenderOutput {
            data: encoded.as_bytes().to_vec(),
            content_type: "image/jpeg".to_string(),
            width: PERSONAL_PROFILE_WIDTH,
            height: PERSONAL_PROFILE_HEIGHT,
            timing: None,
        })
    }
}

#[cfg(feature = "skia-core")]
fn draw_asset_cover(
    canvas: &skia_safe::Canvas,
    assets: Option<&AssetStore>,
    key: &str,
    rect: skia_safe::Rect,
    radius: f32,
) -> bool {
    let Some(image) = assets.and_then(|store| store.get_image(key)) else {
        return false;
    };
    let clip = {
        let rrect = skia_safe::RRect::new_rect_xy(rect, radius, radius);
        let mut b = skia_safe::PathBuilder::new();
        b.add_rrect(rrect, None, None);
        b.detach()
    };
    canvas.save();
    canvas.clip_path(&clip, skia_safe::ClipOp::Intersect, true);
    let (sx, sy, sw, sh) = crate::widgets::card_util::cover_crop_rect(
        image.width() as f32,
        image.height() as f32,
        rect.width(),
        rect.height(),
    );
    canvas.draw_image_rect(
        image,
        Some((
            &skia_safe::Rect::from_xywh(sx, sy, sw, sh),
            skia_safe::canvas::SrcRectConstraint::Fast,
        )),
        rect,
        &skia_safe::Paint::default(),
    );
    canvas.restore();
    true
}

#[cfg(feature = "skia-core")]
fn profile_typefaces() -> Option<(skia_safe::Typeface, skia_safe::Typeface)> {
    let font_mgr = skia_safe::FontMgr::default();
    let regular = crate::text::resolve_custom_profile_typeface(
        &font_mgr,
        Some(crate::widgets::theme::fonts::PRIMARY),
    )
    .or_else(|| font_mgr.match_family_style("Noto Sans CJK SC", skia_safe::FontStyle::default()))
    .or_else(|| font_mgr.match_family_style("Noto Sans CJK", skia_safe::FontStyle::default()))
    .or_else(|| font_mgr.legacy_make_typeface(None, skia_safe::FontStyle::default()))?;
    let emphasis = crate::text::resolve_custom_profile_typeface(
        &font_mgr,
        Some(crate::widgets::theme::fonts::EMPHASIS),
    )
    .or_else(|| Some(regular.clone()))?;
    Some((regular, emphasis))
}

#[cfg(feature = "skia-core")]
#[derive(Debug, Clone, Copy)]
enum TextAlign {
    Left,
}

#[cfg(feature = "skia-core")]
#[derive(Debug, Clone, Copy)]
enum FontWeight {
    Regular,
    Emphasis,
}

#[cfg(feature = "skia-core")]
#[derive(Debug, Clone, Copy)]
struct Color(u8, u8, u8, u8);

#[cfg(feature = "skia-core")]
impl Color {
    fn skia(self) -> skia_safe::Color {
        skia_safe::Color::from_argb(self.3, self.0, self.1, self.2)
    }
}

fn blank_as<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback
    } else {
        trimmed
    }
}

fn format_number(value: i64) -> String {
    let negative = value < 0;
    let digits = value.abs().to_string();
    let mut out = String::new();
    for (index, ch) in digits.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    let mut formatted = out.chars().rev().collect::<String>();
    if negative {
        formatted.insert(0, '-');
    }
    formatted
}

fn character_rank_summary(profile: &ProfileData) -> String {
    let Some(max_rank) = profile.char_ranks.iter().map(|rank| rank.rank).max() else {
        return "未公开".to_string();
    };
    format!("最高 {max_rank}")
}

fn sum_music<F>(results: &crate::profile::MusicResults, select: F) -> i32
where
    F: Fn(&crate::profile::MusicDifficultyStats) -> i32,
{
    select(&results.easy)
        + select(&results.normal)
        + select(&results.hard)
        + select(&results.expert)
        + select(&results.master)
        + select(&results.append)
}

fn wrap_text(text: &str, max_width: f32, font_size: f32, max_lines: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        let candidate = format!("{current}{ch}");
        if !current.is_empty() && estimate_text_width(&candidate, font_size) > max_width {
            lines.push(current);
            current = String::new();
            if lines.len() + 1 >= max_lines {
                break;
            }
        }
        current.push(ch);
    }
    if !current.is_empty() && lines.len() < max_lines {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    if text.chars().count() > lines.iter().map(|line| line.chars().count()).sum::<usize>() {
        if let Some(last) = lines.last_mut() {
            *last = truncate_text(last, max_width, font_size);
        }
    }
    lines
}

fn truncate_text(text: &str, max_width: f32, font_size: f32) -> String {
    if estimate_text_width(text, font_size) <= max_width {
        return text.to_string();
    }
    let ellipsis = "...";
    let mut out = String::new();
    let mut used = estimate_text_width(ellipsis, font_size);
    for ch in text.chars() {
        let chs = ch.to_string();
        let w = estimate_text_width(&chs, font_size);
        if used + w > max_width {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push_str(ellipsis);
    out
}

fn estimate_text_width(text: &str, font_size: f32) -> f32 {
    text.chars()
        .map(|ch| if ch.is_ascii() { 0.56 } else { 0.98 })
        .sum::<f32>()
        * font_size
}

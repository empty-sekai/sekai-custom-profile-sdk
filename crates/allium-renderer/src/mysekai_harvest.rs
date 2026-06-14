//! MySekai 采集地图渲染。

#![cfg_attr(not(feature = "skia-core"), allow(dead_code))]

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::assets::AssetStore;
use crate::error::RenderError;
use crate::traits::RenderOutput;

const ORIGINAL_WIDTH: f32 = 1920.0;
const ORIGINAL_HEIGHT: f32 = 1080.0;
const GLOBAL_SCALE: f32 = 0.7;
const FIXTURE_ICON_SCALE: f32 = 0.4;
const DROP_LIST_SCALE: f32 = 0.7;
const DROP_QUANTITY_FONT_BASE: f32 = 11.0;
const RARE_SUMMARY_QUANTITY_FONT: f32 = 14.0;
const LABEL_SEARCH_LIMIT: usize = 2500;
const LABEL_EARLY_STOP: usize = 100;
const DEFAULT_QUALITY: u8 = 70;

/// MySekai 采集地图渲染输入。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarvestMapRenderInput {
    /// MySekai site ID。
    pub site_id: i64,
    /// 采集点列表。
    pub fixtures: Vec<HarvestFixture>,
    /// 掉落物列表。
    pub drops: Vec<HarvestDrop>,
    /// 稀有资源 ID 列表。
    pub rare_resource_ids: Vec<i64>,
    /// JPEG 输出质量。
    pub quality: u8,
}

/// MySekai 采集点渲染输入。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarvestFixture {
    /// 采集点 fixture ID。
    pub fixture_id: i64,
    /// 游戏坐标 X。
    pub position_x: f32,
    /// 游戏坐标 Z。
    pub position_z: f32,
    /// 采集点状态。
    pub status: String,
}

/// MySekai 掉落物渲染输入。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarvestDrop {
    /// 资源类型。
    pub resource_type: String,
    /// 资源 ID。
    pub resource_id: i64,
    /// 游戏坐标 X。
    pub position_x: f32,
    /// 游戏坐标 Z。
    pub position_z: f32,
    /// 掉落数量。
    pub quantity: i64,
    /// 掉落状态。
    pub status: String,
    /// 已解析的静态素材 key。
    pub asset_key: Option<String>,
}

/// 将 MySekai 采集地图输入渲染为 JPEG。
pub fn render_harvest_map(
    input: &HarvestMapRenderInput,
    assets: &AssetStore,
) -> Result<RenderOutput, RenderError> {
    render_harvest_map_impl(input, assets)
}

#[derive(Debug, Clone, Copy)]
struct MapConfig {
    grid_size: f32,
    offset_x: f32,
    offset_z: f32,
    dir_x: f32,
    dir_z: f32,
    rev_xz: bool,
    left_crop: f32,
    right_crop: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct PositionKey {
    x: i32,
    z: i32,
}

#[derive(Debug, Clone)]
#[cfg_attr(not(feature = "skia-core"), allow(dead_code))]
struct AggregatedDrop {
    resource_id: i64,
    quantity: i64,
    asset_key: Option<String>,
}

#[derive(Debug, Clone)]
#[cfg_attr(not(feature = "skia-core"), allow(dead_code))]
struct RenderItem {
    pos: Point,
    fixture_id: i64,
    drops: Vec<AggregatedDrop>,
    is_rare: bool,
    color: Rgb,
    icon_rect: Rect,
    label_rect: Option<Rect>,
}

#[derive(Debug, Clone, Copy, Default)]
struct Point {
    x: f32,
    y: f32,
}

#[derive(Debug, Clone, Copy, Default)]
struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(not(feature = "skia-core"), allow(dead_code))]
struct Rgb {
    r: u8,
    g: u8,
    b: u8,
}

fn render_harvest_map_impl(
    input: &HarvestMapRenderInput,
    assets: &AssetStore,
) -> Result<RenderOutput, RenderError> {
    #[cfg(feature = "skia-core")]
    {
        return render_harvest_map_skia(input, assets);
    }

    #[cfg(not(feature = "skia-core"))]
    {
        let _ = assets;
        let _quality = jpeg_quality(input.quality);
        let config = map_config(input.site_id)?;
        let (width, height) = output_size(&config);
        Ok(RenderOutput {
            data: placeholder_jpeg().to_vec(),
            content_type: "image/jpeg".to_string(),
            width,
            height,
            timing: None,
        })
    }
}

fn map_config(site_id: i64) -> Result<MapConfig, RenderError> {
    match site_id {
        5 => Ok(MapConfig {
            grid_size: 33.333,
            offset_x: 0.0,
            offset_z: -60.0,
            dir_x: -1.0,
            dir_z: -1.0,
            rev_xz: true,
            left_crop: 200.0,
            right_crop: 200.0,
        }),
        6 => Ok(MapConfig {
            grid_size: 20.513,
            offset_x: 0.0,
            offset_z: 80.0,
            dir_x: 1.0,
            dir_z: -1.0,
            rev_xz: false,
            left_crop: 200.0,
            right_crop: 200.0,
        }),
        7 => Ok(MapConfig {
            grid_size: 24.806,
            offset_x: -62.015,
            offset_z: 20.672,
            dir_x: -1.0,
            dir_z: -1.0,
            rev_xz: true,
            left_crop: 220.0,
            right_crop: 180.0,
        }),
        8 => Ok(MapConfig {
            grid_size: 21.333,
            offset_x: 0.0,
            offset_z: -130.0,
            dir_x: 1.0,
            dir_z: -1.0,
            rev_xz: false,
            left_crop: 100.0,
            right_crop: 300.0,
        }),
        _ => Err(RenderError::Config(format!(
            "不支持的 MySekai harvest site: {site_id}"
        ))),
    }
}

fn output_size(config: &MapConfig) -> (u32, u32) {
    let width = (ORIGINAL_WIDTH * GLOBAL_SCALE - config.left_crop - config.right_crop).round();
    let height = (ORIGINAL_HEIGHT * GLOBAL_SCALE).round();
    (width.max(1.0) as u32, height.max(1.0) as u32)
}

fn canvas_size() -> (f32, f32) {
    (
        ORIGINAL_WIDTH * GLOBAL_SCALE,
        ORIGINAL_HEIGHT * GLOBAL_SCALE,
    )
}

fn game_pos_to_pixel(config: &MapConfig, position_x: f32, position_z: f32) -> Point {
    let (mut game_x, mut game_z) = (position_x, position_z);
    if config.rev_xz {
        (game_x, game_z) = (game_z, game_x);
    }
    let (canvas_w, canvas_h) = canvas_size();
    Point {
        x: canvas_w / 2.0
            + game_x * (config.grid_size * GLOBAL_SCALE) * config.dir_x
            + config.offset_x * GLOBAL_SCALE,
        y: canvas_h / 2.0
            + game_z * (config.grid_size * GLOBAL_SCALE) * config.dir_z
            + config.offset_z * GLOBAL_SCALE,
    }
}

fn position_key(x: f32, z: f32) -> PositionKey {
    PositionKey {
        x: (x * 1000.0).round() as i32,
        z: (z * 1000.0).round() as i32,
    }
}

fn aggregate_drops(drops: &[HarvestDrop]) -> BTreeMap<PositionKey, Vec<AggregatedDrop>> {
    let mut by_pos: BTreeMap<PositionKey, BTreeMap<(String, i64, Option<String>), i64>> =
        BTreeMap::new();
    for drop in drops {
        if drop.status != "before_drop" || drop.resource_type == "mysekai_music_record" {
            continue;
        }
        let key = position_key(drop.position_x, drop.position_z);
        let item_key = (
            drop.resource_type.clone(),
            drop.resource_id,
            drop.asset_key.clone(),
        );
        let quantity = drop.quantity.max(0);
        *by_pos.entry(key).or_default().entry(item_key).or_default() += quantity;
    }

    by_pos
        .into_iter()
        .map(|(position, drops)| {
            let rendered = drops
                .into_iter()
                .filter_map(|((_resource_type, resource_id, asset_key), quantity)| {
                    (quantity > 0).then_some(AggregatedDrop {
                        resource_id,
                        quantity,
                        asset_key,
                    })
                })
                .collect::<Vec<_>>();
            (position, rendered)
        })
        .collect()
}

fn build_render_items(input: &HarvestMapRenderInput) -> Result<Vec<RenderItem>, RenderError> {
    let config = map_config(input.site_id)?;
    let rare_ids = input
        .rare_resource_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let drops_by_pos = aggregate_drops(&input.drops);
    let mut items = Vec::new();

    for fixture in &input.fixtures {
        if fixture.status != "spawned" {
            continue;
        }
        let key = position_key(fixture.position_x, fixture.position_z);
        let Some(drops) = drops_by_pos.get(&key).filter(|drops| !drops.is_empty()) else {
            continue;
        };
        let pos = game_pos_to_pixel(&config, fixture.position_x, fixture.position_z);
        let icon_size = 50.0 * FIXTURE_ICON_SCALE;
        items.push(RenderItem {
            pos,
            fixture_id: fixture.fixture_id,
            drops: drops.clone(),
            is_rare: drops
                .iter()
                .any(|drop| rare_ids.contains(&drop.resource_id)),
            color: Rgb {
                r: 200,
                g: 200,
                b: 200,
            },
            icon_rect: Rect {
                x: pos.x - icon_size / 2.0,
                y: pos.y - icon_size / 2.0,
                w: icon_size,
                h: icon_size,
            },
            label_rect: None,
        });
    }

    items.sort_by(|left, right| {
        left.pos
            .y
            .total_cmp(&right.pos.y)
            .then(left.pos.x.total_cmp(&right.pos.x))
    });
    assign_colors_and_labels(&mut items);
    Ok(items)
}

fn assign_colors_and_labels(items: &mut [RenderItem]) {
    let mut occupied = Vec::new();
    for (index, item) in items.iter_mut().enumerate() {
        item.color = palette_color(index);
        occupied.push(item.icon_rect);
    }

    let (canvas_w, canvas_h) = canvas_size();
    for item in items.iter_mut() {
        let rect = label_size(&item.drops);
        let mut best_pos = None;
        let mut min_dist = f32::INFINITY;
        for j in 0..LABEL_SEARCH_LIMIT {
            let angle = j as f32 * 0.2;
            let radius = 30.0 + j as f32 * 0.15;
            let x_c = item.pos.x + radius * angle.cos() - rect.w / 2.0;
            let y_c = item.pos.y + radius * angle.sin() - rect.h / 2.0;
            let x = x_c.clamp(0.0, (canvas_w - rect.w).max(0.0));
            let y = y_c.clamp(0.0, (canvas_h - rect.h).max(0.0));
            let current = Rect {
                x,
                y,
                w: rect.w,
                h: rect.h,
            };
            if occupied.iter().all(|rect| !rects_intersect(&current, rect)) {
                let center = Point {
                    x: x + current.w / 2.0,
                    y: y + current.h / 2.0,
                };
                let dist = distance(item.pos, center);
                if dist < min_dist {
                    min_dist = dist;
                    best_pos = Some(current);
                }
            }
            if j > LABEL_EARLY_STOP && best_pos.is_some() {
                break;
            }
        }
        if let Some(rect) = best_pos {
            item.label_rect = Some(rect);
            occupied.push(rect);
        }
    }
}

fn label_size(drops: &[AggregatedDrop]) -> Rect {
    let padding = 5.0 * DROP_LIST_SCALE;
    let icon = 25.0 * DROP_LIST_SCALE;
    let font = DROP_QUANTITY_FONT_BASE * DROP_LIST_SCALE;
    let max_text = drops
        .iter()
        .map(|drop| estimate_text_width(&format!("x{}", drop.quantity), font))
        .fold(0.0_f32, f32::max);
    Rect {
        x: 0.0,
        y: 0.0,
        w: padding * 3.0 + icon + max_text,
        h: drops.len() as f32 * (icon + padding),
    }
}

fn rects_intersect(left: &Rect, right: &Rect) -> bool {
    !(left.x + left.w < right.x
        || right.x + right.w < left.x
        || left.y + left.h < right.y
        || right.y + right.h < left.y)
}

fn distance(left: Point, right: Point) -> f32 {
    let dx = left.x - right.x;
    let dy = left.y - right.y;
    (dx * dx + dy * dy).sqrt()
}

fn palette_color(index: usize) -> Rgb {
    const COLORS: [Rgb; 8] = [
        Rgb {
            r: 255,
            g: 107,
            b: 107,
        },
        Rgb {
            r: 104,
            g: 209,
            b: 243,
        },
        Rgb {
            r: 255,
            g: 184,
            b: 108,
        },
        Rgb {
            r: 120,
            g: 240,
            b: 156,
        },
        Rgb {
            r: 239,
            g: 132,
            b: 245,
        },
        Rgb {
            r: 249,
            g: 217,
            b: 137,
        },
        Rgb {
            r: 189,
            g: 178,
            b: 255,
        },
        Rgb {
            r: 255,
            g: 192,
            b: 203,
        },
    ];
    COLORS[index % COLORS.len()]
}

#[cfg_attr(not(feature = "skia-core"), allow(dead_code))]
fn fixture_inner_color(fixture_id: i64) -> Rgb {
    match fixture_id {
        111 | 112 => Rgb {
            r: 255,
            g: 215,
            b: 0,
        },
        1001..=1004 => Rgb {
            r: 139,
            g: 69,
            b: 19,
        },
        2001..=2006 => Rgb {
            r: 112,
            g: 128,
            b: 144,
        },
        3001 => Rgb {
            r: 160,
            g: 82,
            b: 45,
        },
        4001..=4017 => Rgb {
            r: 34,
            g: 139,
            b: 34,
        },
        5001..=5004 | 5101..=5104 => Rgb {
            r: 245,
            g: 245,
            b: 245,
        },
        6001 => Rgb {
            r: 205,
            g: 133,
            b: 63,
        },
        7001 => Rgb {
            r: 65,
            g: 105,
            b: 225,
        },
        _ => Rgb {
            r: 200,
            g: 200,
            b: 200,
        },
    }
}

fn estimate_text_width(text: &str, font_size: f32) -> f32 {
    text.chars().count() as f32 * font_size * 0.58
}

#[cfg(feature = "skia-core")]
fn render_harvest_map_skia(
    input: &HarvestMapRenderInput,
    assets: &AssetStore,
) -> Result<RenderOutput, RenderError> {
    use skia_safe::{
        canvas::SrcRectConstraint, surfaces, EncodedImageFormat, Paint, Rect as SkRect,
    };

    let config = map_config(input.site_id)?;
    let (canvas_w, canvas_h) = canvas_size();
    let mut surface = surfaces::raster_n32_premul((canvas_w as i32, canvas_h as i32))
        .ok_or_else(|| RenderError::Render("创建 MySekai 地图 Surface 失败".to_string()))?;
    let canvas = surface.canvas();
    let mut paint = Paint::default();
    paint.set_anti_alias(true);

    let background_key = format!("mysekai/map_backgrounds/{}", input.site_id);
    let background = assets
        .get_image(&background_key)
        .ok_or(RenderError::AssetNotFound {
            key: background_key,
        })?;
    canvas.draw_image_rect(
        background,
        None,
        SkRect::from_xywh(0.0, 0.0, canvas_w, canvas_h),
        &paint,
    );

    let items = build_render_items(input)?;
    draw_connectors(canvas, &items);
    draw_fixture_icons(canvas, &items);
    draw_labels(canvas, &items, input, assets);

    let image = surface.image_snapshot();
    let (out_w, out_h) = output_size(&config);
    let mut cropped = surfaces::raster_n32_premul((out_w as i32, out_h as i32))
        .ok_or_else(|| RenderError::Render("创建 MySekai 裁剪 Surface 失败".to_string()))?;
    let crop_canvas = cropped.canvas();
    crop_canvas.draw_image_rect(
        image,
        Some((
            &SkRect::from_xywh(config.left_crop, 0.0, out_w as f32, out_h as f32),
            SrcRectConstraint::Fast,
        )),
        SkRect::from_xywh(0.0, 0.0, out_w as f32, out_h as f32),
        &paint,
    );
    draw_rare_summary(crop_canvas, &items, input, assets, out_w as f32);

    let encoded = cropped
        .image_snapshot()
        .encode(
            None,
            EncodedImageFormat::JPEG,
            Some(u32::from(jpeg_quality(input.quality))),
        )
        .ok_or_else(|| RenderError::Encode("MySekai 地图 JPEG 编码失败".to_string()))?;

    Ok(RenderOutput {
        data: encoded.as_bytes().to_vec(),
        content_type: "image/jpeg".to_string(),
        width: out_w,
        height: out_h,
        timing: None,
    })
}

fn jpeg_quality(quality: u8) -> u8 {
    if quality == 0 {
        DEFAULT_QUALITY
    } else {
        quality.clamp(1, 100)
    }
}

#[cfg(feature = "skia-core")]
fn draw_connectors(canvas: &skia_safe::Canvas, items: &[RenderItem]) {
    use skia_safe::{Paint, PaintStyle, PathBuilder};

    for item in items {
        let Some(label) = item.label_rect else {
            continue;
        };
        let end = nearest_label_edge(item.pos, label);
        let dx = end.x - item.pos.x;
        let dy = end.y - item.pos.y;
        let bend = if dx > 0.0 { -0.25 } else { 0.25 };
        let ctrl = Point {
            x: (item.pos.x + end.x) / 2.0 - dy * bend,
            y: (item.pos.y + end.y) / 2.0 + dx * bend,
        };
        let path = {
            let mut b = PathBuilder::new();
            b.move_to((item.pos.x, item.pos.y));
            b.quad_to((ctrl.x, ctrl.y), (end.x, end.y));
            b.detach()
        };
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_style(PaintStyle::Stroke);
        paint.set_stroke_width(2.0);
        paint.set_color(skia_color(item.color, 255));
        canvas.draw_path(&path, &paint);
        paint.set_style(PaintStyle::Fill);
        canvas.draw_circle((end.x, end.y), 2.0, &paint);
    }
}

#[cfg(feature = "skia-core")]
fn draw_fixture_icons(canvas: &skia_safe::Canvas, items: &[RenderItem]) {
    use skia_safe::{Paint, PaintStyle};

    for item in items {
        if item.is_rare {
            for index in 0..5 {
                let alpha = 150_i32 - index * 30;
                if alpha <= 0 {
                    continue;
                }
                let radius = item.icon_rect.w * 0.8 - index as f32 * 2.0;
                let mut paint = Paint::default();
                paint.set_anti_alias(true);
                paint.set_style(PaintStyle::Fill);
                paint.set_color(skia_color(item.color, alpha as u8));
                canvas.draw_circle((item.pos.x, item.pos.y), radius.max(1.0), &paint);
            }
        }

        let mut outer = Paint::default();
        outer.set_anti_alias(true);
        outer.set_style(PaintStyle::Fill);
        outer.set_color(skia_color(item.color, 180));
        canvas.draw_oval(skia_rect(item.icon_rect), &outer);

        let inner_rect = Rect {
            x: item.icon_rect.x + 3.0,
            y: item.icon_rect.y + 3.0,
            w: (item.icon_rect.w - 6.0).max(1.0),
            h: (item.icon_rect.h - 6.0).max(1.0),
        };
        let mut inner = Paint::default();
        inner.set_anti_alias(true);
        inner.set_style(PaintStyle::Fill);
        inner.set_color(skia_color(fixture_inner_color(item.fixture_id), 255));
        canvas.draw_oval(skia_rect(inner_rect), &inner);
    }
}

#[cfg(feature = "skia-core")]
fn draw_labels(
    canvas: &skia_safe::Canvas,
    items: &[RenderItem],
    input: &HarvestMapRenderInput,
    assets: &AssetStore,
) {
    use skia_safe::{Paint, PaintStyle, Rect as SkRect};

    let rare_ids = input
        .rare_resource_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let padding = 5.0 * DROP_LIST_SCALE;
    let icon = 25.0 * DROP_LIST_SCALE;
    let font_size = DROP_QUANTITY_FONT_BASE * DROP_LIST_SCALE;
    let item_height = icon + padding;
    let Some(font) = harvest_text_font(font_size) else {
        tracing::warn!("MySekai harvest 字体不可用，跳过掉落数量文本");
        return;
    };

    for item in items {
        let Some(rect) = item.label_rect else {
            continue;
        };
        let mut fill = Paint::default();
        fill.set_anti_alias(true);
        fill.set_style(PaintStyle::Fill);
        fill.set_color(skia_safe::Color::from_argb(220, 34, 34, 34));
        canvas.draw_round_rect(skia_rect(rect), 5.0, 5.0, &fill);

        let mut stroke = Paint::default();
        stroke.set_anti_alias(true);
        stroke.set_style(PaintStyle::Stroke);
        stroke.set_stroke_width(2.0);
        stroke.set_color(skia_color(item.color, 255));
        canvas.draw_round_rect(skia_rect(rect), 5.0, 5.0, &stroke);

        for (index, drop) in item.drops.iter().enumerate() {
            let y = rect.y + index as f32 * item_height + (item_height - icon) / 2.0;
            let dst = SkRect::from_xywh(rect.x + padding, y, icon, icon);
            draw_drop_icon(canvas, assets, drop, dst);
            let mut text = Paint::default();
            text.set_anti_alias(true);
            text.set_color(if rare_ids.contains(&drop.resource_id) {
                skia_safe::Color::from_argb(255, 255, 215, 0)
            } else {
                skia_safe::Color::WHITE
            });
            canvas.draw_str(
                format!("x{}", drop.quantity),
                (
                    rect.x + padding * 2.0 + icon,
                    rect.y + index as f32 * item_height + item_height / 2.0 + font_size * 0.35,
                ),
                &font,
                &text,
            );
        }
    }
}

#[cfg(feature = "skia-core")]
fn draw_drop_icon(
    canvas: &skia_safe::Canvas,
    assets: &AssetStore,
    drop: &AggregatedDrop,
    dst: skia_safe::Rect,
) {
    use skia_safe::{Paint, PaintStyle};

    if let Some(key) = drop.asset_key.as_deref() {
        if let Some(image) = assets.get_image(key) {
            let paint = Paint::default();
            canvas.draw_image_rect(image, None, dst, &paint);
            return;
        }
        tracing::warn!(
            asset_key = key,
            resource_id = drop.resource_id,
            "MySekai 掉落素材缺失，使用占位图标"
        );
    }

    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_style(PaintStyle::Fill);
    paint.set_color(skia_safe::Color::from_argb(230, 253, 233, 16));
    canvas.draw_oval(dst, &paint);
    paint.set_style(PaintStyle::Stroke);
    paint.set_stroke_width(2.0);
    paint.set_color(skia_safe::Color::BLACK);
    canvas.draw_oval(dst, &paint);
}

#[cfg(feature = "skia-core")]
fn draw_rare_summary(
    canvas: &skia_safe::Canvas,
    items: &[RenderItem],
    input: &HarvestMapRenderInput,
    assets: &AssetStore,
    output_width: f32,
) {
    use skia_safe::{Paint, PaintStyle, Rect as SkRect};

    let rare_ids = input
        .rare_resource_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut summary: BTreeMap<i64, (i64, Option<String>)> = BTreeMap::new();
    for item in items {
        for drop in &item.drops {
            if rare_ids.contains(&drop.resource_id) {
                let entry = summary
                    .entry(drop.resource_id)
                    .or_insert((0, drop.asset_key.clone()));
                entry.0 += drop.quantity;
                if entry.1.is_none() {
                    entry.1 = drop.asset_key.clone();
                }
            }
        }
    }
    if summary.is_empty() {
        return;
    }

    let padding = 8.0;
    let icon = 35.0;
    let font_size = RARE_SUMMARY_QUANTITY_FONT;
    let item_width = icon + padding;
    let width = summary.len() as f32 * item_width + padding;
    let height = padding * 2.0 + icon + font_size * 1.2;
    let x = output_width - width - 15.0;
    let y = 15.0;
    let rect = SkRect::from_xywh(x, y, width, height);

    let mut fill = Paint::default();
    fill.set_anti_alias(true);
    fill.set_style(PaintStyle::Fill);
    fill.set_color(skia_safe::Color::from_argb(220, 34, 34, 34));
    canvas.draw_round_rect(rect, 8.0, 8.0, &fill);
    let mut stroke = Paint::default();
    stroke.set_anti_alias(true);
    stroke.set_style(PaintStyle::Stroke);
    stroke.set_stroke_width(2.0);
    stroke.set_color(skia_safe::Color::from_argb(255, 255, 215, 0));
    canvas.draw_round_rect(rect, 8.0, 8.0, &stroke);

    let Some(font) = harvest_text_font(font_size) else {
        tracing::warn!("MySekai harvest 字体不可用，跳过稀有资源数量文本");
        return;
    };
    for (index, (resource_id, (quantity, asset_key))) in summary.into_iter().enumerate() {
        let item_x = x + index as f32 * item_width + padding;
        let icon_rect = SkRect::from_xywh(item_x, y + padding, icon, icon);
        draw_drop_icon(
            canvas,
            assets,
            &AggregatedDrop {
                resource_id,
                quantity,
                asset_key,
            },
            icon_rect,
        );
        let text_value = format!("x{quantity}");
        let mut text = Paint::default();
        text.set_anti_alias(true);
        text.set_color(skia_safe::Color::from_argb(255, 255, 215, 0));
        let text_width = estimate_text_width(&text_value, font_size);
        canvas.draw_str(
            text_value,
            (
                item_x + (icon - text_width) / 2.0,
                y + padding + icon + font_size,
            ),
            &font,
            &text,
        );
    }
}

#[cfg(feature = "skia-core")]
fn harvest_text_font(size: f32) -> Option<skia_safe::Font> {
    let font_mgr = skia_safe::FontMgr::default();
    let typeface = crate::text::resolve_custom_profile_typeface(
        &font_mgr,
        Some(crate::widgets::theme::fonts::PRIMARY),
    )
    .or_else(|| {
        crate::text::resolve_custom_profile_typeface(
            &font_mgr,
            Some(crate::widgets::theme::fonts::EMPHASIS),
        )
    })
    .or_else(|| font_mgr.match_family_style("Noto Sans CJK SC", skia_safe::FontStyle::default()))
    .or_else(|| font_mgr.match_family_style("Noto Sans CJK", skia_safe::FontStyle::default()))
    .or_else(|| font_mgr.legacy_make_typeface(None, skia_safe::FontStyle::default()))?;

    Some(skia_safe::Font::new(typeface, Some(size)))
}

#[cfg(feature = "skia-core")]
fn nearest_label_edge(start: Point, label: Rect) -> Point {
    let candidates = [
        Point {
            x: label.x + label.w / 2.0,
            y: label.y,
        },
        Point {
            x: label.x + label.w / 2.0,
            y: label.y + label.h,
        },
        Point {
            x: label.x,
            y: label.y + label.h / 2.0,
        },
        Point {
            x: label.x + label.w,
            y: label.y + label.h / 2.0,
        },
    ];
    candidates
        .into_iter()
        .min_by(|left, right| distance(start, *left).total_cmp(&distance(start, *right)))
        .unwrap_or(Point {
            x: label.x + label.w / 2.0,
            y: label.y + label.h / 2.0,
        })
}

#[cfg(feature = "skia-core")]
fn skia_rect(rect: Rect) -> skia_safe::Rect {
    skia_safe::Rect::from_xywh(rect.x, rect.y, rect.w, rect.h)
}

#[cfg(feature = "skia-core")]
fn skia_color(color: Rgb, alpha: u8) -> skia_safe::Color {
    skia_safe::Color::from_argb(alpha, color.r, color.g, color.b)
}

#[cfg(not(feature = "skia-core"))]
fn placeholder_jpeg() -> &'static [u8] {
    &[
        0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00, 0x01, 0x01, 0x01, 0x00,
        0x48, 0x00, 0x48, 0x00, 0x00, 0xff, 0xdb, 0x00, 0x43, 0x00, 0x08, 0x06, 0x06, 0x07, 0x06,
        0x05, 0x08, 0x07, 0x07, 0x07, 0x09, 0x09, 0x08, 0x0a, 0x0c, 0x14, 0x0d, 0x0c, 0x0b, 0x0b,
        0x0c, 0x19, 0x12, 0x13, 0x0f, 0x14, 0x1d, 0x1a, 0x1f, 0x1e, 0x1d, 0x1a, 0x1c, 0x1c, 0x20,
        0x24, 0x2e, 0x27, 0x20, 0x22, 0x2c, 0x23, 0x1c, 0x1c, 0x28, 0x37, 0x29, 0x2c, 0x30, 0x31,
        0x34, 0x34, 0x34, 0x1f, 0x27, 0x39, 0x3d, 0x38, 0x32, 0x3c, 0x2e, 0x33, 0x34, 0x32, 0xff,
        0xc0, 0x00, 0x0b, 0x08, 0x00, 0x01, 0x00, 0x01, 0x01, 0x01, 0x11, 0x00, 0xff, 0xc4, 0x00,
        0x14, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0xff, 0xc4, 0x00, 0x14, 0x10, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0xda, 0x00, 0x08,
        0x01, 0x01, 0x00, 0x00, 0x3f, 0x00, 0xd2, 0xcf, 0x20, 0xff, 0xd9,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_input() -> HarvestMapRenderInput {
        HarvestMapRenderInput {
            site_id: 5,
            fixtures: vec![
                HarvestFixture {
                    fixture_id: 111,
                    position_x: 1.0,
                    position_z: 2.0,
                    status: "spawned".to_string(),
                },
                HarvestFixture {
                    fixture_id: 112,
                    position_x: 3.0,
                    position_z: 4.0,
                    status: "reserved".to_string(),
                },
            ],
            drops: vec![
                HarvestDrop {
                    resource_type: "mysekai_material".to_string(),
                    resource_id: 121,
                    position_x: 1.0,
                    position_z: 2.0,
                    quantity: 2,
                    status: "before_drop".to_string(),
                    asset_key: Some(
                        "mysekai/resource_drops/mdl_non1001_before_sapling1_121".to_string(),
                    ),
                },
                HarvestDrop {
                    resource_type: "mysekai_material".to_string(),
                    resource_id: 121,
                    position_x: 1.0,
                    position_z: 2.0,
                    quantity: 3,
                    status: "before_drop".to_string(),
                    asset_key: Some(
                        "mysekai/resource_drops/mdl_non1001_before_sapling1_121".to_string(),
                    ),
                },
                HarvestDrop {
                    resource_type: "mysekai_music_record".to_string(),
                    resource_id: 1,
                    position_x: 1.0,
                    position_z: 2.0,
                    quantity: 1,
                    status: "before_drop".to_string(),
                    asset_key: None,
                },
            ],
            rare_resource_ids: vec![121],
            quality: DEFAULT_QUALITY,
        }
    }

    #[test]
    fn mysekai_harvest_transforms_site5_coordinates() {
        let config = map_config(5).expect("site 5 config");
        let pos = game_pos_to_pixel(&config, 1.0, 2.0);
        assert!((pos.x - 625.334).abs() < 0.01);
        assert!((pos.y - 312.667).abs() < 0.01);
    }

    #[test]
    fn mysekai_harvest_aggregates_and_filters_drops() {
        let drops = aggregate_drops(&sample_input().drops);
        let grouped = drops
            .get(&position_key(1.0, 2.0))
            .expect("position should exist");
        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped[0].resource_id, 121);
        assert_eq!(grouped[0].quantity, 5);
    }

    #[test]
    fn mysekai_harvest_layout_uses_bounded_label_search() {
        let mut items = build_render_items(&sample_input()).expect("items");
        assert_eq!(items.len(), 1);
        let label = items.remove(0).label_rect.expect("label should be placed");
        assert!(label.w > 0.0);
        assert!(label.h > 0.0);
    }

    #[cfg(feature = "skia-core")]
    #[test]
    #[ignore = "static 素材不在 oss 仓库中"]
    fn mysekai_harvest_render_smoke() {
        let assets = AssetStore::new(128);
        assets
            .load_static_dir(std::path::Path::new("../../assets/static"))
            .expect("static assets");
        let output = render_harvest_map(&sample_input(), &assets).expect("render");
        assert_eq!(output.width, 944);
        assert_eq!(output.height, 756);
        assert!(output.data.len() > 50_000);
    }
}

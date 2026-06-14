//! 渲染执行器——最基础的线程池隔离示例（已弃用）。
//!
//! 推荐用法是直接调用纯同步的 [`crate::render_document::render_document`]，
//! 由调用方决定在哪个线程、以何种并发策略执行。渲染层只负责"怎么画"，
//! 并发与调度属于上层职责。
//!
//! 本执行器演示的是把同步渲染接入异步服务的**最基础形态**：丢进独立的
//! rayon 线程池、用 oneshot channel 桥接回 async，避免 CPU 密集的光栅化
//! 阻塞 Tokio 的异步 I/O 线程。
//!
//! 但线程池隔离只是最基础的一层。生产环境通常还需要请求优先级、队列上限与
//! 背压、单飞/去重、超时与取消传播、worker panic 重生、指标埋点等——这些都
//! 不在渲染层内，由本 crate 之外的上层调度器承载，`RenderExecutor` 并不覆盖。
//!
//! 生产主路径已改为直接调用 `render_document`，本执行器目前仅由 `render-deck`
//! 示例工具使用。

use std::sync::Arc;

use crate::assets::AssetStore;
use crate::error::RenderError;
use crate::mysekai_harvest::HarvestMapRenderInput;
use crate::primitives::SceneTree;
use crate::ranking::RankingRenderInput;
use crate::traits::{RenderOutput, Renderable};
use crate::widgets::card_thumbnail::CardThumbnail;

/// 渲染执行器，管理 Skia 渲染线程池。
///
/// 在独立 rayon 线程池中调度渲染，避免 CPU 密集渲染阻塞 Tokio 的异步 I/O 线程。
///
/// 生产主路径已改为直接调用 [`crate::render_document::render_document`]，本执行器
/// 仅作为线程池调度示例保留，供 `render-deck` 示例工具使用。
#[deprecated(note = "生产主路径改用 render_document::render_document；此执行器仅作示例保留")]
pub struct RenderExecutor {
    /// 独立的 Skia 渲染线程池
    thread_pool: rayon::ThreadPool,
    /// 共享的素材内存缓存
    assets: Arc<AssetStore>,
}

#[allow(deprecated)]
impl RenderExecutor {
    /// 创建渲染执行器。
    ///
    /// # 参数
    /// - `render_threads`: 渲染线程数（推荐 2~4）
    /// - `assets`: 素材缓存
    pub fn new(render_threads: usize, assets: Arc<AssetStore>) -> Result<Self, String> {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(render_threads)
            .thread_name(|i| format!("skia-worker-{i}"))
            .build()
            .map_err(|e| format!("渲染线程池创建失败: {e}"))?;

        Ok(Self {
            thread_pool: pool,
            assets,
        })
    }

    /// 在渲染线程池中执行 WidgetDocument 渲染。
    pub async fn execute_document(
        &self,
        doc: crate::widget_node::WidgetDocument,
        theme: crate::widgets::theme::Theme,
    ) -> Result<RenderOutput, RenderError> {
        let assets = self.assets.clone();
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<RenderOutput, RenderError>>();

        self.thread_pool.spawn(move || {
            let ctx = crate::context::RenderContext::new(&assets, &theme);
            let result = crate::render_document::render_document(&doc, &ctx);
            let output = RenderOutput {
                data: result.image,
                content_type: match result.format {
                    crate::widget_node::OutputFormat::Jpeg(_) => "image/jpeg",
                    crate::widget_node::OutputFormat::Png => "image/png",
                    crate::widget_node::OutputFormat::Webp(_) => "image/webp",
                }
                .to_string(),
                width: result.width,
                height: result.height,
                timing: Some(result.timing),
            };
            let _ = tx.send(Ok(output));
        });

        rx.await
            .map_err(|_| RenderError::Render("渲染任务被取消".into()))?
    }

    /// 在渲染线程池中预热卡面缩略图缓存。
    pub async fn prewarm_card_thumbnails(
        &self,
        cards: Vec<CardThumbnail>,
        theme: crate::widgets::theme::Theme,
    ) -> Result<usize, RenderError> {
        if cards.is_empty() {
            return Ok(0);
        }

        let assets = self.assets.clone();
        let (tx, rx) = tokio::sync::oneshot::channel::<usize>();

        self.thread_pool.spawn(move || {
            let ctx = crate::context::RenderContext::new(&assets, &theme);
            let count = cards.len();
            for card in &cards {
                card.prewarm(&ctx);
            }
            let _ = tx.send(count);
        });

        rx.await
            .map_err(|_| RenderError::Render("缩略图预热任务被取消".into()))
    }

    /// 在渲染线程池中预热玻璃面板缓存。
    pub async fn prewarm_glass_panels(
        &self,
        panels: Vec<(f32, f32, f32)>,
        theme: crate::widgets::theme::Theme,
    ) -> Result<usize, RenderError> {
        if panels.is_empty() {
            return Ok(0);
        }

        let (tx, rx) = tokio::sync::oneshot::channel::<usize>();
        self.thread_pool.spawn(move || {
            let count = panels.len();
            for (width, height, variance) in panels {
                crate::widgets::glass_panel::prewarm_glass_panel(width, height, variance, &theme);
            }
            let _ = tx.send(count);
        });

        rx.await
            .map_err(|_| RenderError::Render("玻璃面板预热任务被取消".into()))
    }

    /// 在渲染线程池中执行 MySekai 采集地图渲染。
    pub async fn execute_harvest_map(
        &self,
        input: HarvestMapRenderInput,
    ) -> Result<RenderOutput, RenderError> {
        let assets = self.assets.clone();
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<RenderOutput, RenderError>>();

        self.thread_pool.spawn(move || {
            let result = crate::mysekai_harvest::render_harvest_map(&input, &assets);
            let _ = tx.send(result);
        });

        rx.await
            .map_err(|_| RenderError::Render("MySekai 地图渲染任务被取消".into()))?
    }

    /// 在渲染线程池中执行排行榜图片渲染。
    pub async fn execute_ranking(
        &self,
        input: RankingRenderInput,
    ) -> Result<RenderOutput, RenderError> {
        let assets = self.assets.clone();
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<RenderOutput, RenderError>>();

        self.thread_pool.spawn(move || {
            let result = crate::ranking::render_ranking(&input, &assets);
            let _ = tx.send(result);
        });

        rx.await
            .map_err(|_| RenderError::Render("排行榜渲染任务被取消".into()))?
    }

    /// 在渲染线程池中执行渲染任务。
    ///
    /// 1. 在 rayon 线程池中调用 `compose()` 生成图元树
    /// 2. TODO: 遍历图元树 → Skia Canvas → 编码为 JPEG
    /// 3. 通过 oneshot channel 桥接回异步世界
    pub async fn execute(
        &self,
        renderable: Box<dyn Renderable>,
    ) -> Result<RenderOutput, RenderError> {
        let assets = self.assets.clone();
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<RenderOutput, RenderError>>();

        self.thread_pool.spawn(move || {
            let result = renderable
                .compose(&assets)
                .and_then(|scene_tree| render_scene(&scene_tree, &assets));
            // 忽略 send 错误（调用方可能已超时取消）
            let _ = tx.send(result);
        });

        rx.await
            .map_err(|_| RenderError::Render("渲染任务被取消".into()))?
    }
}

#[cfg(feature = "skia-core")]
fn render_scene(scene: &SceneTree, assets: &AssetStore) -> Result<RenderOutput, RenderError> {
    let mut surface =
        skia_safe::surfaces::raster_n32_premul((scene.width as i32, scene.height as i32))
            .ok_or_else(|| RenderError::Render("创建 Skia Surface 失败".to_string()))?;
    let canvas = surface.canvas();
    draw_primitive(canvas, &scene.root, 0.0, 0.0, assets);
    let image = surface.image_snapshot();
    let ctx: Option<&mut skia_safe::gpu::DirectContext> = None;
    let data = image
        .encode(ctx, skia_safe::EncodedImageFormat::JPEG, Some(90))
        .ok_or_else(|| RenderError::Encode("JPEG 编码失败".to_string()))?;
    Ok(RenderOutput {
        data: data.as_bytes().to_vec(),
        content_type: "image/jpeg".to_string(),
        width: scene.width,
        height: scene.height,
        timing: None,
    })
}

#[cfg(not(feature = "skia-core"))]
fn render_scene(scene: &SceneTree, _assets: &AssetStore) -> Result<RenderOutput, RenderError> {
    Ok(RenderOutput {
        data: placeholder_jpeg().to_vec(),
        content_type: "image/jpeg".to_string(),
        width: scene.width,
        height: scene.height,
        timing: None,
    })
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

#[cfg(feature = "skia-core")]
fn draw_primitive(
    canvas: &skia_safe::Canvas,
    primitive: &crate::primitives::Primitive,
    x: f32,
    y: f32,
    assets: &AssetStore,
) {
    use crate::primitives::{Align, ImageFit, Layout, Primitive};

    match primitive {
        Primitive::Text {
            content,
            font_size,
            color,
            align,
            ..
        } => {
            let mut paint = skia_safe::Paint::default();
            paint.set_anti_alias(true);
            paint.set_color(skia_color(*color));
            let font = skia_safe::Font::default();
            let Some(font) = font.with_size(*font_size) else {
                return;
            };
            let text_x = match align {
                Align::Left => x,
                Align::Center => x + 0.5 * estimate_text_width(content, *font_size),
                Align::Right => x + estimate_text_width(content, *font_size),
            };
            canvas.draw_str(content, (text_x, y + font_size), &font, &paint);
        }
        Primitive::Image {
            asset_key,
            width,
            height,
            fit,
        } => {
            let dst = skia_safe::Rect::from_xywh(x, y, *width, *height);
            if let Some(image) = assets.get_image(asset_key) {
                let img_w = image.width() as f32;
                let img_h = image.height() as f32;
                let paint = skia_safe::Paint::default();
                match fit {
                    ImageFit::Fill => {
                        canvas.draw_image_rect(image, None, dst, &paint);
                    }
                    ImageFit::Cover => {
                        let (sx, sy, sw, sh) = crate::widgets::card_util::cover_crop_rect(
                            img_w, img_h, *width, *height,
                        );
                        let src = skia_safe::Rect::from_xywh(sx, sy, sw, sh);
                        canvas.draw_image_rect(
                            image,
                            Some((&src, skia_safe::canvas::SrcRectConstraint::Fast)),
                            dst,
                            &paint,
                        );
                    }
                    ImageFit::Contain => {
                        let scale = (*width / img_w).min(*height / img_h);
                        let dw = img_w * scale;
                        let dh = img_h * scale;
                        let dx = x + (*width - dw) / 2.0;
                        let dy = y + (*height - dh) / 2.0;
                        canvas.draw_image_rect(
                            image,
                            None,
                            skia_safe::Rect::from_xywh(dx, dy, dw, dh),
                            &paint,
                        );
                    }
                }
            } else {
                let mut paint = skia_safe::Paint::default();
                paint.set_anti_alias(true);
                paint.set_color(skia_safe::Color::from_argb(255, 48, 48, 64));
                canvas.draw_rect(dst, &paint);
            }
        }
        Primitive::Rect {
            width,
            height,
            color,
            radius,
            border,
        } => {
            let rect = skia_safe::Rect::from_xywh(x, y, *width, *height);
            let mut fill = skia_safe::Paint::default();
            fill.set_anti_alias(true);
            fill.set_style(skia_safe::PaintStyle::Fill);
            fill.set_color(skia_color(*color));
            canvas.draw_round_rect(rect, *radius, *radius, &fill);
            if let Some(border) = border {
                let mut stroke = skia_safe::Paint::default();
                stroke.set_anti_alias(true);
                stroke.set_style(skia_safe::PaintStyle::Stroke);
                stroke.set_stroke_width(border.width);
                stroke.set_color(skia_color(border.color));
                canvas.draw_round_rect(rect, *radius, *radius, &stroke);
            }
        }
        Primitive::Container { layout, children } => match layout {
            Layout::Absolute => {
                for child in children {
                    draw_primitive(canvas, &child.primitive, x + child.x, y + child.y, assets);
                }
            }
            Layout::Horizontal { gap } => {
                let mut cursor = x;
                for child in children {
                    draw_primitive(canvas, &child.primitive, cursor, y, assets);
                    cursor += primitive_size(&child.primitive).0 + gap;
                }
            }
            Layout::Vertical { gap } => {
                let mut cursor = y;
                for child in children {
                    draw_primitive(canvas, &child.primitive, x, cursor, assets);
                    cursor += primitive_size(&child.primitive).1 + gap;
                }
            }
            Layout::Grid { columns, gap } => {
                let columns = (*columns).max(1);
                for (index, child) in children.iter().enumerate() {
                    let (w, h) = primitive_size(&child.primitive);
                    let col = index as u32 % columns;
                    let row = index as u32 / columns;
                    draw_primitive(
                        canvas,
                        &child.primitive,
                        x + col as f32 * (w + gap),
                        y + row as f32 * (h + gap),
                        assets,
                    );
                }
            }
        },
    }
}

#[cfg(feature = "skia-core")]
fn primitive_size(primitive: &crate::primitives::Primitive) -> (f32, f32) {
    use crate::primitives::{Layout, Primitive};
    match primitive {
        Primitive::Text {
            content, font_size, ..
        } => (estimate_text_width(content, *font_size), font_size * 1.2),
        Primitive::Image { width, height, .. } | Primitive::Rect { width, height, .. } => {
            (*width, *height)
        }
        Primitive::Container { layout, children } => match layout {
            Layout::Absolute => children.iter().fold((0.0_f32, 0.0_f32), |acc, child| {
                let (w, h) = primitive_size(&child.primitive);
                (acc.0.max(child.x + w), acc.1.max(child.y + h))
            }),
            Layout::Horizontal { gap } => {
                let mut width = 0.0;
                let mut height = 0.0_f32;
                for (index, child) in children.iter().enumerate() {
                    let size = primitive_size(&child.primitive);
                    width += size.0;
                    if index > 0 {
                        width += gap;
                    }
                    height = height.max(size.1);
                }
                (width, height)
            }
            Layout::Vertical { gap } => {
                let mut width = 0.0_f32;
                let mut height = 0.0;
                for (index, child) in children.iter().enumerate() {
                    let size = primitive_size(&child.primitive);
                    width = width.max(size.0);
                    height += size.1;
                    if index > 0 {
                        height += gap;
                    }
                }
                (width, height)
            }
            Layout::Grid { columns, gap } => {
                let columns = (*columns).max(1);
                let mut cell = (0.0_f32, 0.0_f32);
                for child in children {
                    let size = primitive_size(&child.primitive);
                    cell.0 = cell.0.max(size.0);
                    cell.1 = cell.1.max(size.1);
                }
                let rows = (children.len() as u32).div_ceil(columns).max(1);
                (
                    columns as f32 * cell.0 + columns.saturating_sub(1) as f32 * gap,
                    rows as f32 * cell.1 + rows.saturating_sub(1) as f32 * gap,
                )
            }
        },
    }
}

#[cfg(feature = "skia-core")]
fn estimate_text_width(text: &str, font_size: f32) -> f32 {
    text.chars().count() as f32 * font_size * 0.58
}

#[cfg(feature = "skia-core")]
fn skia_color(color: crate::primitives::Color) -> skia_safe::Color {
    skia_safe::Color::from_argb(color.a, color.r, color.g, color.b)
}

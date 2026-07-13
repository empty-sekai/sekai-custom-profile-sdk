//! WidgetDocument 整树渲染入口。

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::context::RenderContext;
use crate::instantiate::instantiate;
use crate::widget_node::{Layout, NodeKind, OutputFormat, Position, WidgetDocument, WidgetNode};

/// 文档渲染结果。
#[derive(Debug, Clone)]
pub struct RenderResult {
    /// 渲染后的字节数据。
    pub image: Vec<u8>,
    /// 输出格式。
    pub format: OutputFormat,
    /// 节点级诊断信息。
    pub diagnostics: Vec<NodeDiagnostic>,
    /// 输出宽度。
    pub width: u32,
    /// 输出高度。
    pub height: u32,
    /// 渲染分段耗时。
    pub timing: RenderTiming,
}

/// 文档渲染分段耗时。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RenderTiming {
    /// 布局耗时，毫秒。
    pub layout_ms: f64,
    /// Skia draw 耗时，毫秒。
    pub draw_ms: f64,
    /// 图片编码耗时，毫秒。
    pub encode_ms: f64,
    /// 按 Widget 类型聚合的绘制耗时。
    pub widget_draws: Vec<WidgetDrawTiming>,
}

impl RenderTiming {
    #[cfg(feature = "skia")]
    fn add_widget_draw(&mut self, widget_type: &str, duration: Duration) {
        let elapsed = elapsed_ms(duration);
        if let Some(existing) = self
            .widget_draws
            .iter_mut()
            .find(|item| item.widget_type == widget_type)
        {
            existing.count += 1;
            existing.draw_ms += elapsed;
            return;
        }
        self.widget_draws.push(WidgetDrawTiming {
            widget_type: widget_type.to_string(),
            count: 1,
            draw_ms: elapsed,
        });
    }

    /// 返回 header 友好的热点摘要。
    pub fn hotspot_header(&self) -> String {
        let mut by_cost = self.widget_draws.clone();
        by_cost.sort_by(|a, b| {
            b.draw_ms
                .partial_cmp(&a.draw_ms)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        by_cost
            .iter()
            .map(|item| format!("{}:{:.2}/{}", item.widget_type, item.draw_ms, item.count))
            .collect::<Vec<_>>()
            .join(";")
    }

    /// 按类型查询绘制耗时与次数。
    pub fn widget(&self, widget_type: &str) -> Option<&WidgetDrawTiming> {
        self.widget_draws
            .iter()
            .find(|item| item.widget_type == widget_type)
    }
}

/// 单类 Widget 的绘制耗时。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WidgetDrawTiming {
    /// Widget 类型名。
    pub widget_type: String,
    /// 绘制次数。
    pub count: usize,
    /// 累计绘制耗时，毫秒。
    pub draw_ms: f64,
}

/// 节点级诊断信息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NodeDiagnostic {
    /// 发生问题的节点 ID。
    pub node_id: String,
    /// 严重级别。
    pub severity: Severity,
    /// 诊断代码。
    pub code: DiagnosticCode,
    /// 诊断消息。
    pub message: String,
}

/// 诊断严重级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// 错误。
    Error,
    /// 警告。
    Warning,
    /// 信息。
    Info,
}

/// 诊断代码。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticCode {
    /// 流式布局中 position 被忽略。
    PositionIgnored,
}

#[cfg(any(test, not(feature = "skia")))]
#[derive(Debug, Clone, Serialize)]
struct LayoutSnapshot {
    node_id: String,
    node_type: String,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    rotation: f32,
    scale: (f32, f32),
    children: Vec<LayoutSnapshot>,
}

#[derive(Debug, Clone)]
struct LaidOutNode {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    rotation: f32,
    scale: (f32, f32),
    node: WidgetNode,
    children: Vec<LaidOutNode>,
    visible: bool,
}

/// 渲染整棵 WidgetDocument。
pub fn render_document(doc: &WidgetDocument, ctx: &RenderContext<'_>) -> RenderResult {
    let mut diagnostics = Vec::new();
    let layout_start = Instant::now();
    let root = layout_node(&doc.root, ctx, &mut diagnostics);
    let mut timing = RenderTiming {
        layout_ms: elapsed_ms(layout_start.elapsed()),
        ..RenderTiming::default()
    };

    #[cfg(feature = "skia")]
    let image = render_document_skia(doc, ctx, &root, &mut timing);

    #[cfg(not(feature = "skia"))]
    let image = {
        let encode_start = Instant::now();
        let image = render_document_snapshot(&root, doc);
        timing.encode_ms = elapsed_ms(encode_start.elapsed());
        image
    };

    RenderResult {
        image,
        format: doc.output.clone(),
        diagnostics,
        width: doc.canvas.width,
        height: doc.canvas.height,
        timing,
    }
}

fn measure_node(node: &WidgetNode, ctx: &RenderContext<'_>) -> (f32, f32) {
    match &node.kind {
        NodeKind::Container { layout, children } => measure_container(layout, children, ctx),
        _ => instantiate(node).measure(ctx),
    }
}

fn measure_container(
    layout: &Layout,
    children: &[WidgetNode],
    ctx: &RenderContext<'_>,
) -> (f32, f32) {
    match layout {
        Layout::Absolute => children.iter().fold((0.0_f32, 0.0_f32), |acc, child| {
            let (width, height) = measure_node(child, ctx);
            let pos = child.position;
            (acc.0.max(pos.x + width), acc.1.max(pos.y + height))
        }),
        Layout::Horizontal { gap } => {
            let mut total_width = 0.0_f32;
            let mut max_height = 0.0_f32;
            for (index, child) in children.iter().enumerate() {
                let (width, height) = measure_node(child, ctx);
                if index > 0 {
                    total_width += *gap;
                }
                total_width += width;
                max_height = max_height.max(height);
            }
            (total_width, max_height)
        }
        Layout::Vertical { gap } => {
            let mut max_width = 0.0_f32;
            let mut total_height = 0.0_f32;
            for (index, child) in children.iter().enumerate() {
                let (width, height) = measure_node(child, ctx);
                if index > 0 {
                    total_height += *gap;
                }
                total_height += height;
                max_width = max_width.max(width);
            }
            (max_width, total_height)
        }
    }
}

fn layout_node(
    node: &WidgetNode,
    ctx: &RenderContext<'_>,
    diagnostics: &mut Vec<NodeDiagnostic>,
) -> LaidOutNode {
    let default_position = Position::default();
    let (width, height) = measure_node(node, ctx);

    match &node.kind {
        NodeKind::Container { layout, children } => {
            let laid_out_children = layout_children(&node.id, layout, children, ctx, diagnostics);
            LaidOutNode {
                x: 0.0,
                y: 0.0,
                width,
                height,
                rotation: default_position.rotation,
                scale: default_position.scale,
                node: node.clone(),
                children: laid_out_children,
                visible: true,
            }
        }
        _ => LaidOutNode {
            x: 0.0,
            y: 0.0,
            width,
            height,
            rotation: default_position.rotation,
            scale: default_position.scale,
            node: node.clone(),
            children: Vec::new(),
            visible: true,
        },
    }
}

fn layout_children(
    _parent_id: &str,
    layout: &Layout,
    children: &[WidgetNode],
    ctx: &RenderContext<'_>,
    diagnostics: &mut Vec<NodeDiagnostic>,
) -> Vec<LaidOutNode> {
    match layout {
        Layout::Absolute => children
            .iter()
            .map(|child| {
                let mut node = layout_node(child, ctx, diagnostics);
                let position = child.position;
                node.x = position.x;
                node.y = position.y;
                node.rotation = position.rotation;
                node.scale = position.scale;
                node.visible = child.visible;
                node
            })
            .collect(),
        Layout::Horizontal { gap } => {
            let mut cursor_x = 0.0_f32;
            children
                .iter()
                .map(|child| {
                    if !child.position.is_default() {
                        diagnostics.push(NodeDiagnostic {
                            node_id: child.id.clone(),
                            severity: Severity::Warning,
                            code: DiagnosticCode::PositionIgnored,
                            message: format!(
                                "节点 {} 在 horizontal 布局中忽略了 position",
                                child.id
                            ),
                        });
                    }
                    let mut node = layout_node(child, ctx, diagnostics);
                    node.x = cursor_x;
                    cursor_x += node.width + *gap;
                    node.visible = child.visible;
                    node
                })
                .collect::<Vec<_>>()
        }
        Layout::Vertical { gap } => {
            let mut cursor_y = 0.0_f32;
            children
                .iter()
                .map(|child| {
                    if !child.position.is_default() {
                        diagnostics.push(NodeDiagnostic {
                            node_id: child.id.clone(),
                            severity: Severity::Warning,
                            code: DiagnosticCode::PositionIgnored,
                            message: format!("节点 {} 在 vertical 布局中忽略了 position", child.id),
                        });
                    }
                    let mut node = layout_node(child, ctx, diagnostics);
                    node.y = cursor_y;
                    cursor_y += node.height + *gap;
                    node.visible = child.visible;
                    node
                })
                .collect::<Vec<_>>()
        }
    }
}

#[cfg(not(feature = "skia"))]
fn render_document_snapshot(root: &LaidOutNode, doc: &WidgetDocument) -> Vec<u8> {
    let snapshot = serde_json::json!({
        "canvas": {
            "width": doc.canvas.width,
            "height": doc.canvas.height,
        },
        "root": snapshot_node(root),
    });
    serde_json::to_vec(&snapshot).unwrap_or_else(|_| b"{\"render\":\"snapshot\"}".to_vec())
}

#[cfg(feature = "skia")]
fn render_document_skia(
    doc: &WidgetDocument,
    ctx: &RenderContext<'_>,
    root: &LaidOutNode,
    timing: &mut RenderTiming,
) -> Vec<u8> {
    use skia_safe::surfaces;

    let Some(mut surface) =
        surfaces::raster_n32_premul((doc.canvas.width as i32, doc.canvas.height as i32))
    else {
        return b"skia-surface-create-failed".to_vec();
    };

    let canvas = surface.canvas();
    let draw_start = Instant::now();
    canvas.clear(doc.canvas.background.to_skia().to_color());
    draw_node(canvas, root, ctx, timing);
    timing.draw_ms = elapsed_ms(draw_start.elapsed());

    let image = surface.image_snapshot();
    let encode_start = Instant::now();
    let encoded = match &doc.output {
        OutputFormat::Jpeg(quality) => image
            .encode(
                None,
                skia_safe::EncodedImageFormat::JPEG,
                Some(u32::from(*quality)),
            )
            .map(|data| data.as_bytes().to_vec())
            .unwrap_or_else(|| b"jpeg-encode-failed".to_vec()),
        OutputFormat::Png => image
            .encode(None, skia_safe::EncodedImageFormat::PNG, Some(100))
            .map(|data| data.as_bytes().to_vec())
            .unwrap_or_else(|| b"png-encode-failed".to_vec()),
        OutputFormat::Webp(quality) => image
            .encode(
                None,
                skia_safe::EncodedImageFormat::WEBP,
                Some(u32::from(*quality)),
            )
            .map(|data| data.as_bytes().to_vec())
            .unwrap_or_else(|| b"webp-encode-failed".to_vec()),
    };
    timing.encode_ms = elapsed_ms(encode_start.elapsed());
    encoded
}

#[cfg(feature = "skia")]
fn draw_node(
    canvas: &skia_safe::Canvas,
    node: &LaidOutNode,
    ctx: &RenderContext<'_>,
    timing: &mut RenderTiming,
) {
    canvas.save();
    canvas.translate((node.x, node.y));
    apply_transform(canvas, node.width, node.height, node.rotation, node.scale);

    match &node.node.kind {
        NodeKind::Container { .. } => {
            for child in &node.children {
                if !child.visible {
                    continue;
                }
                draw_node(canvas, child, ctx, timing);
            }
        }
        _ => {
            let widget = instantiate(&node.node);
            let draw_start = Instant::now();
            widget.draw(canvas, 0.0, 0.0, ctx);
            timing.add_widget_draw(widget.name(), draw_start.elapsed());
        }
    }

    canvas.restore();
}

fn elapsed_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

#[cfg(feature = "skia")]
fn apply_transform(
    canvas: &skia_safe::Canvas,
    width: f32,
    height: f32,
    rotation: f32,
    scale: (f32, f32),
) {
    // 与浏览器 CSS 变换对齐：
    // .canvasWidget { transform: rotate(deg); transform-origin: center; }
    // .widgetSurface { transform: scale(sx, sy); transform-origin: top left; }
    //
    // 浏览器中变换顺序（从元素内部到屏幕）：
    // 1. .widgetSurface 先 scale（子元素级，从左上角）
    // 2. .canvasWidget 再 rotate（父元素级，围绕缩放后的中心）
    //
    // Skia 中 canvas 变换是后乘的（对点的实际作用顺序与代码顺序相反）。
    // 代码顺序从上到下 = 矩阵从右到左相乘。
    // 为了匹配 CSS：先 rotate 围绕 scaled_center，最后 scale。
    // 因此 Skia 代码中 scale 要写在最后（最先作用于点）。
    let scaled_cx = width * scale.0 / 2.0;
    let scaled_cy = height * scale.1 / 2.0;
    canvas.translate((scaled_cx, scaled_cy));
    if rotation.abs() > f32::EPSILON {
        canvas.rotate(rotation, None);
    }
    canvas.translate((-scaled_cx, -scaled_cy));
    canvas.scale(scale);
}

#[cfg(any(test, not(feature = "skia")))]
fn snapshot_node(node: &LaidOutNode) -> LayoutSnapshot {
    LayoutSnapshot {
        node_id: node.node.id.clone(),
        node_type: node.node.type_name().to_string(),
        x: node.x,
        y: node.y,
        width: node.width,
        height: node.height,
        rotation: node.rotation,
        scale: node.scale,
        children: node.children.iter().map(snapshot_node).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::{layout_node, render_document, snapshot_node, DiagnosticCode, Severity};
    use crate::assets::AssetStore;
    use crate::context::RenderContext;
    use crate::widget_node::{
        CanvasSpec, Layout, NodeKind, OutputFormat, Position, TextAlignValue, VAlignValue,
        WidgetDocument, WidgetNode, WIDGET_DOCUMENT_SCHEMA_VERSION,
    };
    use crate::widgets::theme::{Color, Theme};

    fn ctx() -> RenderContext<'static> {
        let assets = Box::leak(Box::new(AssetStore::new(8)));
        let theme = Box::leak(Box::new(Theme::default()));
        RenderContext::new(assets, theme)
    }

    fn leaf(id: &str, kind: NodeKind) -> WidgetNode {
        WidgetNode {
            id: id.to_string(),
            position: Position::default(),
            visible: true,
            kind,
        }
    }

    fn at(id: &str, position: Position, kind: NodeKind) -> WidgetNode {
        WidgetNode {
            id: id.to_string(),
            position,
            visible: true,
            kind,
        }
    }

    #[test]
    fn horizontal_layout_respects_gap() {
        let document = WidgetDocument {
            version: WIDGET_DOCUMENT_SCHEMA_VERSION,
            canvas: CanvasSpec {
                width: 400,
                height: 200,
                background: Color::new(0.0, 0.0, 0.0, 1.0),
            },
            root: WidgetNode {
                id: "root".to_string(),
                position: Position::default(),
                visible: true,
                kind: NodeKind::Container {
                    layout: Layout::Horizontal { gap: 10.0 },
                    children: vec![
                        leaf(
                            "a",
                            NodeKind::GlassPanel {
                                width: 20.0,
                                height: 10.0,
                                clip_variance: 0.0,
                            },
                        ),
                        leaf(
                            "b",
                            NodeKind::GlassPanel {
                                width: 30.0,
                                height: 10.0,
                                clip_variance: 0.0,
                            },
                        ),
                    ],
                },
            },
            output: OutputFormat::Png,
        };

        let mut diagnostics = Vec::new();
        let root = layout_node(&document.root, &ctx(), &mut diagnostics);
        let snapshot = snapshot_node(&root);

        assert_eq!(snapshot.children[0].x, 0.0);
        assert_eq!(snapshot.children[1].x, 30.0);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn flow_layout_reports_ignored_position() {
        let document = WidgetDocument {
            version: WIDGET_DOCUMENT_SCHEMA_VERSION,
            canvas: CanvasSpec {
                width: 400,
                height: 200,
                background: Color::new(0.0, 0.0, 0.0, 1.0),
            },
            root: WidgetNode {
                id: "root".to_string(),
                position: Position::default(),
                visible: true,
                kind: NodeKind::Container {
                    layout: Layout::Vertical { gap: 8.0 },
                    children: vec![at(
                        "text",
                        Position {
                            x: 99.0,
                            y: 88.0,
                            rotation: 0.0,
                            scale: (1.0, 1.0),
                        },
                        NodeKind::SimpleText {
                            content: "demo".to_string(),
                            font_size: 16.0,
                            color: Color::new(1.0, 1.0, 1.0, 1.0),
                            width: 260.0,
                            height: 72.0,
                            align: TextAlignValue::Left,
                            v_align: VAlignValue::Top,
                            padding: 4.0,
                            line_height: 1.2,
                            glow: false,
                        },
                    )],
                },
            },
            output: OutputFormat::Png,
        };

        let result = render_document(&document, &ctx());
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].severity, Severity::Warning);
        assert_eq!(result.diagnostics[0].code, DiagnosticCode::PositionIgnored);
    }

    #[test]
    fn render_document_returns_non_empty_bytes() {
        let document = WidgetDocument {
            version: WIDGET_DOCUMENT_SCHEMA_VERSION,
            canvas: CanvasSpec {
                width: 320,
                height: 180,
                background: Color::new(0.1, 0.1, 0.1, 1.0),
            },
            root: WidgetNode {
                id: "root".to_string(),
                position: Position::default(),
                visible: true,
                kind: NodeKind::Container {
                    layout: Layout::Absolute,
                    children: vec![
                        at(
                            "panel",
                            Position {
                                x: 12.0,
                                y: 16.0,
                                rotation: 0.0,
                                scale: (1.0, 1.0),
                            },
                            NodeKind::GlassPanel {
                                width: 120.0,
                                height: 64.0,
                                clip_variance: 0.0,
                            },
                        ),
                        at(
                            "title",
                            Position {
                                x: 24.0,
                                y: 28.0,
                                rotation: 0.0,
                                scale: (1.0, 1.0),
                            },
                            NodeKind::SimpleText {
                                content: "hello".to_string(),
                                font_size: 18.0,
                                color: Color::new(1.0, 1.0, 1.0, 1.0),
                                width: 260.0,
                                height: 72.0,
                                align: TextAlignValue::Left,
                                v_align: VAlignValue::Top,
                                padding: 4.0,
                                line_height: 1.2,
                                glow: false,
                            },
                        ),
                    ],
                },
            },
            output: OutputFormat::Png,
        };

        let result = render_document(&document, &ctx());
        assert!(!result.image.is_empty());
        assert_eq!(result.width, 320);
        assert_eq!(result.height, 180);
    }
}

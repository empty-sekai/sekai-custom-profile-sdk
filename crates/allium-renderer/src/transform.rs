//! 坐标系转换与四元数解析工具。

use crate::types::{ObjectData, Quaternion};

/// 名片画布宽度（Unity 坐标空间）。
pub const CANVAS_WIDTH: f32 = 1830.0;

/// 名片画布高度（Unity 坐标空间）。
pub const CANVAS_HEIGHT: f32 = 812.0;

/// 将 Unity 四元数转换为 2D 旋转角度（度）。
pub fn quaternion_to_degrees(q: &Quaternion) -> f32 {
    -(2.0 * q.z.atan2(q.w).to_degrees())
}

/// 将 Unity 坐标转换为 Skia 坐标。
pub fn unity_to_skia(unity_x: f32, unity_y: f32) -> (f32, f32) {
    unity_to_skia_for_canvas(unity_x, unity_y, CANVAS_WIDTH, CANVAS_HEIGHT)
}

/// 将 Unity 坐标转换为指定尺寸画布上的 Skia 坐标。
pub fn unity_to_skia_for_canvas(
    unity_x: f32,
    unity_y: f32,
    canvas_width: f32,
    canvas_height: f32,
) -> (f32, f32) {
    let skia_x = unity_x + canvas_width / 2.0;
    let skia_y = canvas_height / 2.0 - unity_y;
    (skia_x, skia_y)
}

/// 从 `ObjectData` 提取 2D 变换参数。
pub fn extract_transform(obj: &ObjectData) -> (f32, f32, f32, f32, f32) {
    extract_transform_for_canvas(obj, CANVAS_WIDTH, CANVAS_HEIGHT)
}

/// 从 `ObjectData` 提取指定画布尺寸下的 2D 变换参数。
pub fn extract_transform_for_canvas(
    obj: &ObjectData,
    canvas_width: f32,
    canvas_height: f32,
) -> (f32, f32, f32, f32, f32) {
    let (x, y) =
        unity_to_skia_for_canvas(obj.position.x, obj.position.y, canvas_width, canvas_height);
    let angle = quaternion_to_degrees(&obj.rotation);
    (x, y, angle, obj.scale.x, obj.scale.y)
}

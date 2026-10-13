//! Coordinate-space conversion and quaternion helpers.

use crate::profile_source::{ObjectData, Quaternion};

/// Profile-card canvas width in Unity coordinates.
pub const CANVAS_WIDTH: f32 = 1830.0;

/// Profile-card canvas height in Unity coordinates.
pub const CANVAS_HEIGHT: f32 = 812.0;

/// Converts a Unity quaternion to a 2D rotation in degrees.
pub fn quaternion_to_degrees(q: &Quaternion) -> f32 {
    -(2.0 * q.z.atan2(q.w).to_degrees())
}

/// Converts Unity coordinates to the canonical canvas coordinate space.
pub fn unity_to_skia(unity_x: f32, unity_y: f32) -> (f32, f32) {
    unity_to_skia_for_canvas(unity_x, unity_y, CANVAS_WIDTH, CANVAS_HEIGHT)
}

/// Converts Unity coordinates to a canvas with the requested dimensions.
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

/// Extracts the canonical 2D transform from `ObjectData`.
pub fn extract_transform(obj: &ObjectData) -> (f32, f32, f32, f32, f32) {
    extract_transform_for_canvas(obj, CANVAS_WIDTH, CANVAS_HEIGHT)
}

/// Extracts a 2D transform for a canvas with the requested dimensions.
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

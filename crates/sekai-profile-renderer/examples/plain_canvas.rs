//! Draws plain labels and graphics using the shared software compositor.
use sekai_profile_renderer::core::{Rect, ShapePrimitive};
use sekai_profile_renderer::profile_compositor::canvas::{Canvas, Font};
fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let font_path = args
        .get(1)
        .ok_or("usage: plain_canvas <font-file> <output.png>")?;
    let output = args.get(2).ok_or("output path missing")?;
    let bytes = std::fs::read(font_path).map_err(|e| e.to_string())?;
    let font = Font::new(&bytes, 24.0)?;
    let mut canvas = Canvas::new(640, 360, [245, 247, 250, 255])?;
    canvas.shape(
        Rect {
            x: 24.0,
            y: 24.0,
            width: 592.0,
            height: 312.0,
        },
        ShapePrimitive::RoundedRect {
            radius: [16.0, 16.0],
        },
        [255, 255, 255, 255],
        [210, 220, 230, 255],
        1.0,
    )?;
    canvas.quadratic(
        [[80.0, 260.0], [180.0, 80.0], [480.0, 160.0]],
        3.0,
        [50, 120, 190, 255],
    )?;
    let label = "Shared canvas 123";
    let width = font.measure(label)?;
    canvas.text(&font, label, (640.0 - width) / 2.0, 70.0, [30, 40, 55, 255])?;
    std::fs::write(output, canvas.encode_png()?).map_err(|e| e.to_string())?;
    Ok(())
}

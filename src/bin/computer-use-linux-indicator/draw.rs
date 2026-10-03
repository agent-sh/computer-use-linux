//! CPU drawing for the overlay surfaces. Everything takes logical coordinates
//! and a buffer scale, so surfaces stay crisp under fractional scaling.

use tiny_skia::{
    Color, FillRule, GradientStop, LinearGradient, Paint, Path, PathBuilder, Pixmap, PixmapPaint,
    Point, Rect, SpreadMode, Stroke, Transform,
};

/// Logical size of the square cursor sprite; the arrow tip is its centre.
pub const CURSOR_SIZE: f32 = 112.0;
/// Arrowhead outline relative to the tip, in logical pixels.
const ARROW: [(f32, f32); 4] = [(0.0, 0.0), (19.0, 5.2), (11.0, 9.6), (8.0, 18.5)];
const CORNER: f32 = 2.2;
const RIM: f32 = 1.7;
const FOG_CENTER: (f32, f32) = (8.0, 8.5);

pub fn with_alpha(color: Color, alpha: f32) -> Color {
    let mut color = color;
    color.set_alpha((color.alpha() * alpha).clamp(0.0, 1.0));
    color
}

fn sd_polygon(p: (f32, f32), v: &[(f32, f32)]) -> f32 {
    let dot = |a: (f32, f32), b: (f32, f32)| a.0 * b.0 + a.1 * b.1;
    let sub = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0, a.1 - b.1);
    let mut d = dot(sub(p, v[0]), sub(p, v[0]));
    let mut sign = 1.0;
    for i in 0..v.len() {
        let j = (i + v.len() - 1) % v.len();
        let e = sub(v[j], v[i]);
        let w = sub(p, v[i]);
        let t = (dot(w, e) / dot(e, e)).clamp(0.0, 1.0);
        let b = (w.0 - e.0 * t, w.1 - e.1 * t);
        d = d.min(dot(b, b));
        let c = [p.1 >= v[i].1, p.1 < v[j].1, e.0 * w.1 > e.1 * w.0];
        if c.iter().all(|&x| x) || c.iter().all(|&x| !x) {
            sign = -sign;
        }
    }
    sign * d.sqrt()
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Premultiplied "over".
fn over(dst: [f32; 4], rgb: [f32; 3], a: f32) -> [f32; 4] {
    [
        rgb[0] * a + dst[0] * (1.0 - a),
        rgb[1] * a + dst[1] * (1.0 - a),
        rgb[2] * a + dst[2] * (1.0 - a),
        a + dst[3] * (1.0 - a),
    ]
}

/// The software cursor: a white-rimmed arrowhead with a soft shadow inside a
/// fog, body and fog tinted with the agent colour. Rendered once per colour
/// and scale, then drawn with a transform each frame.
pub fn cursor_sprite(tint: Color, scale: f32) -> Option<Pixmap> {
    let px = (CURSOR_SIZE * scale).ceil() as u32;
    let mut pixmap = Pixmap::new(px, px)?;
    let mix = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let fog_rgb = [
        mix(tint.red(), 1.0, 0.35),
        mix(tint.green(), 1.0, 0.35),
        mix(tint.blue(), 1.0, 0.35),
    ];
    let body_rgb = [tint.red() * 0.62, tint.green() * 0.62, tint.blue() * 0.62];
    let half = CURSOR_SIZE / 2.0;
    for (i, pixel) in pixmap.pixels_mut().iter_mut().enumerate() {
        let (x, y) = ((i as u32 % px) as f32, (i as u32 / px) as f32);
        let p = ((x + 0.5) / scale - half, (y + 0.5) / scale - half);
        let fog_d = (p.0 - FOG_CENTER.0).hypot(p.1 - FOG_CENTER.1);
        // The window fades the gaussian tail to zero inside the sprite, or
        // its square edge would show on dark backgrounds.
        let fog = (0.36 * (-(fog_d / 17.0).powi(2)).exp() + 0.09 * (-(fog_d / 30.0).powi(2)).exp())
            * (1.0 - smoothstep(26.0, 44.0, fog_d));
        let mut c = over([0.0; 4], fog_rgb, fog);
        let shadow_d = sd_polygon((p.0, p.1 - 1.4), &ARROW) - CORNER;
        c = over(c, [0.0; 3], 0.38 * (1.0 - smoothstep(-1.0, 4.5, shadow_d)));
        let d = sd_polygon(p, &ARROW) - CORNER;
        let outer = (0.5 - d * scale).clamp(0.0, 1.0);
        let inner = (0.5 - (d + RIM) * scale).clamp(0.0, 1.0);
        c = over(c, [0.97, 0.97, 0.98], outer);
        c = over(c, body_rgb, inner);
        let alpha = (c[3] * 255.0).round() as u8;
        let channel = |v: f32| ((v * 255.0).round() as u8).min(alpha);
        if let Some(color) = tiny_skia::PremultipliedColorU8::from_rgba(
            channel(c[0]),
            channel(c[1]),
            channel(c[2]),
            alpha,
        ) {
            *pixel = color;
        }
    }
    Some(pixmap)
}

/// Draws `sprite` with its centre (the arrow tip) at logical `tip`.
pub fn draw_sprite(
    pixmap: &mut Pixmap,
    sprite: &Pixmap,
    scale: f32,
    tip: (f32, f32),
    rotation_deg: f32,
    size_factor: f32,
    opacity: f32,
) {
    let half = sprite.width() as f32 / 2.0;
    let transform = Transform::from_translate(tip.0 * scale, tip.1 * scale)
        .pre_rotate(rotation_deg)
        .pre_scale(size_factor, size_factor)
        .pre_translate(-half, -half);
    let paint = PixmapPaint {
        opacity,
        quality: tiny_skia::FilterQuality::Bilinear,
        ..PixmapPaint::default()
    };
    pixmap.draw_pixmap(0, 0, sprite.as_ref(), &paint, transform, None);
}

pub fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Path> {
    let r = r.min(w / 2.0).min(h / 2.0);
    // Cubic approximation of a quarter circle.
    let k = r * 0.552_284_8;
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

fn solid(color: Color) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    paint
}

/// Rounded panel with a soft drop shadow and a hairline border.
pub fn panel(
    pixmap: &mut Pixmap,
    scale: f32,
    rect: (f32, f32, f32, f32),
    radius: f32,
    fill: Color,
    border: Color,
    shadow_alpha: f32,
) {
    let transform = Transform::from_scale(scale, scale);
    let (x, y, w, h) = rect;
    // A few expanding translucent layers read as a blurred shadow.
    for step in 1..=4 {
        let grow = step as f32 * 2.0;
        if let Some(path) = rounded_rect(
            x - grow,
            y - grow + 4.0,
            w + 2.0 * grow,
            h + 2.0 * grow,
            radius + grow,
        ) {
            let paint = solid(
                Color::from_rgba(0.0, 0.0, 0.0, shadow_alpha / 6.0).unwrap_or(Color::TRANSPARENT),
            );
            pixmap.fill_path(&path, &paint, FillRule::Winding, transform, None);
        }
    }
    if let Some(path) = rounded_rect(x, y, w, h, radius) {
        pixmap.fill_path(&path, &solid(fill), FillRule::Winding, transform, None);
        let stroke = Stroke {
            width: 1.0,
            ..Stroke::default()
        };
        pixmap.stroke_path(&path, &solid(border), &stroke, transform, None);
    }
}

pub fn fill_rounded(
    pixmap: &mut Pixmap,
    scale: f32,
    rect: (f32, f32, f32, f32),
    radius: f32,
    color: Color,
) {
    let (x, y, w, h) = rect;
    if let Some(path) = rounded_rect(x, y, w, h, radius) {
        pixmap.fill_path(
            &path,
            &solid(color),
            FillRule::Winding,
            Transform::from_scale(scale, scale),
            None,
        );
    }
}

pub fn stroke_rounded(
    pixmap: &mut Pixmap,
    scale: f32,
    rect: (f32, f32, f32, f32),
    radius: f32,
    width: f32,
    color: Color,
) {
    let (x, y, w, h) = rect;
    if let Some(path) = rounded_rect(x, y, w, h, radius) {
        let stroke = Stroke {
            width,
            ..Stroke::default()
        };
        pixmap.stroke_path(
            &path,
            &solid(color),
            &stroke,
            Transform::from_scale(scale, scale),
            None,
        );
    }
}

pub fn circle(pixmap: &mut Pixmap, scale: f32, center: (f32, f32), radius: f32, color: Color) {
    if let Some(path) = PathBuilder::from_circle(center.0, center.1, radius) {
        pixmap.fill_path(
            &path,
            &solid(color),
            FillRule::Winding,
            Transform::from_scale(scale, scale),
            None,
        );
    }
}

pub fn ring(
    pixmap: &mut Pixmap,
    scale: f32,
    center: (f32, f32),
    radius: f32,
    width: f32,
    color: Color,
) {
    if let Some(path) = PathBuilder::from_circle(center.0, center.1, radius) {
        let stroke = Stroke {
            width,
            ..Stroke::default()
        };
        pixmap.stroke_path(
            &path,
            &solid(color),
            &stroke,
            Transform::from_scale(scale, scale),
            None,
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

/// Fills a whole edge strip with a glow that fades away from the screen edge.
pub fn edge_glow(pixmap: &mut Pixmap, edge: Edge, color: Color, alpha: f32) {
    let (w, h) = (pixmap.width() as f32, pixmap.height() as f32);
    let (start, end) = match edge {
        Edge::Top => (Point::from_xy(0.0, 0.0), Point::from_xy(0.0, h)),
        Edge::Bottom => (Point::from_xy(0.0, h), Point::from_xy(0.0, 0.0)),
        Edge::Left => (Point::from_xy(0.0, 0.0), Point::from_xy(w, 0.0)),
        Edge::Right => (Point::from_xy(w, 0.0), Point::from_xy(0.0, 0.0)),
    };
    let stops = vec![
        GradientStop::new(0.0, with_alpha(color, 0.55 * alpha)),
        GradientStop::new(0.35, with_alpha(color, 0.18 * alpha)),
        GradientStop::new(1.0, with_alpha(color, 0.0)),
    ];
    let Some(shader) =
        LinearGradient::new(start, end, stops, SpreadMode::Pad, Transform::identity())
    else {
        return;
    };
    let paint = Paint {
        shader,
        ..Paint::default()
    };
    if let Some(rect) = Rect::from_xywh(0.0, 0.0, w, h) {
        pixmap.fill_rect(rect, &paint, Transform::identity(), None);
    }
}

/// Copies a premultiplied RGBA pixmap into a little-endian ARGB8888 shm buffer.
pub fn to_argb8888(pixmap: &Pixmap, canvas: &mut [u8]) {
    let (src, _) = pixmap.data().as_chunks::<4>();
    let (dst, _) = canvas.as_chunks_mut::<4>();
    for (src, dst) in src.iter().zip(dst) {
        *dst = [src[2], src[1], src[0], src[3]];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrow_outline_sign() {
        assert!(sd_polygon((8.0, 6.0), &ARROW) < 0.0, "inside the arrow");
        assert!(sd_polygon((-5.0, -5.0), &ARROW) > 0.0, "beyond the tip");
        assert!(sd_polygon((16.0, 16.0), &ARROW) > 0.0, "in the notch");
    }

    #[test]
    fn sprite_edges_are_transparent() {
        let sprite = cursor_sprite(Color::WHITE, 1.5).expect("sprite");
        let w = sprite.width() as usize;
        let data = sprite.data();
        for i in 0..w {
            assert_eq!(data[i * 4 + 3], 0, "top edge");
            assert_eq!(data[((w - 1) * w + i) * 4 + 3], 0, "bottom edge");
        }
    }

    #[test]
    fn argb_conversion_swaps_red_and_blue() {
        let mut pixmap = Pixmap::new(1, 1).expect("pixmap");
        pixmap.fill(Color::from_rgba8(10, 20, 30, 255));
        let mut canvas = [0u8; 4];
        to_argb8888(&pixmap, &mut canvas);
        assert_eq!(canvas, [30, 20, 10, 255]);
    }
}

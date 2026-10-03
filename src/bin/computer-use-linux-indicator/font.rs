//! System UI font loading and single-line text rendering into a pixmap.

use ab_glyph::{Font as _, FontVec, PxScale, ScaleFont as _};
use tiny_skia::{Color, Pixmap};

/// Sans families tried in order before the generic fallback.
const FAMILIES: &[&str] = &[
    "Inter",
    "Open Sans",
    "Noto Sans",
    "Cantarell",
    "Ubuntu",
    "DejaVu Sans",
];

pub struct Fonts {
    pub regular: FontVec,
    pub semibold: FontVec,
}

impl Fonts {
    pub fn load() -> Option<Self> {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let load = |weight: u16| {
            let mut families: Vec<fontdb::Family<'_>> = FAMILIES
                .iter()
                .map(|name| fontdb::Family::Name(name))
                .collect();
            families.push(fontdb::Family::SansSerif);
            let id = db.query(&fontdb::Query {
                families: &families,
                weight: fontdb::Weight(weight),
                ..fontdb::Query::default()
            })?;
            db.with_face_data(id, |data, index| {
                FontVec::try_from_vec_and_index(data.to_vec(), index).ok()
            })
            .flatten()
        };
        let regular = load(400)?;
        let semibold = load(600).or_else(|| load(400))?;
        Some(Self { regular, semibold })
    }
}

/// Advance width of `text` at `size` pixels.
pub fn measure(font: &FontVec, size: f32, text: &str) -> f32 {
    let scaled = font.as_scaled(PxScale::from(size));
    let mut width = 0.0;
    let mut previous = None;
    for c in text.chars() {
        let id = scaled.glyph_id(c);
        if let Some(previous) = previous {
            width += scaled.kern(previous, id);
        }
        width += scaled.h_advance(id);
        previous = Some(id);
    }
    width
}

/// Vertical metrics: (ascent, height) at `size` pixels.
pub fn line_metrics(font: &FontVec, size: f32) -> (f32, f32) {
    let scaled = font.as_scaled(PxScale::from(size));
    (scaled.ascent(), scaled.ascent() - scaled.descent())
}

/// Draws `text` with its baseline at `baseline`, starting at `x`.
pub fn draw(
    pixmap: &mut Pixmap,
    font: &FontVec,
    size: f32,
    (x, baseline): (f32, f32),
    text: &str,
    color: Color,
) {
    let scale = PxScale::from(size);
    let scaled = font.as_scaled(scale);
    let (width, height) = (pixmap.width() as i32, pixmap.height() as i32);
    let premultiplied = color.premultiply();
    let pixels = pixmap.pixels_mut();
    let mut caret = x;
    let mut previous = None;
    for c in text.chars() {
        let id = scaled.glyph_id(c);
        if let Some(previous) = previous {
            caret += scaled.kern(previous, id);
        }
        let glyph = id.with_scale_and_position(scale, ab_glyph::point(caret, baseline));
        caret += scaled.h_advance(id);
        previous = Some(id);
        let Some(outlined) = font.outline_glyph(glyph) else {
            continue;
        };
        let bounds = outlined.px_bounds();
        outlined.draw(|gx, gy, coverage| {
            let px = bounds.min.x as i32 + gx as i32;
            let py = bounds.min.y as i32 + gy as i32;
            if px < 0 || py < 0 || px >= width || py >= height {
                return;
            }
            let coverage = coverage.clamp(0.0, 1.0);
            let pixel = &mut pixels[(py * width + px) as usize];
            let keep = 1.0 - premultiplied.alpha() * coverage;
            let blend = |src: f32, dst: u8| {
                (src * 255.0 * coverage + f32::from(dst) * keep)
                    .round()
                    .clamp(0.0, 255.0) as u8
            };
            let alpha = blend(premultiplied.alpha(), pixel.alpha());
            // Rounding must not push a colour channel above alpha.
            let channel = |src: f32, dst: u8| blend(src, dst).min(alpha);
            if let Some(blended) = tiny_skia::PremultipliedColorU8::from_rgba(
                channel(premultiplied.red(), pixel.red()),
                channel(premultiplied.green(), pixel.green()),
                channel(premultiplied.blue(), pixel.blue()),
                alpha,
            ) {
                *pixel = blended;
            }
        });
    }
}

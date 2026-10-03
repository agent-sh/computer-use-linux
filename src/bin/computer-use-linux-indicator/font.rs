//! System UI font lookup through fontconfig, and single-line text drawn as
//! glyph outlines.

use std::process::Command;

use skrifa::{
    instance::{LocationRef, Size},
    outline::{DrawSettings, OutlinePen},
    FontRef, GlyphId, MetadataProvider,
};
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Transform};

#[derive(Clone)]
pub struct Face {
    data: Vec<u8>,
    index: u32,
}

impl Face {
    fn font(&self) -> Option<FontRef<'_>> {
        FontRef::from_index(&self.data, self.index).ok()
    }
}

pub struct Fonts {
    pub regular: Face,
    pub semibold: Face,
}

impl Fonts {
    /// Resolves the desktop's sans-serif faces; `None` without fontconfig.
    pub fn load() -> Option<Self> {
        let regular = lookup("sans-serif:weight=80")?;
        let semibold = lookup("sans-serif:weight=180").unwrap_or_else(|| regular.clone());
        Some(Self { regular, semibold })
    }
}

fn lookup(pattern: &str) -> Option<Face> {
    let output = Command::new("fc-match")
        .args(["--format=%{file}|%{index}", pattern])
        .output()
        .ok()?;
    let line = String::from_utf8(output.stdout).ok()?;
    let (file, index) = line.trim().rsplit_once('|')?;
    let face = Face {
        data: std::fs::read(file).ok()?,
        index: index.parse().unwrap_or(0),
    };
    face.font()?;
    Some(face)
}

fn glyph(font: &FontRef<'_>, c: char) -> GlyphId {
    font.charmap().map(c).unwrap_or(GlyphId::NOTDEF)
}

/// Advance width of `text` at `size` pixels.
pub fn measure(face: &Face, size: f32, text: &str) -> f32 {
    let Some(font) = face.font() else {
        return 0.0;
    };
    let metrics = font.glyph_metrics(Size::new(size), LocationRef::default());
    text.chars()
        .filter_map(|c| metrics.advance_width(glyph(&font, c)))
        .sum()
}

/// Vertical metrics: (ascent, height) at `size` pixels.
pub fn line_metrics(face: &Face, size: f32) -> (f32, f32) {
    let Some(font) = face.font() else {
        return (size, size * 1.3);
    };
    let metrics = font.metrics(Size::new(size), LocationRef::default());
    (metrics.ascent, metrics.ascent - metrics.descent)
}

/// Collects glyph outlines into one path, flipping font y-up to screen y-down.
struct Pen {
    path: PathBuilder,
    origin: (f32, f32),
}

impl Pen {
    fn at(&self, x: f32, y: f32) -> (f32, f32) {
        (self.origin.0 + x, self.origin.1 - y)
    }
}

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.at(x, y);
        self.path.move_to(x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.at(x, y);
        self.path.line_to(x, y);
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        let (cx0, cy0) = self.at(cx0, cy0);
        let (x, y) = self.at(x, y);
        self.path.quad_to(cx0, cy0, x, y);
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let (cx0, cy0) = self.at(cx0, cy0);
        let (cx1, cy1) = self.at(cx1, cy1);
        let (x, y) = self.at(x, y);
        self.path.cubic_to(cx0, cy0, cx1, cy1, x, y);
    }

    fn close(&mut self) {
        self.path.close();
    }
}

/// Draws `text` with its baseline at `baseline`, starting at `x`.
pub fn draw(
    pixmap: &mut Pixmap,
    face: &Face,
    size: f32,
    (x, baseline): (f32, f32),
    text: &str,
    color: Color,
) {
    let Some(font) = face.font() else {
        return;
    };
    let location = LocationRef::default();
    let metrics = font.glyph_metrics(Size::new(size), location);
    let outlines = font.outline_glyphs();
    let mut pen = Pen {
        path: PathBuilder::new(),
        origin: (x, baseline),
    };
    for c in text.chars() {
        let id = glyph(&font, c);
        if let Some(outline) = outlines.get(id) {
            let _ = outline.draw(DrawSettings::unhinted(Size::new(size), location), &mut pen);
        }
        pen.origin.0 += metrics.advance_width(id).unwrap_or(0.0);
    }
    let Some(path) = pen.path.finish() else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    pixmap.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );
}

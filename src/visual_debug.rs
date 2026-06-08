use crate::atspi_tree::{AccessibilityNode, Bounds};
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use image::{Rgba, RgbaImage};
use schemars::JsonSchema;
use serde::Serialize;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct OcrRegion {
    pub text: String,
    pub confidence: Option<f32>,
    pub bounds: Bounds,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct HighlightedScreenshot {
    pub mime_type: String,
    pub data_url: String,
    pub highlighted_count: usize,
    pub width: u32,
    pub height: u32,
}

pub fn highlight_element_refs(
    png_bytes: &[u8],
    nodes: &[AccessibilityNode],
    max_labels: usize,
) -> Result<HighlightedScreenshot> {
    let mut image = image::load_from_memory(png_bytes)
        .context("failed to decode screenshot for highlighting")?
        .to_rgba8();
    let (width, height) = image.dimensions();
    let mut highlighted_count = 0usize;

    for node in nodes.iter().take(max_labels) {
        let Some(bounds) = node.bounds.as_ref() else {
            continue;
        };
        if bounds.width <= 0 || bounds.height <= 0 {
            continue;
        }
        draw_hollow_rect(
            &mut image,
            bounds.x,
            bounds.y,
            bounds.width as u32,
            bounds.height as u32,
            Rgba([255, 64, 64, 220]),
        );
        highlighted_count += 1;
    }

    let mut encoded = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut encoded),
            image::ImageFormat::Png,
        )
        .context("failed to encode highlighted screenshot")?;
    let data_url = format!("data:image/png;base64,{}", STANDARD.encode(&encoded));
    Ok(HighlightedScreenshot {
        mime_type: "image/png".to_string(),
        data_url,
        highlighted_count,
        width,
        height,
    })
}

fn draw_hollow_rect(image: &mut RgbaImage, x: i32, y: i32, width: u32, height: u32, color: Rgba<u8>) {
    let image_width = image.width() as i32;
    let image_height = image.height() as i32;
    let left = x.max(0);
    let top = y.max(0);
    let right = (x + width as i32).min(image_width);
    let bottom = (y + height as i32).min(image_height);
    if left >= right || top >= bottom {
        return;
    }
    for px in left..right {
        if top < image_height {
            image.put_pixel(px as u32, top as u32, color);
        }
        if bottom - 1 < image_height {
            image.put_pixel(px as u32, (bottom - 1) as u32, color);
        }
    }
    for py in top..bottom {
        if left < image_width {
            image.put_pixel(left as u32, py as u32, color);
        }
        if right - 1 < image_width {
            image.put_pixel((right - 1) as u32, py as u32, color);
        }
    }
}

pub fn ocr_png_regions(png_bytes: &[u8]) -> Result<Vec<OcrRegion>> {
    let tesseract = Command::new("tesseract")
        .arg("stdin")
        .arg("stdout")
        .arg("-l")
        .arg("eng")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let Ok(mut child) = tesseract else {
        bail!("tesseract is not installed; install tesseract-ocr for OCR support");
    };
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin
            .write_all(png_bytes)
            .context("failed to write screenshot to tesseract stdin")?;
    }
    let output = child
        .wait_with_output()
        .context("failed waiting for tesseract")?;
    if !output.status.success() {
        bail!(
            "tesseract failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        return Ok(Vec::new());
    }
    Ok(vec![OcrRegion {
        text,
        confidence: None,
        bounds: Bounds {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        },
    }])
}
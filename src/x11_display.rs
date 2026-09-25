//! Minimal X11 protocol client for native X11 sessions.
//!
//! The EWMH window backend lists windows with `wmctrl -lG`, but wmctrl's x/y
//! are not the window origin: it translates the client's offset inside its
//! parent a second time (Red Hat bug 654888, closed WONTFIX). Under a
//! reparenting WM that adds the frame offset twice; under a non-reparenting WM
//! it doubles the absolute position. This module asks the X server directly.
//!
//! It also captures the root window for the native X11 screenshot route:
//! `GetImage` returns device pixels, the same space xdotool/XTEST and these
//! window origins use, with no toolkit scaling layer (issue #155).
//!
//! Callers must gate on [`is_native_x11_session`] first, so XWayland under a
//! Wayland compositor is never used.

use anyhow::{anyhow, bail, Context, Result};
use std::env;
use std::time::Duration;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt as _, ImageFormat, ImageOrder, VisualClass, Window,
};
use x11rb::rust_connection::RustConnection;

/// Bound on one X11 query, including the connection handshake.
pub(crate) const X11_QUERY_TIMEOUT: Duration = Duration::from_secs(2);
/// Root-window capture moves tens of MB on large screens; allow more time.
pub(crate) const X11_CAPTURE_TIMEOUT: Duration = Duration::from_secs(5);

/// True when this looks like a plain X11 session.
///
/// Requires an X `DISPLAY` and either an explicit `x11` session type or the
/// absence of a Wayland display, so we never hijack XWayland under a Wayland
/// compositor (where a native backend should answer instead).
pub(crate) fn is_native_x11_session() -> bool {
    if env_nonempty("DISPLAY").is_none() {
        return false;
    }
    match env_nonempty("XDG_SESSION_TYPE").as_deref() {
        Some("x11") => true,
        Some("wayland") => false,
        _ => env_nonempty("WAYLAND_DISPLAY").is_none(),
    }
}

fn env_nonempty(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Run `f` against a fresh X connection off the async runtime, bounded by
/// `limit` so a wedged X server cannot hang a tool call.
pub(crate) async fn with_x11_display<T, F>(limit: Duration, f: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(&X11Display) -> T + Send + 'static,
{
    let task =
        tokio::task::spawn_blocking(move || X11Display::connect().map(|display| f(&display)));
    match tokio::time::timeout(limit, task).await {
        Ok(Ok(result)) => result,
        Ok(Err(join_error)) => Err(anyhow!("X11 query task failed: {join_error}")),
        Err(_) => bail!("X server did not answer within {limit:?}"),
    }
}

pub(crate) struct X11Display {
    conn: RustConnection,
    root: Window,
    screen: usize,
}

/// Root-window pixels as tightly packed RGB8.
pub(crate) struct RootImage {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

/// `_NET_FRAME_EXTENTS`: decoration widths the WM adds around the client area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FrameExtents {
    pub left: u32,
    pub right: u32,
    pub top: u32,
    pub bottom: u32,
}

impl X11Display {
    /// Connect to the display named by `DISPLAY`, authenticating with
    /// `XAUTHORITY` or `~/.Xauthority` the same way Xlib clients do.
    pub(crate) fn connect() -> Result<Self> {
        let (conn, screen) =
            x11rb::connect(None).context("failed to connect to the X server named by DISPLAY")?;
        let root = conn
            .setup()
            .roots
            .get(screen)
            .context("X server reported no screen for DISPLAY")?
            .root;
        Ok(Self { conn, root, screen })
    }

    /// `DISPLAY` screen size and root depth, for doctor.
    pub(crate) fn describe(&self) -> String {
        let screen = &self.conn.setup().roots[self.screen];
        format!(
            "native X11 root window {}x{}, depth {}",
            screen.width_in_pixels, screen.height_in_pixels, screen.root_depth
        )
    }

    /// Capture the whole root window with one `GetImage` request.
    pub(crate) fn capture_root(&self) -> Result<RootImage> {
        let setup = self.conn.setup();
        let screen = &setup.roots[self.screen];
        let (width, height) = (screen.width_in_pixels, screen.height_in_pixels);
        let reply = self
            .conn
            .get_image(ImageFormat::Z_PIXMAP, self.root, 0, 0, width, height, !0)?
            .reply()
            .context("X server refused GetImage on the root window")?;
        let visual = screen
            .allowed_depths
            .iter()
            .flat_map(|depth| &depth.visuals)
            .find(|visual| visual.visual_id == reply.visual)
            .with_context(|| format!("root image visual 0x{:x} is not listed", reply.visual))?;
        if !matches!(
            visual.class,
            VisualClass::TRUE_COLOR | VisualClass::DIRECT_COLOR
        ) {
            bail!(
                "root visual class {:?} is not TrueColor or DirectColor; palette-based displays are not supported",
                visual.class
            );
        }
        let format = setup
            .pixmap_formats
            .iter()
            .find(|format| format.depth == reply.depth)
            .with_context(|| format!("no pixmap format for depth {}", reply.depth))?;
        let layout = ZPixmapLayout {
            bits_per_pixel: u32::from(format.bits_per_pixel),
            scanline_pad: u32::from(format.scanline_pad),
            msb_first: setup.image_byte_order == ImageOrder::MSB_FIRST,
            red_mask: visual.red_mask,
            green_mask: visual.green_mask,
            blue_mask: visual.blue_mask,
        };
        let rgb = zpixmap_to_rgb(&reply.data, u32::from(width), u32::from(height), &layout)?;
        Ok(RootImage {
            width: u32::from(width),
            height: u32::from(height),
            rgb,
        })
    }

    /// Absolute root-window origin of each window's client area, the value
    /// `xwininfo` prints as "Absolute upper-left". Requests are pipelined, so
    /// this costs one round trip for the whole list. `None` marks a window that
    /// vanished, is invalid, or sits on another screen.
    pub(crate) fn client_origins(&self, windows: &[Window]) -> Vec<Option<(i32, i32)>> {
        let cookies = windows
            .iter()
            .map(|&window| {
                self.conn
                    .translate_coordinates(window, self.root, 0, 0)
                    .ok()
            })
            .collect::<Vec<_>>();
        cookies
            .into_iter()
            .map(|cookie| {
                let reply = cookie?.reply().ok()?;
                reply
                    .same_screen
                    .then(|| (i32::from(reply.dst_x), i32::from(reply.dst_y)))
            })
            .collect()
    }

    /// The WM's `_NET_FRAME_EXTENTS` for `window`, or `None` when the WM does
    /// not publish it.
    pub(crate) fn frame_extents(&self, window: Window) -> Result<Option<FrameExtents>> {
        let atom = self
            .conn
            .intern_atom(true, b"_NET_FRAME_EXTENTS")?
            .reply()
            .context("failed to intern _NET_FRAME_EXTENTS")?
            .atom;
        if atom == x11rb::NONE {
            return Ok(None);
        }
        let reply = self
            .conn
            .get_property(false, window, atom, AtomEnum::CARDINAL, 0, 4)?
            .reply()
            .context("failed to read _NET_FRAME_EXTENTS")?;
        Ok(parse_frame_extents(
            reply.value32().map(|values| values.collect::<Vec<_>>()),
        ))
    }
}

fn parse_frame_extents(values: Option<Vec<u32>>) -> Option<FrameExtents> {
    match values.as_deref() {
        Some(&[left, right, top, bottom]) => Some(FrameExtents {
            left,
            right,
            top,
            bottom,
        }),
        _ => None,
    }
}

/// Frame (outer) origin from a client origin and the WM's extents. With the
/// default NorthWest gravity this is the point `wmctrl -e 0,x,y,...` places.
pub(crate) fn frame_origin(client_origin: (i32, i32), extents: FrameExtents) -> (i32, i32) {
    (
        client_origin
            .0
            .saturating_sub(i32::try_from(extents.left).unwrap_or(i32::MAX)),
        client_origin
            .1
            .saturating_sub(i32::try_from(extents.top).unwrap_or(i32::MAX)),
    )
}

/// How the server packs one ZPixmap scanline, from the connection setup and
/// the image's visual.
#[derive(Clone, Copy, Debug)]
struct ZPixmapLayout {
    bits_per_pixel: u32,
    scanline_pad: u32,
    msb_first: bool,
    red_mask: u32,
    green_mask: u32,
    blue_mask: u32,
}

fn zpixmap_to_rgb(data: &[u8], width: u32, height: u32, layout: &ZPixmapLayout) -> Result<Vec<u8>> {
    let bytes_per_pixel = match layout.bits_per_pixel {
        16 | 24 | 32 => (layout.bits_per_pixel / 8) as usize,
        other => bail!("{other}-bit ZPixmap pixels are not supported"),
    };
    let pad = layout.scanline_pad.max(8);
    let stride = ((width * layout.bits_per_pixel).div_ceil(pad) * pad / 8) as usize;
    let (width, height) = (width as usize, height as usize);
    let needed = stride
        .checked_mul(height)
        .context("root image size overflowed")?;
    if data.len() < needed {
        bail!(
            "GetImage returned {} bytes; {width}x{height} at stride {stride} needs {needed}",
            data.len()
        );
    }
    let mut rgb = Vec::with_capacity(width * height * 3);
    for row in data[..needed].chunks_exact(stride) {
        for pixel in row[..width * bytes_per_pixel].chunks_exact(bytes_per_pixel) {
            let value = pixel.iter().enumerate().fold(0_u32, |acc, (index, &byte)| {
                let shift = if layout.msb_first {
                    8 * (bytes_per_pixel - 1 - index)
                } else {
                    8 * index
                };
                acc | (u32::from(byte) << shift)
            });
            rgb.push(channel_to_u8(value, layout.red_mask));
            rgb.push(channel_to_u8(value, layout.green_mask));
            rgb.push(channel_to_u8(value, layout.blue_mask));
        }
    }
    Ok(rgb)
}

/// Extract the channel selected by `mask` and scale it to 8 bits.
fn channel_to_u8(pixel: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let bits = (mask >> shift).count_ones();
    let value = (pixel & mask) >> shift;
    if bits >= 8 {
        (value >> (bits - 8)) as u8
    } else {
        let max = (1_u32 << bits) - 1;
        ((value * 255 + max / 2) / max) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BGRX: ZPixmapLayout = ZPixmapLayout {
        bits_per_pixel: 32,
        scanline_pad: 32,
        msb_first: false,
        red_mask: 0x00ff_0000,
        green_mask: 0x0000_ff00,
        blue_mask: 0x0000_00ff,
    };

    #[test]
    fn depth24_lsb_first_is_bgrx_bytes() {
        // Two pixels: pure red, then 0x123456. Bytes are B,G,R,X per pixel.
        let data = [0x00, 0x00, 0xff, 0x00, 0x56, 0x34, 0x12, 0xaa];
        assert_eq!(
            zpixmap_to_rgb(&data, 2, 1, &BGRX).unwrap(),
            vec![0xff, 0x00, 0x00, 0x12, 0x34, 0x56]
        );
    }

    #[test]
    fn msb_first_reads_bytes_big_endian() {
        let layout = ZPixmapLayout {
            msb_first: true,
            ..BGRX
        };
        let data = [0x00, 0x12, 0x34, 0x56];
        assert_eq!(
            zpixmap_to_rgb(&data, 1, 1, &layout).unwrap(),
            vec![0x12, 0x34, 0x56]
        );
    }

    #[test]
    fn rgb565_scales_five_and_six_bit_channels_to_full_range() {
        let layout = ZPixmapLayout {
            bits_per_pixel: 16,
            scanline_pad: 32,
            msb_first: false,
            red_mask: 0xf800,
            green_mask: 0x07e0,
            blue_mask: 0x001f,
        };
        // white 0xffff, black 0x0000; width 2 at 16 bpp is already 32-bit aligned.
        let data = [0xff, 0xff, 0x00, 0x00];
        assert_eq!(
            zpixmap_to_rgb(&data, 2, 1, &layout).unwrap(),
            vec![255, 255, 255, 0, 0, 0]
        );
    }

    #[test]
    fn scanline_padding_is_skipped_between_rows() {
        let layout = ZPixmapLayout {
            bits_per_pixel: 16,
            scanline_pad: 32,
            msb_first: false,
            red_mask: 0xf800,
            green_mask: 0x07e0,
            blue_mask: 0x001f,
        };
        // Width 1 at 16 bpp pads each row to 4 bytes; the pad bytes are junk.
        let data = [0xff, 0xff, 0xee, 0xee, 0x00, 0x00, 0xee, 0xee];
        assert_eq!(
            zpixmap_to_rgb(&data, 1, 2, &layout).unwrap(),
            vec![255, 255, 255, 0, 0, 0]
        );
    }

    #[test]
    fn ten_bit_channels_keep_their_top_eight_bits() {
        let layout = ZPixmapLayout {
            red_mask: 0x3ff0_0000,
            green_mask: 0x000f_fc00,
            blue_mask: 0x0000_03ff,
            ..BGRX
        };
        let pixel: u32 = (0x3ff << 20) | (0x200 << 10) | 0x001;
        assert_eq!(
            zpixmap_to_rgb(&pixel.to_le_bytes(), 1, 1, &layout).unwrap(),
            vec![0xff, 0x80, 0x00]
        );
    }

    #[test]
    fn short_or_unsupported_images_are_errors() {
        assert!(zpixmap_to_rgb(&[0; 7], 2, 1, &BGRX).is_err());
        let one_bit = ZPixmapLayout {
            bits_per_pixel: 1,
            ..BGRX
        };
        assert!(zpixmap_to_rgb(&[0; 4], 1, 1, &one_bit).is_err());
    }

    #[test]
    fn frame_extents_need_exactly_four_cardinals() {
        assert_eq!(
            parse_frame_extents(Some(vec![1, 1, 22, 5])),
            Some(FrameExtents {
                left: 1,
                right: 1,
                top: 22,
                bottom: 5
            })
        );
        assert_eq!(parse_frame_extents(None), None);
        assert_eq!(parse_frame_extents(Some(vec![1, 1, 22])), None);
        assert_eq!(parse_frame_extents(Some(vec![])), None);
    }

    #[test]
    fn frame_origin_subtracts_left_and_top_extents() {
        // openbox measurement: client at 301,222 with extents 1,1,22,5 was
        // placed by `wmctrl -e 0,300,200,...`.
        let extents = FrameExtents {
            left: 1,
            right: 1,
            top: 22,
            bottom: 5,
        };
        assert_eq!(frame_origin((301, 222), extents), (300, 200));
        assert_eq!(
            frame_origin((0, 0), FrameExtents::default()),
            (0, 0),
            "an undecorated window's frame is its client area"
        );
    }
}

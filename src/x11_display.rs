//! Minimal X11 protocol client for native X11 sessions.
//!
//! The EWMH window backend lists windows with `wmctrl -lG`, but wmctrl's x/y
//! are not the window origin: it translates the client's offset inside its
//! parent a second time (Red Hat bug 654888, closed WONTFIX). Under a
//! reparenting WM that adds the frame offset twice; under a non-reparenting WM
//! it doubles the absolute position. This module asks the X server directly.
//!
//! Callers must gate on a native X11 session first (never XWayland).

use anyhow::{Context, Result};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, Window};
use x11rb::rust_connection::RustConnection;

pub(crate) struct X11Display {
    conn: RustConnection,
    root: Window,
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
        Ok(Self { conn, root })
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

#[cfg(test)]
mod tests {
    use super::*;

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

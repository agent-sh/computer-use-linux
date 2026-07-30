use crate::terminal::enrich_terminal_windows;
use crate::windowing::registry::BackendProbe;
use crate::windowing::types::{WindowBounds, WindowInfo};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::process::Command;

pub const NIRI_BACKEND: &str = "niri";

/// Probe the running niri compositor by asking it for its window list.
///
/// `niri msg --json windows` only succeeds when niri is the active
/// compositor for this session (it auto-detects its socket from
/// `WAYLAND_DISPLAY` + `XDG_RUNTIME_DIR`), so a failed probe cleanly
/// disables the backend on non-niri sessions.
pub fn probe() -> BackendProbe {
    match niri_msg(&["--json", "windows"]) {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let ok = matches!(
                serde_json::from_str::<serde_json::Value>(&stdout),
                Ok(serde_json::Value::Array(_))
            );
            BackendProbe {
                id: NIRI_BACKEND,
                ok,
                can_list_windows: ok,
                can_focus_apps: ok,
                can_focus_windows: ok,
                detail: if ok {
                    "niri msg --json windows returned a JSON array".to_string()
                } else {
                    "niri msg --json windows did not return a JSON array".to_string()
                },
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            BackendProbe {
                id: NIRI_BACKEND,
                ok: false,
                can_list_windows: false,
                can_focus_apps: false,
                can_focus_windows: false,
                detail: if stderr.is_empty() { stdout } else { stderr },
            }
        }
        Err(error) => BackendProbe {
            id: NIRI_BACKEND,
            ok: false,
            can_list_windows: false,
            can_focus_apps: false,
            can_focus_windows: false,
            detail: error.to_string(),
        },
    }
}

pub fn list_windows() -> Result<Vec<WindowInfo>> {
    let output = niri_msg(&["--json", "windows"])
        .context("failed to run niri msg --json windows")?;
    if !output.status.success() {
        bail!(
            "niri msg --json windows failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let windows_json = String::from_utf8_lossy(&output.stdout);
    parse_niri_windows(&windows_json)
}

pub(crate) fn parse_niri_windows(json: &str) -> Result<Vec<WindowInfo>> {
    let niri_windows: Vec<NiriWindow> =
        serde_json::from_str(json).context("failed to parse niri msg --json windows output")?;
    let mut windows = niri_windows
        .into_iter()
        .map(WindowInfo::try_from)
        .collect::<Result<Vec<_>>>()?;
    windows.sort_by_key(|window| window.window_id);
    enrich_terminal_windows(&mut windows);
    Ok(windows)
}

pub fn activate_window(window_id: u64) -> Result<()> {
    let id_arg = window_id.to_string();
    let output = niri_msg(&["action", "focus-window", "--id", &id_arg])
        .with_context(|| format!("failed to run niri msg action focus-window --id {window_id}"))?;
    if output.status.success() {
        Ok(())
    } else {
        bail!(
            "niri focus-window failed for id {window_id}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
}

fn niri_msg(args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new("niri").arg("msg").args(args).output()
}

#[derive(Debug, Deserialize)]
struct NiriWindow {
    id: u64,
    title: Option<String>,
    app_id: Option<String>,
    pid: Option<i64>,
    workspace_id: Option<i64>,
    is_focused: Option<bool>,
    layout: Option<NiriLayout>,
}

#[derive(Debug, Deserialize)]
struct NiriLayout {
    /// `[width, height]` in logical pixels. niri emits these as integers
    /// but the schema permits floats, so parse as `f64` and round.
    window_size: Option<[f64; 2]>,
    /// Absolute position of the tile within the workspace view, in logical
    /// pixels. `null` when niri does not expose a position for the window
    /// (e.g. windows on workspaces that are not currently rendered).
    tile_pos_in_workspace_view: Option<[f64; 2]>,
}

impl TryFrom<NiriWindow> for WindowInfo {
    type Error = anyhow::Error;

    fn try_from(window: NiriWindow) -> Result<Self> {
        let bounds = window.layout.and_then(|layout| {
            let window_size = layout.window_size?;
            let (width, height) = (window_size[0], window_size[1]);
            if width <= 0.0 || height <= 0.0 {
                return None;
            }
            let (x, y) = layout
                .tile_pos_in_workspace_view
                .map(|[x, y]| (Some(x.round() as i32), Some(y.round() as i32)))
                .unwrap_or((None, None));
            Some(WindowBounds {
                x,
                y,
                width: width.round() as u32,
                height: height.round() as u32,
            })
        });

        let app_id = window.app_id;
        Ok(WindowInfo {
            window_id: window.id,
            title: window.title,
            app_id: app_id.clone(),
            wm_class: app_id,
            pid: window.pid.and_then(|pid| u32::try_from(pid).ok()),
            bounds,
            workspace: window.workspace_id.and_then(|id| i32::try_from(id).ok()),
            focused: window.is_focused.unwrap_or(false),
            hidden: false,
            client_type: Some("wayland".to_string()),
            backend: NIRI_BACKEND.to_string(),
            terminal: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_tiled_window_with_size_but_no_absolute_position() {
        let json = r#"[{
            "id": 16,
            "title": "~",
            "app_id": "kitty",
            "pid": 2872402,
            "workspace_id": 2,
            "is_focused": false,
            "is_floating": false,
            "is_urgent": false,
            "layout": {
                "pos_in_scrolling_layout": [1, 1],
                "tile_size": [945.0, 1024.0],
                "window_size": [945, 1024],
                "tile_pos_in_workspace_view": null,
                "window_offset_in_tile": [0.0, 0.0]
            },
            "focus_timestamp": {"secs": 172427, "nanos": 771495935}
        }]"#;
        let windows = parse_niri_windows(json).unwrap();
        let window = &windows[0];
        assert_eq!(window.window_id, 16);
        assert_eq!(window.app_id.as_deref(), Some("kitty"));
        assert_eq!(window.wm_class.as_deref(), Some("kitty"));
        assert_eq!(window.pid, Some(2872402));
        assert_eq!(window.workspace, Some(2));
        assert!(!window.focused);
        assert_eq!(window.client_type.as_deref(), Some("wayland"));
        assert_eq!(window.backend, NIRI_BACKEND);
        let bounds = window.bounds.as_ref().unwrap();
        assert_eq!((bounds.x, bounds.y), (None, None));
        assert_eq!((bounds.width, bounds.height), (945, 1024));
    }

    #[test]
    fn parses_absolute_position_when_niri_exposes_it() {
        let json = r#"[{
            "id": 4,
            "title": "Term",
            "app_id": "kitty",
            "pid": 5024,
            "workspace_id": 1,
            "is_focused": true,
            "is_floating": false,
            "is_urgent": false,
            "layout": {
                "pos_in_scrolling_layout": [2, 1],
                "tile_size": [1517.0, 1024.0],
                "window_size": [1517, 1024],
                "tile_pos_in_workspace_view": [3747.0, 0.0],
                "window_offset_in_tile": [0.0, 0.0]
            },
            "focus_timestamp": {"secs": 177316, "nanos": 240211240}
        }]"#;
        let windows = parse_niri_windows(json).unwrap();
        let window = &windows[0];
        assert!(window.focused);
        let bounds = window.bounds.as_ref().unwrap();
        assert_eq!((bounds.x, bounds.y), (Some(3747), Some(0)));
        assert_eq!((bounds.width, bounds.height), (1517, 1024));
    }

    #[test]
    fn omits_bounds_when_window_size_is_missing() {
        let json = r#"[{
            "id": 7,
            "title": "Signal",
            "app_id": "signal",
            "pid": 10164,
            "workspace_id": 3,
            "is_focused": false,
            "is_floating": false,
            "is_urgent": false,
            "layout": {
                "pos_in_scrolling_layout": [1, 1],
                "tile_size": [1920.0, 1044.0],
                "window_size": null,
                "tile_pos_in_workspace_view": null,
                "window_offset_in_tile": [0.0, 0.0]
            }
        }]"#;
        let windows = parse_niri_windows(json).unwrap();
        assert!(windows[0].bounds.is_none());
    }
}
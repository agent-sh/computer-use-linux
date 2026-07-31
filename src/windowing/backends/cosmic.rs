use crate::cosmic_helper;
use crate::terminal::enrich_terminal_windows;
use crate::windowing::registry::BackendProbe;
use crate::windowing::types::WindowInfo;
use anyhow::{bail, Context, Result};

pub const COSMIC_WAYLAND_BACKEND: &str = "cosmic-wayland";

pub fn probe() -> BackendProbe {
    match cosmic_helper::probe() {
        Ok(probe) => BackendProbe {
            id: COSMIC_WAYLAND_BACKEND,
            ok: probe.ok,
            can_list_windows: probe.can_list_windows,
            can_focus_apps: probe.can_activate_windows,
            can_focus_windows: probe.can_activate_windows,
            detail: probe.detail,
        },
        Err(error) => BackendProbe {
            id: COSMIC_WAYLAND_BACKEND,
            ok: false,
            can_list_windows: false,
            can_focus_apps: false,
            can_focus_windows: false,
            detail: error.to_string(),
        },
    }
}

pub async fn list_windows() -> Result<Vec<WindowInfo>> {
    let json = cosmic_helper::list_windows_json().await?;
    let mut windows: Vec<WindowInfo> =
        serde_json::from_str(&json).context("COSMIC helper returned invalid list-windows JSON")?;
    for window in &mut windows {
        window.backend = COSMIC_WAYLAND_BACKEND.to_string();
    }
    windows.sort_by_key(|window| window.window_id);
    enrich_terminal_windows(&mut windows);
    Ok(windows)
}

pub async fn focused_window() -> Result<Option<WindowInfo>> {
    let json = cosmic_helper::focused_window_json().await?;
    let mut window: Option<WindowInfo> = serde_json::from_str(&json)
        .context("COSMIC helper returned invalid focused-window JSON")?;
    if let Some(window) = window.as_mut() {
        window.backend = COSMIC_WAYLAND_BACKEND.to_string();
    }
    Ok(window)
}

pub async fn activate_window(window_id: u64) -> Result<()> {
    let activation = cosmic_helper::activate_window(window_id).await?;
    if activation.ok {
        Ok(())
    } else {
        bail!("COSMIC helper refused activation: {}", activation.detail);
    }
}

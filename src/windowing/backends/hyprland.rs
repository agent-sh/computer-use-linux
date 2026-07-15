use crate::terminal::enrich_terminal_windows;
use crate::windowing::registry::BackendProbe;
use crate::windowing::types::{WindowBounds, WindowInfo};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::fs;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

pub const HYPRLAND_BACKEND: &str = "hyprland";

pub fn probe() -> BackendProbe {
    match hyprctl_output(&["clients", "-j"]) {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let ok = matches!(
                serde_json::from_str::<serde_json::Value>(&stdout),
                Ok(serde_json::Value::Array(_))
            );
            BackendProbe {
                id: HYPRLAND_BACKEND,
                ok,
                can_list_windows: ok,
                can_focus_apps: ok,
                can_focus_windows: ok,
                detail: if ok {
                    "hyprctl clients -j returned a JSON array".to_string()
                } else {
                    "hyprctl clients -j did not return a JSON array".to_string()
                },
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            BackendProbe {
                id: HYPRLAND_BACKEND,
                ok: false,
                can_list_windows: false,
                can_focus_apps: false,
                can_focus_windows: false,
                detail: if stderr.is_empty() { stdout } else { stderr },
            }
        }
        Err(error) => BackendProbe {
            id: HYPRLAND_BACKEND,
            ok: false,
            can_list_windows: false,
            can_focus_apps: false,
            can_focus_windows: false,
            detail: error.to_string(),
        },
    }
}

pub fn list_windows() -> Result<Vec<WindowInfo>> {
    let output = hyprctl_output(&["clients", "-j"]).context("failed to run hyprctl clients -j")?;
    if !output.status.success() {
        bail!(
            "hyprctl clients -j failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let clients_json = String::from_utf8_lossy(&output.stdout);
    let monitors_output = hyprctl_output(&["monitors", "-j"]).ok();
    match monitors_output.filter(|output| output.status.success()) {
        Some(monitors) => parse_hyprland_clients_with_monitors(&clients_json, &monitors.stdout),
        None => parse_hyprland_clients(&clients_json),
    }
}

fn parse_hyprland_clients_with_monitors(
    clients_json: &str,
    monitors_json: &[u8],
) -> Result<Vec<WindowInfo>> {
    let monitors: Vec<HyprlandMonitor> = serde_json::from_slice(monitors_json)
        .context("failed to parse hyprctl monitors -j output")?;
    let monitors = monitors
        .into_iter()
        .map(|monitor| (monitor.id, monitor))
        .collect::<std::collections::HashMap<_, _>>();
    let mut clients: Vec<HyprlandClient> =
        serde_json::from_str(clients_json).context("failed to parse hyprctl clients -j output")?;
    for client in &mut clients {
        let Some(monitor) = client.monitor.and_then(|id| monitors.get(&id)) else {
            continue;
        };
        if let Some(at) = client.at.as_mut() {
            at[0] = scale_i32(at[0] - monitor.x, monitor.scale);
            at[1] = scale_i32(at[1] - monitor.y, monitor.scale);
        }
        if let Some(size) = client.size.as_mut() {
            size[0] = scale_u32(size[0], monitor.scale);
            size[1] = scale_u32(size[1], monitor.scale);
        }
    }
    windows_from_hyprland_clients(clients)
}

pub(crate) fn parse_hyprland_clients(json: &str) -> Result<Vec<WindowInfo>> {
    let clients: Vec<HyprlandClient> =
        serde_json::from_str(json).context("failed to parse hyprctl clients -j output")?;
    windows_from_hyprland_clients(clients)
}

fn scale_i32(value: i32, scale: f64) -> i32 {
    (f64::from(value) * scale).round() as i32
}

fn scale_u32(value: u32, scale: f64) -> u32 {
    (f64::from(value) * scale).round() as u32
}

fn windows_from_hyprland_clients(clients: Vec<HyprlandClient>) -> Result<Vec<WindowInfo>> {
    let mut windows = clients
        .into_iter()
        .filter(|client| client.mapped.unwrap_or(true))
        .map(WindowInfo::try_from)
        .collect::<Result<Vec<_>>>()?;
    windows.sort_by_key(|window| window.window_id);
    enrich_terminal_windows(&mut windows);
    Ok(windows)
}

pub fn activate_window(window_id: u64) -> Result<()> {
    let address = format!("address:0x{window_id:x}");
    let lua_dispatch = lua_focus_dispatch(&address);
    let lua_output = hyprctl_output(&["dispatch", &lua_dispatch])
        .with_context(|| format!("failed to run Hyprland Lua focus dispatcher for {address}"))?;
    if lua_output.status.success() {
        return Ok(());
    }

    let legacy_output = hyprctl_output(&["dispatch", "focuswindow", &address])
        .with_context(|| format!("failed to run hyprctl dispatch focuswindow {address}"))?;
    if legacy_output.status.success() {
        Ok(())
    } else {
        bail!(
            "Hyprland window focus failed for {address}; Lua dispatcher: {}; legacy dispatcher: {}",
            String::from_utf8_lossy(&lua_output.stderr).trim(),
            String::from_utf8_lossy(&legacy_output.stderr).trim()
        );
    }
}

fn lua_focus_dispatch(address: &str) -> String {
    format!("hl.dsp.focus({{ window = \"{address}\" }})")
}

fn hyprctl_output(args: &[&str]) -> std::io::Result<std::process::Output> {
    let mut command = Command::new("hyprctl");
    let has_signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE")
        .ok()
        .is_some_and(|value| !value.trim().is_empty());
    if !has_signature {
        if let Some(signature) = infer_hyprland_instance_signature() {
            command.args(["-i", &signature]);
        }
    }
    command.args(args).output()
}

fn infer_hyprland_instance_signature() -> Option<String> {
    let runtime = xdg_runtime_dir()?;
    let hypr_dir = runtime.join("hypr");
    let wayland_display = std::env::var("WAYLAND_DISPLAY").ok();
    let candidates = fs::read_dir(hypr_dir)
        .ok()?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            let signature = path.file_name()?.to_string_lossy().into_owned();
            hyprland_instance_candidate(&path, signature, wayland_display.as_deref())
        })
        .collect::<Vec<_>>();

    select_hyprland_instance(candidates).map(|candidate| candidate.signature)
}

fn hyprland_instance_candidate(
    path: &Path,
    signature: String,
    wayland_display: Option<&str>,
) -> Option<HyprlandInstanceCandidate> {
    if !path
        .join(".socket.sock")
        .metadata()
        .map(|metadata| metadata.file_type().is_socket())
        .unwrap_or(false)
    {
        return None;
    }

    let lock = fs::read_to_string(path.join("hyprland.lock")).ok()?;
    let mut lines = lock.lines();
    let pid = lines.next()?.trim();
    if pid.is_empty() || !Path::new("/proc").join(pid).exists() {
        return None;
    }
    let lock_wayland_display = lines
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let wayland_display_matches =
        wayland_display.is_some() && lock_wayland_display == wayland_display;
    let modified = path
        .join(".socket.sock")
        .metadata()
        .and_then(|metadata| metadata.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);

    Some(HyprlandInstanceCandidate {
        signature,
        wayland_display_matches,
        modified,
    })
}

fn select_hyprland_instance(
    candidates: Vec<HyprlandInstanceCandidate>,
) -> Option<HyprlandInstanceCandidate> {
    candidates
        .into_iter()
        .max_by_key(|candidate| (candidate.wayland_display_matches, candidate.modified))
}

fn xdg_runtime_dir() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("XDG_RUNTIME_DIR") {
        return Some(PathBuf::from(value));
    }
    let uid = fs::metadata("/proc/self").ok()?.uid();
    Some(PathBuf::from(format!("/run/user/{uid}")))
}

#[derive(Debug)]
struct HyprlandInstanceCandidate {
    signature: String,
    wayland_display_matches: bool,
    modified: SystemTime,
}

#[derive(Debug, Deserialize)]
struct HyprlandMonitor {
    id: i32,
    x: i32,
    y: i32,
    scale: f64,
}

#[derive(Debug, Deserialize)]
struct HyprlandClient {
    address: String,
    mapped: Option<bool>,
    hidden: Option<bool>,
    at: Option<[i32; 2]>,
    size: Option<[u32; 2]>,
    monitor: Option<i32>,
    workspace: Option<HyprlandWorkspace>,
    #[serde(rename = "class")]
    class_name: Option<String>,
    title: Option<String>,
    pid: Option<i64>,
    xwayland: Option<bool>,
    #[serde(rename = "focusHistoryID")]
    focus_history_id: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct HyprlandWorkspace {
    id: Option<i32>,
}

impl TryFrom<HyprlandClient> for WindowInfo {
    type Error = anyhow::Error;

    fn try_from(client: HyprlandClient) -> Result<Self> {
        let window_id = parse_hyprland_address(&client.address)?;
        let bounds = client.size.map(|[width, height]| WindowBounds {
            x: client.at.map(|[x, _]| x),
            y: client.at.map(|[_, y]| y),
            width,
            height,
        });
        let client_type = client.xwayland.map(|xwayland| {
            if xwayland {
                "x11".to_string()
            } else {
                "wayland".to_string()
            }
        });

        Ok(WindowInfo {
            window_id,
            title: client.title,
            app_id: client.class_name.clone(),
            wm_class: client.class_name,
            pid: client.pid.and_then(|pid| u32::try_from(pid).ok()),
            bounds,
            workspace: client.workspace.and_then(|workspace| workspace.id),
            focused: client.focus_history_id == Some(0),
            hidden: client.hidden.unwrap_or(false),
            client_type,
            backend: HYPRLAND_BACKEND.to_string(),
            terminal: None,
        })
    }
}

fn parse_hyprland_address(address: &str) -> Result<u64> {
    let hex = address
        .trim()
        .strip_prefix("0x")
        .context("Hyprland window address did not start with 0x")?;
    u64::from_str_radix(hex, 16)
        .with_context(|| format!("failed to parse Hyprland window address {address}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn rebases_global_window_coordinates_to_screenshot_space() {
        let clients = r#"[{
            "address":"0x1234",
            "mapped":true,
            "at":[4714,1494],
            "size":[931,1124],
            "monitor":0,
            "class":"com.mitchellh.ghostty",
            "title":"Ghostty"
        }]"#;
        let monitors = br#"[
            {"id":0,"x":3747,"y":1440,"scale":1.8},
            {"id":1,"x":5667,"y":0,"scale":2.0}
        ]"#;
        let windows = parse_hyprland_clients_with_monitors(clients, monitors).unwrap();

        let bounds = windows[0].bounds.as_ref().unwrap();
        assert_eq!((bounds.x, bounds.y), (Some(1741), Some(97)));
        assert_eq!((bounds.width, bounds.height), (1676, 2023));
    }

    #[test]
    fn builds_hyprland_055_lua_focus_dispatch() {
        assert_eq!(
            lua_focus_dispatch("address:0x1234abcd"),
            "hl.dsp.focus({ window = \"address:0x1234abcd\" })"
        );
    }

    #[test]
    fn selects_wayland_matching_hyprland_instance_before_newer_nonmatch() {
        let older_match = HyprlandInstanceCandidate {
            signature: "match".to_string(),
            wayland_display_matches: true,
            modified: SystemTime::UNIX_EPOCH,
        };
        let newer_nonmatch = HyprlandInstanceCandidate {
            signature: "nonmatch".to_string(),
            wayland_display_matches: false,
            modified: SystemTime::UNIX_EPOCH + Duration::from_secs(10),
        };

        let selected = select_hyprland_instance(vec![older_match, newer_nonmatch]).unwrap();

        assert_eq!(selected.signature, "match");
    }

    #[test]
    fn selects_newest_hyprland_instance_when_wayland_match_is_tied() {
        let older = HyprlandInstanceCandidate {
            signature: "older".to_string(),
            wayland_display_matches: false,
            modified: SystemTime::UNIX_EPOCH,
        };
        let newer = HyprlandInstanceCandidate {
            signature: "newer".to_string(),
            wayland_display_matches: false,
            modified: SystemTime::UNIX_EPOCH + Duration::from_secs(10),
        };

        let selected = select_hyprland_instance(vec![older, newer]).unwrap();

        assert_eq!(selected.signature, "newer");
    }
}

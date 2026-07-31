use crate::command_runner;
use crate::terminal::enrich_terminal_windows;
use crate::windowing::registry::BackendProbe;
use crate::windowing::types::{WindowBounds, WindowInfo};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::fs;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::time::SystemTime;
use tokio::process::Command;

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

pub async fn list_windows() -> Result<Vec<WindowInfo>> {
    let output = hyprctl_output_async(&["clients", "-j"])
        .await
        .context("failed to run hyprctl clients -j")?;
    if !output.status.success() {
        bail!(
            "hyprctl clients -j failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let clients_json = String::from_utf8_lossy(&output.stdout);
    let monitors_output = hyprctl_output_async(&["monitors", "-j"]).await.ok();
    match monitors_output.filter(|output| output.status.success()) {
        Some(monitors) => parse_hyprland_clients_with_monitors(&clients_json, &monitors.stdout),
        None => parse_hyprland_clients_without_bounds(&clients_json),
    }
}

fn parse_hyprland_clients_with_monitors(
    clients_json: &str,
    monitors_json: &[u8],
) -> Result<Vec<WindowInfo>> {
    let mut clients: Vec<HyprlandClient> =
        serde_json::from_str(clients_json).context("failed to parse hyprctl clients -j output")?;
    let Ok(monitors) = serde_json::from_slice::<Vec<HyprlandMonitor>>(monitors_json) else {
        clear_hyprland_client_bounds(&mut clients);
        return windows_from_hyprland_clients(clients);
    };
    let Some(layout) = HyprlandCaptureLayout::from_monitors(&monitors) else {
        clear_hyprland_client_bounds(&mut clients);
        return windows_from_hyprland_clients(clients);
    };
    let monitor_ids = monitors
        .iter()
        .map(|monitor| monitor.id)
        .collect::<std::collections::HashSet<_>>();
    for client in &mut clients {
        if !client
            .monitor
            .is_some_and(|monitor_id| monitor_ids.contains(&monitor_id))
        {
            client.at = None;
            client.size = None;
            continue;
        }
        let Some((at, size)) = client
            .at
            .zip(client.size)
            .and_then(|(at, size)| layout.map_bounds(at, size))
        else {
            client.at = None;
            client.size = None;
            continue;
        };
        client.at = Some(at);
        client.size = Some(size);
    }
    windows_from_hyprland_clients(clients)
}

#[cfg(test)]
pub(crate) fn parse_hyprland_clients(json: &str) -> Result<Vec<WindowInfo>> {
    let clients: Vec<HyprlandClient> =
        serde_json::from_str(json).context("failed to parse hyprctl clients -j output")?;
    windows_from_hyprland_clients(clients)
}

fn parse_hyprland_clients_without_bounds(json: &str) -> Result<Vec<WindowInfo>> {
    let mut clients: Vec<HyprlandClient> =
        serde_json::from_str(json).context("failed to parse hyprctl clients -j output")?;
    clear_hyprland_client_bounds(&mut clients);
    windows_from_hyprland_clients(clients)
}

fn clear_hyprland_client_bounds(clients: &mut [HyprlandClient]) {
    for client in clients {
        client.at = None;
        client.size = None;
    }
}

#[derive(Debug, Clone, Copy)]
struct HyprlandCaptureLayout {
    origin_x: i32,
    origin_y: i32,
    scale: f64,
}

impl HyprlandCaptureLayout {
    fn from_monitors(monitors: &[HyprlandMonitor]) -> Option<Self> {
        let first = monitors.first()?;
        if monitors
            .iter()
            .any(|monitor| !monitor.scale.is_finite() || monitor.scale <= 0.0)
        {
            return None;
        }
        Some(Self {
            origin_x: monitors
                .iter()
                .map(|monitor| monitor.x)
                .min()
                .unwrap_or(first.x),
            origin_y: monitors
                .iter()
                .map(|monitor| monitor.y)
                .min()
                .unwrap_or(first.y),
            scale: monitors
                .iter()
                .map(|monitor| monitor.scale)
                .fold(first.scale, f64::max),
        })
    }

    fn map_bounds(&self, at: [i32; 2], size: [u32; 2]) -> Option<([i32; 2], [u32; 2])> {
        if size[0] == 0 || size[1] == 0 {
            return None;
        }
        let left = ((i64::from(at[0]) - i64::from(self.origin_x)) as f64 * self.scale).floor();
        let top = ((i64::from(at[1]) - i64::from(self.origin_y)) as f64 * self.scale).floor();
        let right = ((i64::from(at[0]) + i64::from(size[0]) - i64::from(self.origin_x)) as f64
            * self.scale)
            .ceil();
        let bottom = ((i64::from(at[1]) + i64::from(size[1]) - i64::from(self.origin_y)) as f64
            * self.scale)
            .ceil();
        if !left.is_finite()
            || !top.is_finite()
            || !right.is_finite()
            || !bottom.is_finite()
            || left < f64::from(i32::MIN)
            || left > f64::from(i32::MAX)
            || top < f64::from(i32::MIN)
            || top > f64::from(i32::MAX)
            || right <= left
            || bottom <= top
            || right - left > f64::from(u32::MAX)
            || bottom - top > f64::from(u32::MAX)
        {
            return None;
        }
        Some((
            [left as i32, top as i32],
            [(right - left) as u32, (bottom - top) as u32],
        ))
    }
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

pub async fn activate_window(window_id: u64) -> Result<()> {
    let address = format!("address:0x{window_id:x}");
    let lua_dispatch = lua_focus_dispatch(&address);
    let lua_output = hyprctl_output_async(&["dispatch", &lua_dispatch])
        .await
        .with_context(|| format!("failed to run Hyprland Lua focus dispatcher for {address}"))?;
    if dispatch_succeeded(&lua_output) {
        return Ok(());
    }

    let legacy_output = hyprctl_output_async(&["dispatch", "focuswindow", &address])
        .await
        .with_context(|| format!("failed to run hyprctl dispatch focuswindow {address}"))?;
    if dispatch_succeeded(&legacy_output) {
        Ok(())
    } else {
        bail!(
            "Hyprland window focus failed for {address}; Lua dispatcher: {}; legacy dispatcher: {}",
            command_detail(&lua_output),
            command_detail(&legacy_output)
        );
    }
}

fn dispatch_succeeded(output: &std::process::Output) -> bool {
    output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "ok"
}

fn command_detail(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    if detail.is_empty() {
        format!("exit status {}", output.status)
    } else {
        detail.to_string()
    }
}

fn lua_focus_dispatch(address: &str) -> String {
    format!("hl.dsp.focus({{ window = \"{address}\" }})")
}

fn hyprctl_output(args: &[&str]) -> std::io::Result<std::process::Output> {
    let mut command = StdCommand::new("hyprctl");
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

async fn hyprctl_output_async(args: &[&str]) -> Result<std::process::Output> {
    let mut command = Command::new("hyprctl");
    let has_signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE")
        .ok()
        .is_some_and(|value| !value.trim().is_empty());
    if !has_signature {
        if let Some(signature) = infer_hyprland_instance_signature() {
            command.args(["-i", &signature]);
        }
    }
    command.args(args);
    command_runner::output(command, "run hyprctl").await
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
    use std::os::unix::process::ExitStatusExt;
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
        assert_eq!((bounds.x, bounds.y), (Some(1934), Some(2988)));
        assert_eq!((bounds.width, bounds.height), (1862, 2248));
    }

    #[test]
    fn global_union_accounts_for_negative_monitor_origins() {
        let clients = r#"[{
            "address":"0x1234",
            "at":[-1800,100],
            "size":[600,400],
            "monitor":0,
            "class":"foot",
            "title":"Shell"
        }]"#;
        let monitors = br#"[
            {"id":0,"x":-1920,"y":0,"scale":1.0},
            {"id":1,"x":0,"y":0,"scale":2.0}
        ]"#;
        let windows = parse_hyprland_clients_with_monitors(clients, monitors).unwrap();

        let bounds = windows[0].bounds.as_ref().unwrap();
        assert_eq!((bounds.x, bounds.y), (Some(240), Some(200)));
        assert_eq!((bounds.width, bounds.height), (1200, 800));
    }

    #[test]
    fn fractional_scale_rounds_crop_edges_outward() {
        let clients = r#"[{
            "address":"0x1234",
            "at":[1,1],
            "size":[1,1],
            "monitor":0,
            "class":"foot",
            "title":"Shell"
        }]"#;
        let monitors = br#"[{"id":0,"x":0,"y":0,"scale":1.25}]"#;
        let windows = parse_hyprland_clients_with_monitors(clients, monitors).unwrap();

        let bounds = windows[0].bounds.as_ref().unwrap();
        assert_eq!((bounds.x, bounds.y), (Some(1), Some(1)));
        assert_eq!((bounds.width, bounds.height), (2, 2));
    }

    #[test]
    fn bounds_are_omitted_without_valid_monitor_metadata() {
        let clients = r#"[{
            "address":"0x1234",
            "at":[100,100],
            "size":[600,400],
            "monitor":7,
            "class":"foot",
            "title":"Shell"
        }]"#;

        for monitors in [
            br#"[]"#.as_slice(),
            br#"[{"id":7,"x":0,"y":0,"scale":0.0}]"#.as_slice(),
            b"not json".as_slice(),
        ] {
            let windows = parse_hyprland_clients_with_monitors(clients, monitors).unwrap();
            assert!(windows[0].bounds.is_none());
        }
        let windows = parse_hyprland_clients_without_bounds(clients).unwrap();
        assert!(windows[0].bounds.is_none());
    }

    #[test]
    fn builds_hyprland_055_lua_focus_dispatch() {
        assert_eq!(
            lua_focus_dispatch("address:0x1234abcd"),
            "hl.dsp.focus({ window = \"address:0x1234abcd\" })"
        );
    }

    #[test]
    fn dispatch_rejects_exit_zero_error_output() {
        let output = std::process::Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: b"Invalid dispatcher\n".to_vec(),
            stderr: Vec::new(),
        };

        assert!(!dispatch_succeeded(&output));
        assert_eq!(command_detail(&output), "Invalid dispatcher");
    }

    #[test]
    fn dispatch_accepts_ok_output() {
        let output = std::process::Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: b"ok\n".to_vec(),
            stderr: Vec::new(),
        };

        assert!(dispatch_succeeded(&output));
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

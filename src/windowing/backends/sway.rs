use crate::terminal::enrich_terminal_windows;
use crate::windowing::registry::BackendProbe;
use crate::windowing::types::{WindowBounds, WindowInfo};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::{env, fs, os::unix::fs::FileTypeExt, path::PathBuf, process::Command};

pub const SWAY_BACKEND: &str = "sway";

pub fn probe() -> BackendProbe {
    match sway_msg_command().args(["-t", "get_tree"]).output() {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let ok = matches!(
                serde_json::from_str::<serde_json::Value>(&stdout),
                Ok(serde_json::Value::Object(_))
            );
            BackendProbe {
                id: SWAY_BACKEND,
                ok,
                can_list_windows: ok,
                can_focus_apps: ok,
                can_focus_windows: ok,
                detail: if ok {
                    "swaymsg -t get_tree returned a JSON tree".to_string()
                } else {
                    "swaymsg -t get_tree did not return a JSON object".to_string()
                },
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            BackendProbe {
                id: SWAY_BACKEND,
                ok: false,
                can_list_windows: false,
                can_focus_apps: false,
                can_focus_windows: false,
                detail: if stderr.is_empty() { stdout } else { stderr },
            }
        }
        Err(error) => BackendProbe {
            id: SWAY_BACKEND,
            ok: false,
            can_list_windows: false,
            can_focus_apps: false,
            can_focus_windows: false,
            detail: error.to_string(),
        },
    }
}

pub fn list_windows() -> Result<Vec<WindowInfo>> {
    let output = sway_msg_command()
        .args(["-t", "get_tree"])
        .output()
        .context("failed to run swaymsg -t get_tree")?;
    if !output.status.success() {
        bail!(
            "swaymsg -t get_tree failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let mut windows = parse_sway_tree(&String::from_utf8_lossy(&output.stdout))?;
    hydrate_sway_window_pids(&mut windows);
    enrich_terminal_windows(&mut windows);
    Ok(windows)
}

pub(crate) fn parse_sway_tree(json: &str) -> Result<Vec<WindowInfo>> {
    let root: SwayNode =
        serde_json::from_str(json).context("failed to parse swaymsg get_tree output")?;
    let mut windows = Vec::new();
    collect_sway_windows(&root, None, false, &mut windows);
    windows.sort_by_key(|window| window.window_id);
    Ok(windows)
}

pub fn activate_window(window_id: u64) -> Result<()> {
    let selector = format!("[con_id={window_id}] focus");
    let output = sway_msg_command()
        .arg(&selector)
        .output()
        .with_context(|| format!("failed to run swaymsg {selector}"))?;
    if !output.status.success() {
        bail!(
            "swaymsg {selector} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let replies: Vec<SwayCommandReply> =
        serde_json::from_slice(&output.stdout).context("failed to parse swaymsg focus reply")?;
    if replies.iter().all(|reply| reply.success) {
        Ok(())
    } else {
        let details = replies
            .into_iter()
            .filter_map(|reply| reply.error)
            .collect::<Vec<_>>()
            .join("; ");
        bail!(
            "swaymsg {selector} did not focus the window: {}",
            if details.is_empty() {
                "unknown sway failure"
            } else {
                details.as_str()
            }
        );
    }
}

fn collect_sway_windows(
    node: &SwayNode,
    workspace: Option<i32>,
    in_dockarea: bool,
    windows: &mut Vec<WindowInfo>,
) {
    let node_type = node.node_type.as_deref();
    let current_workspace = if node_type == Some("workspace") {
        node.num
    } else {
        workspace
    };
    let current_in_dockarea = in_dockarea || node_type == Some("dockarea");

    if let Some(window) = node.to_window_info(current_workspace, current_in_dockarea) {
        windows.push(window);
    }

    for child in &node.nodes {
        collect_sway_windows(child, current_workspace, current_in_dockarea, windows);
    }
    for child in &node.floating_nodes {
        collect_sway_windows(child, current_workspace, current_in_dockarea, windows);
    }
}

fn hydrate_sway_window_pids(windows: &mut [WindowInfo]) {
    for window in windows {
        if window.pid.is_none() {
            if let Some(client_type) = window.client_type.as_deref() {
                if client_type == "x11" {
                    window.pid = sway_x11_window_pid(window.window_id);
                }
            }
        }
    }
}

fn sway_x11_window_pid(window_id: u64) -> Option<u32> {
    let output = Command::new("xprop")
        .args(["-id", &window_id.to_string(), "_NET_WM_PID"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    crate::windowing::backends::i3::parse_xprop_pid(&String::from_utf8_lossy(&output.stdout))
}

fn sway_msg_command() -> Command {
    let mut command = Command::new("swaymsg");
    if let Some(socket_path) = sway_socket_path() {
        command.arg("-s").arg(socket_path);
    }
    command
}

fn sway_socket_path() -> Option<PathBuf> {
    if let Some(value) = env_var("SWAYSOCK") {
        return Some(PathBuf::from(value));
    }

    let socket_dir = xdg_runtime_dir()?;
    let mut sockets = fs::read_dir(socket_dir)
        .ok()?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let file_name = entry.file_name();
            let file_name = file_name.to_str()?;
            if !file_name.starts_with("sway-ipc.") || !file_name.ends_with(".sock") {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            if !metadata.file_type().is_socket() {
                return None;
            }
            let modified = metadata.modified().ok();
            Some((modified, entry.path()))
        })
        .collect::<Vec<_>>();
    sockets.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    sockets.into_iter().map(|(_, path)| path).next()
}

fn xdg_runtime_dir() -> Option<PathBuf> {
    env_var("XDG_RUNTIME_DIR").map(PathBuf::from)
}

fn env_var(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn clean_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "null")
        .map(ToOwned::to_owned)
}

#[derive(Debug, Deserialize)]
struct SwayCommandReply {
    success: bool,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SwayNode {
    id: Option<u64>,
    #[serde(rename = "type")]
    node_type: Option<String>,
    name: Option<String>,
    window: Option<u64>,
    window_type: Option<String>,
    app_id: Option<String>,
    window_properties: Option<SwayWindowProperties>,
    rect: Option<SwayRect>,
    geometry: Option<SwayRect>,
    #[serde(default)]
    focused: bool,
    #[serde(default)]
    nodes: Vec<SwayNode>,
    #[serde(default)]
    floating_nodes: Vec<SwayNode>,
    num: Option<i32>,
    scratchpad_state: Option<String>,
    pid: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct SwayWindowProperties {
    class: Option<String>,
    instance: Option<String>,
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SwayRect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl SwayNode {
    fn to_window_info(&self, workspace: Option<i32>, in_dockarea: bool) -> Option<WindowInfo> {
        if in_dockarea {
            return None;
        }
        if self.node_type.as_deref() != Some("con") {
            return None;
        }
        if self.window_type.as_deref() == Some("dock") {
            return None;
        }

        let has_window = self.window.is_some()
            || self.app_id.is_some()
            || self
                .window_properties
                .as_ref()
                .is_some_and(|properties| properties.title.is_some() || properties.class.is_some());
        if !has_window {
            return None;
        }

        let window_id = self.id.or(self.window)?;
        let properties = self.window_properties.as_ref();
        let title = clean_string(
            properties
                .and_then(|properties| properties.title.as_deref())
                .or(self.name.as_deref()),
        );
        let wm_class = clean_string(
            properties
                .and_then(|properties| properties.class.as_deref())
                .or_else(|| properties.and_then(|properties| properties.instance.as_deref())),
        );
        let app_id = clean_string(
            self.app_id
                .as_deref()
                .or_else(|| properties.and_then(|properties| properties.instance.as_deref()))
                .or(wm_class.as_deref()),
        );
        let rect = self.rect.as_ref().or(self.geometry.as_ref());
        let bounds = rect.map(|rect| WindowBounds {
            x: Some(rect.x),
            y: Some(rect.y),
            width: rect.width,
            height: rect.height,
        });
        let client_type = if self.window.is_some() {
            "x11".to_string()
        } else {
            "wayland".to_string()
        };

        Some(WindowInfo {
            window_id,
            title,
            app_id,
            wm_class,
            pid: self.pid.and_then(|pid| u32::try_from(pid).ok()),
            bounds,
            workspace,
            focused: self.focused,
            hidden: self.scratchpad_state.as_deref() == Some("fresh"),
            client_type: Some(client_type),
            backend: SWAY_BACKEND.to_string(),
            terminal: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sway_tree_with_wayland_container_id() {
        let json = r#"{
            "id": 1,
            "type": "workspace",
            "num": 1,
            "nodes": [{
                "id": 42,
                "type": "con",
                "name": "Firefox",
                "app_id": "firefox",
                "focused": true,
                "rect": {"x": 10, "y": 20, "width": 800, "height": 600},
                "nodes": []
            }]
        }"#;

        let windows = parse_sway_tree(json).expect("parse sway tree");
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].window_id, 42);
        assert_eq!(windows[0].backend, SWAY_BACKEND);
        assert_eq!(windows[0].client_type.as_deref(), Some("wayland"));
        assert!(windows[0].focused);
    }
}
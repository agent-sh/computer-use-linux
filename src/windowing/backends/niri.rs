use crate::command_runner;
use crate::terminal::enrich_terminal_windows;
use crate::windowing::registry::BackendProbe;
use crate::windowing::types::{WindowBounds, WindowInfo};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::process::Command as StdCommand;
use std::time::Duration;
use tokio::process::Command;

pub const NIRI_BACKEND: &str = "niri";

const NIRI_COMMAND_TIMEOUT: Duration = Duration::from_secs(2);

pub fn probe() -> BackendProbe {
    match niri_output_blocking(&["msg", "--json", "windows"]) {
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
        Ok(output) => BackendProbe {
            id: NIRI_BACKEND,
            ok: false,
            can_list_windows: false,
            can_focus_apps: false,
            can_focus_windows: false,
            detail: command_failure_detail(&output),
        },
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

pub async fn list_windows() -> Result<Vec<WindowInfo>> {
    let output = niri_output(&["msg", "--json", "windows"])
        .await
        .context("failed to run niri msg --json windows")?;
    if !output.status.success() {
        bail!(
            "niri msg --json windows failed: {}",
            command_failure_detail(&output)
        );
    }

    parse_niri_windows(&String::from_utf8_lossy(&output.stdout))
}

pub(crate) fn parse_niri_windows(json: &str) -> Result<Vec<WindowInfo>> {
    let records: Vec<NiriWindow> =
        serde_json::from_str(json).context("failed to parse niri msg --json windows output")?;
    let mut windows = records
        .into_iter()
        .map(WindowInfo::from)
        .collect::<Vec<_>>();
    windows.sort_by_key(|window| window.window_id);
    enrich_terminal_windows(&mut windows);
    Ok(windows)
}

pub async fn activate_window(window_id: u64) -> Result<()> {
    let args = niri_focus_args(window_id);
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let output = niri_output(&arg_refs)
        .await
        .with_context(|| format!("failed to focus Niri window {window_id}"))?;
    if output.status.success() {
        Ok(())
    } else {
        bail!(
            "niri msg action focus-window --id {window_id} failed: {}",
            command_failure_detail(&output)
        );
    }
}

pub(crate) fn niri_focus_args(window_id: u64) -> [String; 5] {
    [
        "msg".to_string(),
        "action".to_string(),
        "focus-window".to_string(),
        "--id".to_string(),
        window_id.to_string(),
    ]
}

fn niri_output_blocking(args: &[&str]) -> Result<std::process::Output> {
    let mut command = StdCommand::new("niri");
    command.args(args);
    command_runner::output_blocking_with_timeout(
        &mut command,
        "run niri IPC command",
        NIRI_COMMAND_TIMEOUT,
    )
}

async fn niri_output(args: &[&str]) -> Result<std::process::Output> {
    let mut command = Command::new("niri");
    command.args(args);
    command_runner::output(command, "run niri IPC command").await
}

fn command_failure_detail(output: &std::process::Output) -> String {
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

#[derive(Debug, Deserialize)]
struct NiriWindow {
    id: u64,
    title: Option<String>,
    app_id: Option<String>,
    pid: Option<i64>,
    workspace_id: Option<u64>,
    #[serde(default)]
    is_focused: bool,
    layout: Option<NiriWindowLayout>,
}

#[derive(Debug, Deserialize)]
struct NiriWindowLayout {
    tile_size: Option<[f64; 2]>,
    window_size: Option<[i64; 2]>,
}

impl From<NiriWindow> for WindowInfo {
    fn from(window: NiriWindow) -> Self {
        let bounds = window.layout.and_then(|layout| {
            let tile_dimensions = layout.tile_size.and_then(|[width, height]| {
                Some((
                    positive_finite_dimension(width)?,
                    positive_finite_dimension(height)?,
                ))
            });
            let window_dimensions = layout.window_size.and_then(|[width, height]| {
                Some((
                    u32::try_from(width).ok().filter(|value| *value > 0)?,
                    u32::try_from(height).ok().filter(|value| *value > 0)?,
                ))
            });
            let (width, height) = tile_dimensions.or(window_dimensions)?;
            Some(WindowBounds {
                x: None,
                y: None,
                width,
                height,
            })
        });

        Self {
            window_id: window.id,
            title: window.title,
            app_id: window.app_id.clone(),
            wm_class: window.app_id,
            pid: window.pid.and_then(|pid| u32::try_from(pid).ok()),
            bounds,
            workspace: window
                .workspace_id
                .and_then(|workspace| i32::try_from(workspace).ok()),
            focused: window.is_focused,
            hidden: false,
            client_type: None,
            backend: NIRI_BACKEND.to_string(),
            terminal: None,
        }
    }
}

fn positive_finite_dimension(value: f64) -> Option<u32> {
    (value.is_finite() && value > 0.0 && value <= f64::from(u32::MAX)).then(|| value.ceil() as u32)
}

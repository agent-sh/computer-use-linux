use crate::diagnostics::hydrate_session_bus_env;
use crate::identity;
use crate::windowing::backends::gnome::list_extension_windows;
use crate::windows::{window_permission_hint, WindowInfo};
use schemars::JsonSchema;
use serde::Serialize;
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

pub const UUID: &str = identity::GNOME_EXTENSION_UUID;
const METADATA_JSON: &str =
    include_str!("../gnome-shell-extension/computer-use-linux@avifenesh.dev/metadata.json");
const EXTENSION_JS: &str =
    include_str!("../gnome-shell-extension/computer-use-linux@avifenesh.dev/extension.js");

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct WindowTargetingSetupReport {
    pub extension_dir: String,
    pub wrote_files: bool,
    pub enable_command: SetupCommandReport,
    pub windows: Vec<WindowInfo>,
    pub windows_error: Option<String>,
    pub permissions_hint: Option<String>,
    pub requires_shell_reload: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SetupCommandReport {
    pub ok: bool,
    pub detail: String,
}

pub async fn setup_window_targeting_report() -> WindowTargetingSetupReport {
    hydrate_session_bus_env();

    let extension_dir = extension_dir();
    let mut wrote_files = false;
    let mut write_error = None;
    match write_extension_files(&extension_dir) {
        Ok(()) => wrote_files = true,
        Err(error) => write_error = Some(error),
    }

    let enable_command = if let Some(error) = &write_error {
        SetupCommandReport {
            ok: false,
            detail: format!("extension file write failed: {error}"),
        }
    } else {
        run_gnome_extensions_enable()
    };

    let (windows, windows_error, permissions_hint) = match list_extension_windows().await {
        Ok(windows) => (windows, None, None),
        Err(error) => {
            let error = format!("{error:#}");
            let hint = window_permission_hint(&error);
            (Vec::new(), Some(error), hint)
        }
    };

    let requires_shell_reload = windows_error.is_some();
    let message = if !wrote_files {
        "Could not install the computer-use-linux GNOME Shell extension files.".to_string()
    } else if !enable_command.ok {
        "computer-use-linux GNOME Shell extension files were installed, but enabling the extension failed. Enable it with gnome-extensions after GNOME Shell sees the new extension."
            .to_string()
    } else if windows_error.is_none() {
        "computer-use-linux GNOME Shell extension is active and window targeting is available."
            .to_string()
    } else {
        "computer-use-linux GNOME Shell extension files were installed and enable was requested, but GNOME Shell is not serving the window-control DBus API yet. Log out and back in, then retry setup_window_targeting."
            .to_string()
    };

    WindowTargetingSetupReport {
        extension_dir: extension_dir.display().to_string(),
        wrote_files,
        enable_command,
        windows,
        windows_error,
        permissions_hint,
        requires_shell_reload,
        message,
    }
}

fn write_extension_files(extension_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(extension_dir)
        .map_err(|error| format!("failed to create {}: {error}", extension_dir.display()))?;
    let metadata_json = render_extension_asset(METADATA_JSON);
    let extension_js = render_extension_asset(EXTENSION_JS);

    fs::write(extension_dir.join("metadata.json"), metadata_json).map_err(|error| {
        format!(
            "failed to write {}: {error}",
            extension_dir.join("metadata.json").display()
        )
    })?;
    fs::write(extension_dir.join("extension.js"), extension_js).map_err(|error| {
        format!(
            "failed to write {}: {error}",
            extension_dir.join("extension.js").display()
        )
    })?;
    Ok(())
}

fn render_extension_asset(asset: &str) -> String {
    asset
        .replace(
            identity::DEFAULT_GNOME_EXTENSION_UUID,
            identity::GNOME_EXTENSION_UUID,
        )
        .replace(identity::DEFAULT_DBUS_SERVICE, identity::DBUS_SERVICE)
        .replace(
            identity::DEFAULT_DBUS_OBJECT_PATH,
            identity::DBUS_OBJECT_PATH,
        )
}

fn run_gnome_extensions_enable() -> SetupCommandReport {
    let mut command = Command::new("gnome-extensions");
    command.args(["enable", UUID]);
    add_session_env(&mut command);

    let primary = match command.output() {
        Ok(output) if output.status.success() => SetupCommandReport {
            ok: true,
            detail: output_detail(&output.stdout, &output.stderr, "gnome-extensions enable ok"),
        },
        Ok(output) => SetupCommandReport {
            ok: false,
            detail: output_detail(
                &output.stdout,
                &output.stderr,
                &format!("gnome-extensions exited with {}", output.status),
            ),
        },
        Err(error) => SetupCommandReport {
            ok: false,
            detail: format!("failed to run gnome-extensions: {error}"),
        },
    };
    if primary.ok {
        return primary;
    }

    let fallback = run_gsettings_enable_fallback();
    if fallback.ok {
        SetupCommandReport {
            ok: true,
            detail: format!(
                "gnome-extensions enable failed: {}; {detail}",
                primary.detail,
                detail = fallback.detail
            ),
        }
    } else {
        SetupCommandReport {
            ok: false,
            detail: format!(
                "gnome-extensions enable failed: {}; gsettings fallback failed: {}",
                primary.detail, fallback.detail
            ),
        }
    }
}

fn run_gsettings_enable_fallback() -> SetupCommandReport {
    let mut get_command = Command::new("gsettings");
    get_command.args(["get", "org.gnome.shell", "enabled-extensions"]);
    add_session_env(&mut get_command);
    let current = match get_command.output() {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
        Ok(output) => {
            return SetupCommandReport {
                ok: false,
                detail: output_detail(&output.stdout, &output.stderr, "gsettings get failed"),
            }
        }
        Err(error) => {
            return SetupCommandReport {
                ok: false,
                detail: format!("failed to run gsettings get: {error}"),
            }
        }
    };

    let Some(updated) = enabled_extensions_literal(&current) else {
        return SetupCommandReport {
            ok: false,
            detail: format!("could not parse enabled-extensions value: {current}"),
        };
    };
    if updated == current {
        return SetupCommandReport {
            ok: true,
            detail: format!("{UUID} already present in org.gnome.shell enabled-extensions"),
        };
    }

    let mut set_command = Command::new("gsettings");
    set_command.args(["set", "org.gnome.shell", "enabled-extensions", &updated]);
    add_session_env(&mut set_command);
    match set_command.output() {
        Ok(output) if output.status.success() => SetupCommandReport {
            ok: true,
            detail: format!(
                "added {UUID} to org.gnome.shell enabled-extensions for the next GNOME Shell load"
            ),
        },
        Ok(output) => SetupCommandReport {
            ok: false,
            detail: output_detail(&output.stdout, &output.stderr, "gsettings set failed"),
        },
        Err(error) => SetupCommandReport {
            ok: false,
            detail: format!("failed to run gsettings set: {error}"),
        },
    }
}

fn enabled_extensions_literal(current: &str) -> Option<String> {
    let trimmed = current.trim();
    let quoted = format!("'{UUID}'");
    if trimmed.contains(&quoted) {
        return Some(trimmed.to_string());
    }

    let list = if trimmed == "@as []" { "[]" } else { trimmed };
    if list == "[]" {
        return Some(format!("[{quoted}]"));
    }

    let prefix = list.strip_suffix(']')?;
    Some(format!("{prefix}, {quoted}]"))
}

fn add_session_env(command: &mut Command) {
    if let Some(address) = env::var("DBUS_SESSION_BUS_ADDRESS")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        command.env("DBUS_SESSION_BUS_ADDRESS", address);
    }
    if let Some(runtime) = env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        command.env("XDG_RUNTIME_DIR", runtime);
    }
}

fn output_detail(stdout: &[u8], stderr: &[u8], fallback: &str) -> String {
    let stderr = String::from_utf8_lossy(stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    let stdout = String::from_utf8_lossy(stdout).trim().to_string();
    if !stdout.is_empty() {
        return stdout;
    }
    fallback.to_string()
}

fn extension_dir() -> PathBuf {
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".local/share/gnome-shell/extensions")
        .join(UUID)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_extensions_literal_adds_uuid_to_existing_list() {
        assert_eq!(
            enabled_extensions_literal("['ubuntu-dock@ubuntu.com']").unwrap(),
            format!("['ubuntu-dock@ubuntu.com', '{UUID}']")
        );
    }

    #[test]
    fn enabled_extensions_literal_handles_empty_typed_array() {
        assert_eq!(
            enabled_extensions_literal("@as []").unwrap(),
            format!("['{UUID}']")
        );
    }

    #[test]
    fn enabled_extensions_literal_is_idempotent() {
        let value = format!("['{UUID}']");

        assert_eq!(enabled_extensions_literal(&value).unwrap(), value);
    }

    #[test]
    fn rendered_metadata_uses_build_identity() {
        let rendered = render_extension_asset(METADATA_JSON);

        assert!(rendered.contains(&format!("\"uuid\": \"{UUID}\"")));
    }

    #[test]
    fn rendered_extension_uses_build_identity() {
        let rendered = render_extension_asset(EXTENSION_JS);

        assert!(rendered.contains(&format!(
            "const SERVICE_NAME = '{service}'",
            service = identity::DBUS_SERVICE
        )));
        assert!(rendered.contains(&format!(
            "const OBJECT_PATH = '{path}'",
            path = identity::DBUS_OBJECT_PATH
        )));
    }
}

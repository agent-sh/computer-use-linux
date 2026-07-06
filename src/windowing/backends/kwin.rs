use crate::diagnostics::hydrate_session_bus_env;
use crate::terminal::enrich_terminal_windows;
use crate::windowing::registry::BackendProbe;
use crate::windowing::types::{WindowBounds, WindowInfo};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    sync::mpsc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::time::{sleep, timeout};
use zbus::Proxy;

pub const KWIN_BACKEND: &str = "kwin";
const KWIN_SCRIPT_TIMEOUT: Duration = Duration::from_secs(2);
const KWIN_SCRIPTING_SERVICE: &str = "org.kde.KWin";
const KWIN_SCRIPTING_OBJECT_PATH: &str = "/Scripting";
const KWIN_SCRIPTING_INTERFACE: &str = "org.kde.kwin.Scripting";
const KWIN_CALLBACK_OBJECT_PATH_PREFIX: &str = "/dev/avifenesh/ComputerUseLinux/KWinWindowQuery";
const KWIN_CALLBACK_INTERFACE: &str = "dev.avifenesh.ComputerUseLinux.KWinWindowQuery";

pub fn probe() -> BackendProbe {
    let check = gdbus_introspect_contains(
        "org.kde.KWin",
        "/Scripting",
        "org.kde.kwin.Scripting",
        "loadScript",
    );
    BackendProbe {
        id: KWIN_BACKEND,
        ok: check.ok,
        can_list_windows: check.ok,
        can_focus_apps: check.ok,
        can_focus_windows: check.ok,
        detail: if check.ok {
            "KWin scripting is available on the session bus".to_string()
        } else {
            format!("KWin scripting unavailable: {}", check.detail)
        },
    }
}

pub async fn list_windows() -> Result<Vec<WindowInfo>> {
    let json = call_kwin_window_script().await?;
    let mut windows = parse_kwin_windows(&json)?;
    enrich_terminal_windows(&mut windows);
    Ok(windows)
}

pub async fn activate_window(window_id: u64) -> Result<()> {
    let uuid = kwin_uuid_for_window_id(window_id).await?.with_context(|| {
        format!("No KWin window matched window_id {window_id} during activation")
    })?;
    call_kwin_activate_script(&uuid).await
}

async fn kwin_uuid_for_window_id(window_id: u64) -> Result<Option<String>> {
    let json = call_kwin_window_script().await?;
    let snapshot = parse_kwin_snapshot(&json)?;
    Ok(snapshot.windows.into_iter().find_map(|window| {
        let uuid = window.kwin_uuid()?;
        (kwin_window_id_from_uuid(&uuid) == window_id).then_some(uuid)
    }))
}

#[derive(Debug, Deserialize)]
struct KwinScriptResult {
    #[serde(default)]
    ok: bool,
    error: Option<String>,
}

async fn call_kwin_activate_script(uuid: &str) -> Result<()> {
    let uuid = uuid.to_string();
    let json = call_kwin_script(|service_name, callback_object_path, plugin_name| {
        write_kwin_activate_script(service_name, callback_object_path, plugin_name, &uuid)
    })
    .await?;
    let result: KwinScriptResult =
        serde_json::from_str(&json).context("failed to parse KWin activation script output")?;

    if result.ok {
        Ok(())
    } else {
        bail!(
            "KWin activation script refused activation: {}",
            result.error.unwrap_or_else(|| "unknown error".to_string())
        );
    }
}

async fn call_kwin_window_script() -> Result<String> {
    call_kwin_script(write_kwin_window_script).await
}

async fn call_kwin_script<F>(write_script: F) -> Result<String>
where
    F: FnOnce(&str, &str, &str) -> Result<std::path::PathBuf>,
{
    hydrate_session_bus_env();

    let connection = zbus::Connection::session()
        .await
        .context("failed to connect to session bus")?;
    let unique_name = connection
        .unique_name()
        .context("session bus did not assign a unique name")?
        .to_string();
    let plugin_name = temporary_kwin_plugin_name();
    let callback_object_path = format!("{KWIN_CALLBACK_OBJECT_PATH_PREFIX}/{plugin_name}");
    let (sender, receiver) = mpsc::channel();
    connection
        .object_server()
        .at(callback_object_path.as_str(), KwinWindowCallback { sender })
        .await
        .context("failed to register temporary KWin callback object")?;

    let mut script_path = None;
    let mut loaded_script = false;
    let result = async {
        let path = write_script(&unique_name, &callback_object_path, &plugin_name)?;
        script_path = Some(path.clone());
        let scripting_proxy = Proxy::new(
            &connection,
            KWIN_SCRIPTING_SERVICE,
            KWIN_SCRIPTING_OBJECT_PATH,
            KWIN_SCRIPTING_INTERFACE,
        )
        .await
        .context("failed to create KWin scripting proxy")?;

        // Plasma 6 can return 0 here even when isScriptLoaded reports success;
        // the callback below is the authoritative completion signal.
        let _script_id: i32 = scripting_proxy
            .call(
                "loadScript",
                &(path.to_string_lossy().as_ref(), plugin_name.as_str()),
            )
            .await
            .context("KWin loadScript failed")?;
        loaded_script = true;

        let _: () = scripting_proxy
            .call("start", &())
            .await
            .context("KWin start failed after loading the temporary script")?;

        timeout(KWIN_SCRIPT_TIMEOUT, async move {
            loop {
                match receiver.try_recv() {
                    Ok(json) => return Ok(json),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        bail!("KWin temporary script callback disconnected before returning data");
                    }
                    Err(mpsc::TryRecvError::Empty) => sleep(Duration::from_millis(20)).await,
                }
            }
        })
        .await
        .context("KWin temporary script did not return data before timeout")?
    }
    .await;

    if loaded_script {
        if let Ok(scripting_proxy) = Proxy::new(
            &connection,
            KWIN_SCRIPTING_SERVICE,
            KWIN_SCRIPTING_OBJECT_PATH,
            KWIN_SCRIPTING_INTERFACE,
        )
        .await
        {
            let _: Result<bool, _> = scripting_proxy
                .call("unloadScript", &(plugin_name.as_str()))
                .await;
        }
    }
    let _: Result<bool, _> = connection
        .object_server()
        .remove::<KwinWindowCallback, _>(callback_object_path.as_str())
        .await;
    if let Some(script_path) = script_path {
        let _ = fs::remove_file(script_path);
    }

    result
}

struct KwinWindowCallback {
    sender: mpsc::Sender<String>,
}

#[zbus::interface(name = "dev.avifenesh.ComputerUseLinux.KWinWindowQuery")]
impl KwinWindowCallback {
    fn receive_windows(&self, json: &str) -> zbus::fdo::Result<()> {
        self.sender
            .send(json.to_string())
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }

    fn receive_result(&self, json: &str) -> zbus::fdo::Result<()> {
        self.sender
            .send(json.to_string())
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }
}

fn temporary_kwin_plugin_name() -> String {
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("computer_use_linux_kwin_window_query_{pid}_{nanos}")
}

fn write_kwin_window_script(
    service_name: &str,
    callback_object_path: &str,
    plugin_name: &str,
) -> Result<std::path::PathBuf> {
    let script = kwin_window_script_source(service_name, callback_object_path, plugin_name)?;
    write_kwin_script_file(plugin_name, &script)
}

fn kwin_window_script_source(
    service_name: &str,
    callback_object_path: &str,
    plugin_name: &str,
) -> Result<String> {
    let service_name = serde_json::to_string(service_name)?;
    let object_path = serde_json::to_string(callback_object_path)?;
    let interface = serde_json::to_string(KWIN_CALLBACK_INTERFACE)?;
    let plugin_name_json = serde_json::to_string(plugin_name)?;
    Ok(format!(
        r#"(function() {{
    var serviceName = {service_name};
    var objectPath = {object_path};
    var iface = {interface};
    var pluginName = {plugin_name_json};

    function read(obj, key) {{
        try {{
            if (obj === null || obj === undefined) {{
                return null;
            }}
            var value = obj[key];
            if (typeof value === "function") {{
                return null;
            }}
            return serialize(value);
        }} catch (error) {{
            return null;
        }}
    }}

    function serialize(value) {{
        if (value === null || value === undefined) {{
            return null;
        }}
        if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") {{
            return value;
        }}
        if (Array.isArray(value)) {{
            return value.map(serialize);
        }}
        try {{
            if (typeof value.toString === "function") {{
                return value.toString();
            }}
        }} catch (error) {{}}
        return null;
    }}

    function geometry(window) {{
        var frame = null;
        try {{
            frame = window.frameGeometry;
        }} catch (error) {{}}
        var x = read(window, "x");
        var y = read(window, "y");
        var width = read(window, "width");
        var height = read(window, "height");
        return {{
            x: x !== null ? x : read(frame, "x"),
            y: y !== null ? y : read(frame, "y"),
            width: width !== null ? width : read(frame, "width"),
            height: height !== null ? height : read(frame, "height")
        }};
    }}

    function firstDesktop(window) {{
        var desktops = read(window, "desktops");
        if (!Array.isArray(desktops) || desktops.length === 0) {{
            return null;
        }}
        var first = desktops[0];
        var parsed = parseInt(first, 10);
        return isFinite(parsed) ? parsed : null;
    }}

    function clientType(window) {{
        if (read(window, "waylandClient")) {{
            return "wayland";
        }}
        if (read(window, "x11Client")) {{
            return "x11";
        }}
        return null;
    }}

    var activeWindow = null;
    try {{
        activeWindow = workspace.activeWindow;
    }} catch (error) {{}}
    var windows = workspace.windowList().map(function(window) {{
        var geo = geometry(window);
        return {{
            uuid: read(window, "uuid"),
            internalId: read(window, "internalId"),
            caption: read(window, "caption"),
            desktopFile: read(window, "desktopFile"),
            resourceClass: read(window, "resourceClass"),
            resourceName: read(window, "resourceName"),
            windowClass: read(window, "windowClass"),
            pid: read(window, "pid"),
            x: geo.x,
            y: geo.y,
            width: geo.width,
            height: geo.height,
            workspace: firstDesktop(window),
            minimized: read(window, "minimized"),
            active: read(window, "active") || window === activeWindow,
            clientType: clientType(window),
            normalWindow: read(window, "normalWindow"),
            desktopWindow: read(window, "desktopWindow"),
            skipTaskbar: read(window, "skipTaskbar"),
            dock: read(window, "dock")
        }};
    }});

    callDBus(serviceName, objectPath, iface, "ReceiveWindows", JSON.stringify({{
        backend: "kwin",
        pluginName: pluginName,
        windows: windows
    }}));
}})();
"#
    ))
}

fn write_kwin_activate_script(
    service_name: &str,
    callback_object_path: &str,
    plugin_name: &str,
    uuid: &str,
) -> Result<std::path::PathBuf> {
    let script =
        kwin_activate_script_source(service_name, callback_object_path, plugin_name, uuid)?;
    write_kwin_script_file(plugin_name, &script)
}

pub(crate) fn kwin_activate_script_source(
    service_name: &str,
    callback_object_path: &str,
    plugin_name: &str,
    uuid: &str,
) -> Result<String> {
    let target_uuid = normalize_kwin_uuid(uuid).context("KWin activation requires a uuid")?;
    let service_name = serde_json::to_string(service_name)?;
    let object_path = serde_json::to_string(callback_object_path)?;
    let interface = serde_json::to_string(KWIN_CALLBACK_INTERFACE)?;
    let plugin_name_json = serde_json::to_string(plugin_name)?;
    let target_uuid = serde_json::to_string(&target_uuid)?;

    Ok(format!(
        r#"(function() {{
    var serviceName = {service_name};
    var objectPath = {object_path};
    var iface = {interface};
    var pluginName = {plugin_name_json};
    var targetUuid = {target_uuid};

    function send(payload) {{
        payload.backend = "kwin";
        payload.pluginName = pluginName;
        callDBus(serviceName, objectPath, iface, "ReceiveResult", JSON.stringify(payload));
    }}

    function serialize(value) {{
        if (value === null || value === undefined) {{
            return null;
        }}
        if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") {{
            return value;
        }}
        try {{
            if (typeof value.toString === "function") {{
                return value.toString();
            }}
        }} catch (error) {{}}
        return null;
    }}

    function read(obj, key) {{
        try {{
            if (obj === null || obj === undefined) {{
                return null;
            }}
            var value = obj[key];
            if (typeof value === "function") {{
                return null;
            }}
            return serialize(value);
        }} catch (error) {{
            return null;
        }}
    }}

    function normalizeUuid(value) {{
        var text = serialize(value);
        if (text === null || text === undefined) {{
            return null;
        }}
        text = String(text).trim().toLowerCase();
        if (text.charAt(0) === "{{" && text.charAt(text.length - 1) === "}}") {{
            text = text.substring(1, text.length - 1);
        }}
        return text.length > 0 ? text : null;
    }}

    function windowUuid(window) {{
        return normalizeUuid(read(window, "uuid")) || normalizeUuid(read(window, "internalId"));
    }}

    function listWindows() {{
        try {{
            if (typeof workspace.windowList === "function") {{
                return workspace.windowList();
            }}
        }} catch (error) {{}}
        try {{
            if (workspace.stackingOrder && typeof workspace.stackingOrder.length === "number") {{
                return workspace.stackingOrder;
            }}
        }} catch (error) {{}}
        return [];
    }}

    function activateDesktop(window) {{
        var desktops = null;
        try {{
            desktops = window.desktops;
        }} catch (error) {{}}
        if (desktops && desktops.length > 0) {{
            try {{
                workspace.currentDesktop = desktops[0];
            }} catch (error) {{}}
        }}
    }}

    try {{
        var targetWindow = null;
        var windows = listWindows();
        for (var i = 0; i < windows.length; i++) {{
            if (windowUuid(windows[i]) === targetUuid) {{
                targetWindow = windows[i];
                break;
            }}
        }}

        if (!targetWindow) {{
            throw new Error("window not found: " + targetUuid);
        }}

        try {{
            targetWindow.minimized = false;
        }} catch (error) {{}}
        activateDesktop(targetWindow);

        var activated = false;
        var activationError = null;
        try {{
            workspace.activeWindow = targetWindow;
            activated = true;
        }} catch (error) {{
            activationError = error;
        }}
        if (!activated) {{
            try {{
                workspace.activeClient = targetWindow;
                activated = true;
            }} catch (error) {{
                activationError = error;
            }}
        }}
        if (!activated) {{
            try {{
                if (typeof targetWindow.activate === "function") {{
                    targetWindow.activate();
                    activated = true;
                }}
            }} catch (error) {{
                activationError = error;
            }}
        }}
        if (!activated) {{
            throw activationError || new Error("workspace refused activeWindow assignment");
        }}

        try {{
            if (typeof workspace.raiseWindow === "function") {{
                workspace.raiseWindow(targetWindow);
            }}
        }} catch (error) {{}}

        send({{
            ok: true,
            uuid: windowUuid(targetWindow)
        }});
    }} catch (error) {{
        send({{
            ok: false,
            error: String(error && error.message ? error.message : error)
        }});
    }}
}})();
"#
    ))
}

fn write_kwin_script_file(plugin_name: &str, script: &str) -> Result<std::path::PathBuf> {
    for attempt in 0..4 {
        let filename = if attempt == 0 {
            format!("{plugin_name}.js")
        } else {
            format!("{plugin_name}-{attempt}.js")
        };
        let path = std::env::temp_dir().join(filename);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                if let Err(error) = file.write_all(script.as_bytes()) {
                    let _ = fs::remove_file(&path);
                    return Err(error).with_context(|| {
                        format!("failed to write temporary KWin script {}", path.display())
                    });
                }
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to create temporary KWin script {}", path.display())
                });
            }
        }
    }

    bail!("failed to create a unique temporary KWin script path for {plugin_name}")
}

pub(crate) fn parse_kwin_windows(json: &str) -> Result<Vec<WindowInfo>> {
    let snapshot = parse_kwin_snapshot(json)?;
    let mut windows = snapshot
        .windows
        .into_iter()
        .filter(|window| !json_value_as_bool(window.desktop_window.as_ref()).unwrap_or(false))
        .filter(|window| !json_value_as_bool(window.dock.as_ref()).unwrap_or(false))
        .filter(|window| !json_value_as_bool(window.skip_taskbar.as_ref()).unwrap_or(false))
        .filter(|window| json_value_as_bool(window.normal_window.as_ref()).unwrap_or(true))
        .map(WindowInfo::try_from)
        .collect::<Result<Vec<_>>>()?;
    windows.sort_by_key(|window| window.window_id);
    Ok(windows)
}

fn parse_kwin_snapshot(json: &str) -> Result<KwinWindowSnapshot> {
    serde_json::from_str(json).context("failed to parse KWin temporary script output")
}

#[derive(Debug, Deserialize)]
struct KwinWindowSnapshot {
    windows: Vec<KwinRawWindow>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KwinRawWindow {
    uuid: Option<String>,
    internal_id: Option<String>,
    caption: Option<String>,
    desktop_file: Option<String>,
    resource_class: Option<String>,
    resource_name: Option<String>,
    window_class: Option<String>,
    pid: Option<serde_json::Value>,
    x: Option<serde_json::Value>,
    y: Option<serde_json::Value>,
    width: Option<serde_json::Value>,
    height: Option<serde_json::Value>,
    workspace: Option<serde_json::Value>,
    minimized: Option<serde_json::Value>,
    active: Option<serde_json::Value>,
    client_type: Option<String>,
    normal_window: Option<serde_json::Value>,
    desktop_window: Option<serde_json::Value>,
    skip_taskbar: Option<serde_json::Value>,
    dock: Option<serde_json::Value>,
}

impl KwinRawWindow {
    fn kwin_uuid(&self) -> Option<String> {
        self.uuid
            .as_deref()
            .or(self.internal_id.as_deref())
            .and_then(normalize_kwin_uuid)
    }
}

impl TryFrom<KwinRawWindow> for WindowInfo {
    type Error = anyhow::Error;

    fn try_from(window: KwinRawWindow) -> Result<Self> {
        let uuid = window
            .kwin_uuid()
            .context("KWin window did not include uuid or internalId")?;
        let width = json_value_as_u32(window.width.as_ref());
        let height = json_value_as_u32(window.height.as_ref());
        let bounds = width.zip(height).map(|(width, height)| WindowBounds {
            x: json_value_as_i32(window.x.as_ref()),
            y: json_value_as_i32(window.y.as_ref()),
            width,
            height,
        });
        let app_id = clean_string(window.desktop_file.as_deref())
            .or_else(|| clean_string(window.resource_class.as_deref()));
        let wm_class = clean_string(window.resource_class.as_deref())
            .or_else(|| clean_string(window.window_class.as_deref()))
            .or_else(|| clean_string(window.resource_name.as_deref()));
        let client_type = clean_string(window.client_type.as_deref());

        Ok(WindowInfo {
            window_id: kwin_window_id_from_uuid(&uuid),
            title: clean_string(window.caption.as_deref()),
            app_id,
            wm_class,
            pid: json_value_as_u32(window.pid.as_ref()),
            bounds,
            workspace: json_value_as_i32(window.workspace.as_ref()),
            focused: json_value_as_bool(window.active.as_ref()).unwrap_or(false),
            hidden: json_value_as_bool(window.minimized.as_ref()).unwrap_or(false),
            client_type,
            backend: KWIN_BACKEND.to_string(),
            terminal: None,
        })
    }
}

pub(crate) fn kwin_window_id_from_uuid(uuid: &str) -> u64 {
    let normalized = normalize_kwin_uuid(uuid).unwrap_or_else(|| uuid.trim().to_ascii_lowercase());
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in normalized.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn normalize_kwin_uuid(uuid: &str) -> Option<String> {
    let value = uuid
        .trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .trim()
        .to_ascii_lowercase();
    (!value.is_empty()).then_some(value)
}

fn clean_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "null")
        .map(ToOwned::to_owned)
}

fn json_value_as_bool(value: Option<&serde_json::Value>) -> Option<bool> {
    match value? {
        serde_json::Value::Bool(value) => Some(*value),
        serde_json::Value::String(value) => match value.to_ascii_lowercase().as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn json_value_as_u32(value: Option<&serde_json::Value>) -> Option<u32> {
    let value = json_value_as_f64(value)?;
    if !value.is_finite() || value < 0.0 || value > u32::MAX as f64 {
        return None;
    }
    Some(value.round() as u32)
}

fn json_value_as_i32(value: Option<&serde_json::Value>) -> Option<i32> {
    let value = json_value_as_f64(value)?;
    if !value.is_finite() || value < i32::MIN as f64 || value > i32::MAX as f64 {
        return None;
    }
    Some(value.round() as i32)
}

fn json_value_as_f64(value: Option<&serde_json::Value>) -> Option<f64> {
    match value? {
        serde_json::Value::Number(value) => value.as_f64(),
        serde_json::Value::String(value) => value.parse::<f64>().ok(),
        _ => None,
    }
}

struct ProbeCheck {
    ok: bool,
    detail: String,
}

fn gdbus_introspect_contains(
    destination: &str,
    object_path: &str,
    interface: &str,
    member: &str,
) -> ProbeCheck {
    match std::process::Command::new("gdbus")
        .args([
            "introspect",
            "--session",
            "--dest",
            destination,
            "--object-path",
            object_path,
        ])
        .output()
    {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let needle = format!("{interface}.{member}");
            let ok = stdout.contains(&needle) || stdout.contains(member);
            ProbeCheck {
                ok,
                detail: if ok {
                    format!("{interface}.{member} is present")
                } else {
                    format!("{interface}.{member} not found")
                },
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            ProbeCheck {
                ok: false,
                detail: if stderr.is_empty() { stdout } else { stderr },
            }
        }
        Err(error) => ProbeCheck {
            ok: false,
            detail: error.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_normal_window() {
        let json = r#"{
            "windows": [{
                "uuid": "{12345678-90AB-CDEF-1234-567890ABCDEF}",
                "caption": "  Dolphin  ",
                "desktopFile": "org.kde.dolphin",
                "resourceClass": "dolphin",
                "resourceName": "dolphin",
                "pid": 4321,
                "x": 10.0,
                "y": 20.0,
                "width": 800,
                "height": 600,
                "workspace": 2,
                "minimized": false,
                "active": true,
                "clientType": "wayland",
                "normalWindow": true
            }]
        }"#;
        let windows = parse_kwin_windows(json).expect("valid KWin JSON parses");
        assert_eq!(windows.len(), 1);
        let window = &windows[0];
        assert_eq!(window.title.as_deref(), Some("Dolphin"));
        assert_eq!(window.app_id.as_deref(), Some("org.kde.dolphin"));
        assert_eq!(window.wm_class.as_deref(), Some("dolphin"));
        assert_eq!(window.pid, Some(4321));
        assert_eq!(window.backend, KWIN_BACKEND);
        assert!(window.focused);
        assert!(!window.hidden);
        let bounds = window.bounds.as_ref().expect("bounds present");
        assert_eq!(
            (bounds.x, bounds.y, bounds.width, bounds.height),
            (Some(10), Some(20), 800, 600)
        );
    }

    #[test]
    fn filters_out_desktop_dock_and_skip_taskbar_windows() {
        let json = r#"{
            "windows": [
                {"uuid": "a", "desktopWindow": true},
                {"uuid": "b", "dock": true},
                {"uuid": "c", "skipTaskbar": true},
                {"uuid": "d", "caption": "Real", "normalWindow": true}
            ]
        }"#;
        let windows = parse_kwin_windows(json).expect("parses");
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].title.as_deref(), Some("Real"));
    }

    #[test]
    fn keeps_windows_when_normal_window_flag_is_absent() {
        // normalWindow defaults to true when missing, so the window is kept.
        let json = r#"{"windows": [{"uuid": "a", "caption": "NoFlag"}]}"#;
        let windows = parse_kwin_windows(json).expect("parses");
        assert_eq!(windows.len(), 1);
    }

    #[test]
    fn falls_back_to_internal_id_when_uuid_missing() {
        let json =
            r#"{"windows": [{"internalId": "{DEAD-BEEF}", "caption": "X", "normalWindow": true}]}"#;
        let windows = parse_kwin_windows(json).expect("parses");
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].window_id, kwin_window_id_from_uuid("dead-beef"));
    }

    #[test]
    fn window_without_any_uuid_is_a_hard_error() {
        let json = r#"{"windows": [{"caption": "X", "normalWindow": true}]}"#;
        assert!(parse_kwin_windows(json).is_err());
    }

    #[test]
    fn windows_are_sorted_by_derived_id_ascending() {
        // Input order (zzz, aaa) must be re-sorted to ascending derived id so the
        // window list is stable across queries regardless of KWin's iteration order.
        let json = r#"{
            "windows": [
                {"uuid": "zzz", "caption": "Z", "normalWindow": true},
                {"uuid": "aaa", "caption": "A", "normalWindow": true}
            ]
        }"#;
        let windows = parse_kwin_windows(json).expect("parses");
        assert_eq!(windows.len(), 2);
        let id_aaa = kwin_window_id_from_uuid("aaa");
        let id_zzz = kwin_window_id_from_uuid("zzz");
        let (min, max) = if id_aaa < id_zzz {
            (id_aaa, id_zzz)
        } else {
            (id_zzz, id_aaa)
        };
        assert_eq!(windows[0].window_id, min, "smallest id must come first");
        assert_eq!(windows[1].window_id, max);
    }

    #[test]
    fn parses_numeric_fields_delivered_as_strings() {
        // KWin's scripting bridge frequently serializes numbers as strings; the
        // typed WindowInfo fields must still come through as real integers. This
        // guards the string branch of the JSON coercions end-to-end, not just in
        // isolation — dropping it would silently blank out pid/bounds on Plasma.
        let json = r#"{
            "windows": [{
                "uuid": "a",
                "caption": "Konsole",
                "pid": "9182",
                "x": "100",
                "y": "-40",
                "width": "1024",
                "height": "768",
                "workspace": "3",
                "normalWindow": true
            }]
        }"#;
        let windows = parse_kwin_windows(json).expect("parses");
        let window = &windows[0];
        assert_eq!(window.pid, Some(9182));
        assert_eq!(window.workspace, Some(3));
        let bounds = window.bounds.as_ref().expect("bounds present");
        assert_eq!(
            (bounds.x, bounds.y, bounds.width, bounds.height),
            (Some(100), Some(-40), 1024, 768)
        );
    }

    #[test]
    fn minimized_window_maps_to_hidden() {
        // `minimized` drives the `hidden` flag callers use to skip off-screen
        // windows; a regression flipping this mapping is invisible to a raw
        // coverage number but breaks window selection.
        let json = r#"{"windows": [{"uuid": "a", "minimized": true, "active": false, "normalWindow": true}]}"#;
        let windows = parse_kwin_windows(json).expect("parses");
        assert!(windows[0].hidden);
        assert!(!windows[0].focused);
    }

    #[test]
    fn boolean_filter_fields_delivered_as_strings_still_filter() {
        // The desktop/dock/skipTaskbar filters must honor the stringified bool
        // form KWin can emit, otherwise panels leak into the window list.
        let json = r#"{
            "windows": [
                {"uuid": "a", "skipTaskbar": "true"},
                {"uuid": "b", "caption": "Real", "normalWindow": "true"}
            ]
        }"#;
        let windows = parse_kwin_windows(json).expect("parses");
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].title.as_deref(), Some("Real"));
    }

    #[test]
    fn malformed_json_is_reported_not_panicked() {
        assert!(parse_kwin_windows("not json").is_err());
        assert!(parse_kwin_windows("{}").is_err());
    }

    #[test]
    fn app_id_falls_back_to_resource_class_without_desktop_file() {
        let json = r#"{"windows": [{"uuid": "a", "resourceClass": "kate", "normalWindow": true}]}"#;
        let windows = parse_kwin_windows(json).expect("parses");
        assert_eq!(windows[0].app_id.as_deref(), Some("kate"));
    }

    #[test]
    fn window_id_from_uuid_is_stable_and_case_insensitive() {
        let braced = kwin_window_id_from_uuid("{ABCD-1234}");
        let bare = kwin_window_id_from_uuid("abcd-1234");
        assert_eq!(braced, bare, "braces and case must not change the id");
        assert_ne!(
            kwin_window_id_from_uuid("abcd-1234"),
            kwin_window_id_from_uuid("abcd-1235"),
            "distinct uuids must yield distinct ids"
        );
    }

    #[test]
    fn normalize_uuid_strips_braces_and_rejects_empty() {
        assert_eq!(normalize_kwin_uuid("{ABC}").as_deref(), Some("abc"));
        assert_eq!(normalize_kwin_uuid("  def  ").as_deref(), Some("def"));
        assert_eq!(normalize_kwin_uuid("{}"), None);
        assert_eq!(normalize_kwin_uuid("   "), None);
    }

    #[test]
    fn clean_string_drops_empty_and_literal_null() {
        assert_eq!(clean_string(Some("  hi  ")).as_deref(), Some("hi"));
        assert_eq!(clean_string(Some("")), None);
        assert_eq!(clean_string(Some("   ")), None);
        assert_eq!(clean_string(Some("null")), None);
        assert_eq!(clean_string(None), None);
    }

    #[test]
    fn json_value_coercions_accept_numbers_and_numeric_strings() {
        use serde_json::json;
        assert_eq!(json_value_as_u32(Some(&json!(42))), Some(42));
        assert_eq!(json_value_as_u32(Some(&json!("42"))), Some(42));
        assert_eq!(json_value_as_u32(Some(&json!(42.7))), Some(43));
        // Out-of-range / negative values for u32 are rejected.
        assert_eq!(json_value_as_u32(Some(&json!(-1))), None);
        assert_eq!(json_value_as_i32(Some(&json!(-5))), Some(-5));
        assert_eq!(json_value_as_i32(Some(&json!("-5"))), Some(-5));
        // Bools accept native and stringified forms.
        assert_eq!(json_value_as_bool(Some(&json!(true))), Some(true));
        assert_eq!(json_value_as_bool(Some(&json!("false"))), Some(false));
        assert_eq!(json_value_as_bool(Some(&json!("maybe"))), None);
        assert_eq!(json_value_as_bool(Some(&json!(1))), None);
    }

    #[test]
    fn bounds_are_dropped_when_width_or_height_missing() {
        let json =
            r#"{"windows": [{"uuid": "a", "x": 5, "y": 5, "width": 100, "normalWindow": true}]}"#;
        let windows = parse_kwin_windows(json).expect("parses");
        assert!(
            windows[0].bounds.is_none(),
            "missing height must drop bounds"
        );
    }
}

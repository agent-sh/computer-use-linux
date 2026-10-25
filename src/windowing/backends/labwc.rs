use crate::windowing::registry::BackendProbe;
use crate::windowing::types::{WindowBounds, WindowInfo};
use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

pub const LABWC_BACKEND: &str = "labwc";
const REPLY_CAP: usize = 4 * 1024 * 1024;
const QUERY_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Deserialize)]
struct LabwcWindow {
    pid: u32,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    title: Option<String>,
    app_id: Option<String>,
    minimized: bool,
    focused: bool,
}

pub(crate) fn window_id(pid: u32, app_id: Option<&str>) -> u64 {
    // shortcut: two windows that share pid and app_id collide (Chrome). Upgrade when labwc exposes an id.
    let mut id = 0xcbf29ce484222325u64;
    for byte in pid
        .to_le_bytes()
        .into_iter()
        .chain(app_id.unwrap_or("").bytes())
    {
        id ^= u64::from(byte);
        id = id.wrapping_mul(0x100000001b3);
    }
    id
}

pub(crate) fn parse_windows(json: &str) -> Result<Vec<WindowInfo>> {
    let value: serde_json::Value =
        serde_json::from_str(json).context("labwc reply was not JSON")?;
    if let Some(error) = value.get("error").and_then(|item| item.as_str()) {
        bail!("{error}");
    }
    let windows: Vec<LabwcWindow> =
        serde_json::from_value(value).context("labwc window list had an unexpected shape")?;
    Ok(windows.into_iter().map(to_window).collect())
}

pub(crate) fn parse_window(json: &str) -> Result<Option<WindowInfo>> {
    let value: serde_json::Value =
        serde_json::from_str(json).context("labwc reply was not JSON")?;
    if value.is_null() || value.as_object().is_some_and(|object| object.is_empty()) {
        return Ok(None);
    }
    if let Some(error) = value.get("error").and_then(|item| item.as_str()) {
        bail!("{error}");
    }
    let window: LabwcWindow =
        serde_json::from_value(value).context("labwc focused window had an unexpected shape")?;
    Ok(Some(to_window(window)))
}

fn to_window(window: LabwcWindow) -> WindowInfo {
    let app_id = window.app_id.filter(|value| !value.is_empty());
    WindowInfo {
        window_id: window_id(window.pid, app_id.as_deref()),
        title: window.title.filter(|value| !value.is_empty()),
        wm_class: app_id.clone(),
        app_id: app_id.clone(),
        pid: Some(window.pid),
        bounds: Some(WindowBounds {
            x: Some(window.x),
            y: Some(window.y),
            width: window.width,
            height: window.height,
        }),
        workspace: None,
        focused: window.focused,
        hidden: window.minimized,
        client_type: None,
        backend: LABWC_BACKEND.to_string(),
        terminal: None,
    }
}

pub fn probe() -> BackendProbe {
    let path = socket_path();
    let ok = path.as_ref().is_some_and(|path| path.exists());
    BackendProbe {
        id: LABWC_BACKEND,
        ok,
        can_list_windows: ok,
        can_focus_apps: false,
        can_focus_windows: ok,
        detail: match path {
            Some(path) if ok => format!("socket {}", path.display()),
            Some(path) => format!("no socket at {}", path.display()),
            None => "XDG_RUNTIME_DIR is unset".to_string(),
        },
    }
}

pub(crate) async fn list_windows() -> Result<Vec<WindowInfo>> {
    parse_windows(&query("GET_WINDOWS").await?)
}

pub(crate) async fn focused_window() -> Result<Option<WindowInfo>> {
    parse_window(&query("GET_FOCUSED_WINDOW").await?)
}

pub(crate) async fn activate(window: &WindowInfo) -> Result<()> {
    crate::windowing::backends::labwc::foreign::activate(window).await
}

fn socket_path() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    Some(PathBuf::from(runtime).join("labwc.sock"))
}

async fn query(command: &str) -> Result<String> {
    let path = socket_path().context("XDG_RUNTIME_DIR is unset")?;
    let connect = UnixStream::connect(&path);
    let mut stream = tokio::time::timeout(QUERY_TIMEOUT, connect)
        .await
        .context("labwc socket connect timed out")?
        .with_context(|| format!("could not connect to {}", path.display()))?;
    stream
        .write_all(format!("{command}\n").as_bytes())
        .await
        .context("labwc socket write failed")?;
    let _ = stream.shutdown().await;
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    loop {
        let read = stream.read(&mut tmp);
        let n = tokio::time::timeout(QUERY_TIMEOUT, read)
            .await
            .context("labwc socket read timed out")?
            .context("labwc socket read failed")?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.len() > REPLY_CAP {
            bail!("labwc reply exceeded 4 MiB");
        }
    }
    let text = String::from_utf8(buf).context("labwc reply was not UTF-8")?;
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
        if let Some(error) = value.get("error").and_then(|item| item.as_str()) {
            return Err(anyhow!(error.to_string()));
        }
    }
    Ok(text)
}

mod foreign {
    use super::WindowInfo;
    use anyhow::{bail, Context, Result};
    use std::collections::HashMap;
    use wayland_client::globals::{registry_queue_init, GlobalListContents};
    use wayland_client::protocol::{wl_registry, wl_seat};
    use wayland_client::{event_created_child, Connection, Dispatch, Proxy, QueueHandle};
    use wayland_protocols_wlr::foreign_toplevel::v1::client::{
        zwlr_foreign_toplevel_handle_v1, zwlr_foreign_toplevel_manager_v1,
    };

    pub(super) async fn activate(window: &WindowInfo) -> Result<()> {
        let title = window.title.clone();
        let app_id = window.app_id.clone();
        tokio::task::spawn_blocking(move || activate_blocking(title.as_deref(), app_id.as_deref()))
            .await
            .context("labwc activation task failed")?
    }

    fn activate_blocking(title: Option<&str>, app_id: Option<&str>) -> Result<()> {
        let conn = Connection::connect_to_env().context("no Wayland display")?;
        let (globals, mut queue) = registry_queue_init(&conn).context("Wayland registry failed")?;
        let mut state = State::default();
        let qh = queue.handle();
        state.manager = globals
            .bind::<zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1, _, _>(
                &qh,
                1..=3,
                (),
            )
            .ok();
        if state.manager.is_none() {
            bail!("activation unsupported");
        }
        state.seat = globals.bind::<wl_seat::WlSeat, _, _>(&qh, 1..=1, ()).ok();
        queue
            .roundtrip(&mut state)
            .context("Wayland roundtrip failed")?;
        let Some(seat) = state.seat.as_ref() else {
            bail!("activation unsupported");
        };
        // The handle proxy is owned by the map. Activate through the stored proxy.
        if let Some(proxy) = state.proxies.iter().find(|proxy| {
            state
                .handles
                .get(&proxy.id().protocol_id())
                .is_some_and(|handle| {
                    (app_id.is_none() || handle.app_id.as_deref() == app_id)
                        && (title.is_none() || handle.title.as_deref() == title)
                })
        }) {
            proxy.activate(seat);
            let _ = queue.roundtrip(&mut state);
            return Ok(());
        }
        bail!("labwc has no toplevel matching the window");
    }

    #[derive(Default)]
    struct State {
        manager: Option<zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1>,
        seat: Option<wl_seat::WlSeat>,
        finished: bool,
        handles: HashMap<u32, HandleState>,
        proxies: Vec<zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1>,
    }

    #[derive(Default)]
    struct HandleState {
        title: Option<String>,
        app_id: Option<String>,
    }

    impl Dispatch<zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1, ()> for State {
        fn event(
            state: &mut Self,
            _: &zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1,
            event: zwlr_foreign_toplevel_manager_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } => {
                    let id = toplevel.id().protocol_id();
                    state.handles.entry(id).or_default();
                    state.proxies.push(toplevel);
                }
                zwlr_foreign_toplevel_manager_v1::Event::Finished => state.finished = true,
                _ => {}
            }
        }

        event_created_child!(
            State,
            zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1,
            [zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE => (zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1, ())]
        );
    }

    impl Dispatch<zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1, ()> for State {
        fn event(
            state: &mut Self,
            handle: &zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1,
            event: zwlr_foreign_toplevel_handle_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            let slot = state.handles.entry(handle.id().protocol_id()).or_default();
            match event {
                zwlr_foreign_toplevel_handle_v1::Event::Title { title } => slot.title = Some(title),
                zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                    slot.app_id = Some(app_id)
                }
                zwlr_foreign_toplevel_handle_v1::Event::Closed => {
                    state.handles.remove(&handle.id().protocol_id());
                }
                _ => {}
            }
        }
    }

    impl Dispatch<wl_seat::WlSeat, ()> for State {
        fn event(
            _: &mut Self,
            _: &wl_seat::WlSeat,
            _: wl_seat::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
        fn event(
            _: &mut Self,
            _: &wl_registry::WlRegistry,
            _: wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"[{"pid":510,"x":0,"y":0,"width":1024,"height":768,"title":"New Tab - Google Chrome","app_id":"google-chrome","minimized":false,"fullscreen":false,"maximized":true,"tiled":false,"focused":true}]"#;

    #[test]
    fn parses_labwc_windows() {
        let windows = parse_windows(FIXTURE).unwrap();
        assert_eq!(windows.len(), 1);
        let window = &windows[0];
        assert_eq!(window.title.as_deref(), Some("New Tab - Google Chrome"));
        assert_eq!(window.app_id.as_deref(), Some("google-chrome"));
        assert_eq!(window.pid, Some(510));
        let bounds = window.bounds.as_ref().unwrap();
        assert_eq!(bounds.x, Some(0));
        assert_eq!(bounds.y, Some(0));
        assert_eq!((bounds.width, bounds.height), (1024, 768));
        assert!(window.focused);
        assert!(!window.hidden);
        assert_eq!(window.backend, "labwc");
        assert!(window.client_type.is_none());
    }

    #[test]
    fn labwc_error_reply_is_err() {
        let error = parse_windows(r#"{"error":"unknown command"}"#).unwrap_err();
        assert!(error.to_string().contains("unknown command"), "{error}");
    }

    #[test]
    fn empty_focused_reply_is_none() {
        assert!(parse_window("{}").unwrap().is_none());
        assert!(parse_window("null").unwrap().is_none());
    }

    #[test]
    fn window_id_is_stable() {
        assert_eq!(
            window_id(510, Some("google-chrome")),
            window_id(510, Some("google-chrome"))
        );
        assert_ne!(
            window_id(510, Some("google-chrome")),
            window_id(511, Some("google-chrome"))
        );
    }
}

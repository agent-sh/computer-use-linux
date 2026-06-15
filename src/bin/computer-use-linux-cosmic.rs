use anyhow::{anyhow, bail, Context, Result};
use cosmic_protocols::{
    toplevel_info::v1::client::{zcosmic_toplevel_handle_v1, zcosmic_toplevel_info_v1},
    toplevel_management::v1::client::zcosmic_toplevel_manager_v1,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use wayland_client::{
    event_created_child,
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_registry, wl_seat},
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
    ext_foreign_toplevel_handle_v1, ext_foreign_toplevel_list_v1,
};

const HELP: &str = "computer-use-linux-cosmic\n\nUsage:\n  computer-use-linux-cosmic probe\n  computer-use-linux-cosmic list-windows\n  computer-use-linux-cosmic focused-window\n  computer-use-linux-cosmic activate-window --window-id <id>";
const BACKEND: &str = "cosmic-wayland";
const ACTIVATION_STATE_TTL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WindowInfo {
    window_id: u64,
    title: Option<String>,
    app_id: Option<String>,
    wm_class: Option<String>,
    pid: Option<u32>,
    bounds: Option<WindowBounds>,
    workspace: Option<i32>,
    focused: bool,
    hidden: bool,
    client_type: Option<String>,
    backend: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WindowBounds {
    x: Option<i32>,
    y: Option<i32>,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProbeOutput {
    ok: bool,
    can_list_windows: bool,
    can_activate_windows: bool,
    detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActivationOutput {
    ok: bool,
    detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActivationState {
    window_id: u64,
    timestamp_ms: u64,
}

#[derive(Debug, Clone, Default)]
struct ToplevelRecord {
    foreign: Option<ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1>,
    cosmic: Option<zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1>,
    identifier: Option<String>,
    title: Option<String>,
    app_id: Option<String>,
    focused: bool,
    hidden: bool,
}

impl ToplevelRecord {
    fn to_window(&self) -> Option<WindowInfo> {
        let identifier = self.identifier.as_deref()?;
        Some(WindowInfo {
            window_id: stable_window_id(identifier),
            title: self.title.clone().filter(|value| !value.trim().is_empty()),
            app_id: self.app_id.clone().filter(|value| !value.trim().is_empty()),
            wm_class: None,
            pid: None,
            bounds: None,
            workspace: None,
            focused: self.focused,
            hidden: self.hidden,
            client_type: Some("wayland".to_string()),
            backend: BACKEND.to_string(),
        })
    }
}

#[derive(Default)]
struct AppData {
    toplevel_info: Option<zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1>,
    toplevel_manager: Option<zcosmic_toplevel_manager_v1::ZcosmicToplevelManagerV1>,
    seats: Vec<wl_seat::WlSeat>,
    capabilities:
        Vec<WEnum<zcosmic_toplevel_manager_v1::ZcosmicToplelevelManagementCapabilitiesV1>>,
    records: Vec<ToplevelRecord>,
    by_foreign_id: HashMap<u32, usize>,
    by_cosmic_id: HashMap<u32, usize>,
}

fn main() -> Result<()> {
    match Command::parse(std::env::args().skip(1).collect())? {
        Command::Probe => print_json(&probe()?),
        Command::ListWindows => print_json(&collect_windows()?),
        Command::FocusedWindow => print_json(&focused_window()?),
        Command::ActivateWindow { window_id } => print_json(&activate_window(window_id)?),
    }
}

#[derive(Debug)]
enum Command {
    Probe,
    ListWindows,
    FocusedWindow,
    ActivateWindow { window_id: u64 },
}

impl Command {
    fn parse(args: Vec<String>) -> Result<Self> {
        match args.as_slice() {
            [command] if command == "probe" => Ok(Self::Probe),
            [command] if command == "list-windows" => Ok(Self::ListWindows),
            [command] if command == "focused-window" => Ok(Self::FocusedWindow),
            [command, flag, value] if command == "activate-window" && flag == "--window-id" => {
                Ok(Self::ActivateWindow {
                    window_id: value
                        .parse::<u64>()
                        .with_context(|| format!("invalid window id {value}"))?,
                })
            }
            [command] if command == "--help" || command == "-h" => {
                println!("{HELP}");
                std::process::exit(0);
            }
            [] => {
                println!("{HELP}");
                std::process::exit(0);
            }
            _ => bail!("unknown arguments. Expected one of: probe, list-windows, focused-window, activate-window --window-id <id>"),
        }
    }
}

fn probe() -> Result<ProbeOutput> {
    let snapshot = Snapshot::collect()?;
    let windows = snapshot.windows();
    let can_activate = snapshot.can_activate_windows();
    Ok(ProbeOutput {
        ok: !windows.is_empty(),
        can_list_windows: !windows.is_empty(),
        can_activate_windows: can_activate,
        detail: if !windows.is_empty() {
            if can_activate {
                format!(
                    "COSMIC foreign toplevel listing is available and activation is supported for {} window(s).",
                    windows.len()
                )
            } else {
                format!(
                    "COSMIC foreign toplevel listing is available for {} window(s), but activation support is incomplete.",
                    windows.len()
                )
            }
        } else {
            "COSMIC foreign toplevel listing is unavailable in this session.".to_string()
        },
    })
}

fn collect_windows() -> Result<Vec<WindowInfo>> {
    Ok(Snapshot::collect()?.windows())
}

fn focused_window() -> Result<Option<WindowInfo>> {
    let snapshot = Snapshot::collect()?;
    if let Some(window) = snapshot.windows().into_iter().find(|window| window.focused) {
        clear_activation_state();
        return Ok(Some(window));
    }

    let Some(state) = read_activation_state() else {
        return Ok(None);
    };

    if state_is_stale(&state) {
        clear_activation_state();
        return Ok(None);
    }

    let mut window = snapshot
        .windows()
        .into_iter()
        .find(|window| window.window_id == state.window_id);
    if let Some(window) = window.as_mut() {
        window.focused = true;
    }
    Ok(window)
}

fn activate_window(window_id: u64) -> Result<ActivationOutput> {
    let mut snapshot = Snapshot::collect()?;
    snapshot.activate(window_id)?;
    write_activation_state(window_id)?;
    Ok(ActivationOutput {
        ok: true,
        detail: format!("Requested COSMIC activation for window_id {window_id}."),
    })
}

struct Snapshot {
    event_queue: wayland_client::EventQueue<AppData>,
    app_data: AppData,
}

impl Snapshot {
    fn collect() -> Result<Self> {
        let conn = Connection::connect_to_env().context("failed to connect to Wayland display")?;
        let (globals, event_queue) =
            registry_queue_init(&conn).context("failed to initialize Wayland registry queue")?;
        let mut snapshot = Self {
            event_queue,
            app_data: AppData::default(),
        };
        let qh = snapshot.event_queue.handle();
        snapshot.app_data.toplevel_info = globals
            .bind::<zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1, _, _>(&qh, 1..=3, ())
            .ok();
        snapshot.app_data.toplevel_manager = globals
            .bind::<zcosmic_toplevel_manager_v1::ZcosmicToplevelManagerV1, _, _>(&qh, 1..=4, ())
            .ok();
        globals.contents().with_list(|entries| {
            for global in entries {
                if global.interface == "wl_seat" {
                    snapshot
                        .app_data
                        .seats
                        .push(globals.registry().bind::<wl_seat::WlSeat, _, _>(
                            global.name,
                            global.version.min(9),
                            &qh,
                            (),
                        ));
                }
            }
        });
        if snapshot.app_data.toplevel_info.is_some() {
            let _ = globals
                .bind::<ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1, _, _>(
                    &qh,
                    1..=1,
                    (),
                )
                .ok();
        }
        snapshot.prime()?;
        Ok(snapshot)
    }

    fn prime(&mut self) -> Result<()> {
        for _ in 0..4 {
            self.event_queue
                .roundtrip(&mut self.app_data)
                .context("Wayland roundtrip failed")?;
        }
        Ok(())
    }

    fn windows(&self) -> Vec<WindowInfo> {
        self.app_data
            .records
            .iter()
            .filter_map(ToplevelRecord::to_window)
            .collect()
    }

    fn can_activate_windows(&self) -> bool {
        !self.app_data.seats.is_empty()
            && self.app_data.toplevel_manager.is_some()
            && self.app_data.records.iter().any(|record| record.cosmic.is_some())
            && self.app_data.capabilities.iter().any(|capability| {
                matches!(
                    capability,
                    WEnum::Value(
                        zcosmic_toplevel_manager_v1::ZcosmicToplelevelManagementCapabilitiesV1::Activate
                    )
                )
            })
    }

    fn activate(&mut self, window_id: u64) -> Result<()> {
        if !self.can_activate_windows() {
            bail!("COSMIC activation capability is unavailable");
        }
        let seat = self
            .app_data
            .seats
            .first()
            .cloned()
            .ok_or_else(|| anyhow!("no wl_seat available for activation"))?;
        let record = self
            .app_data
            .records
            .iter()
            .find(|record| {
                record
                    .identifier
                    .as_deref()
                    .is_some_and(|id| stable_window_id(id) == window_id)
            })
            .ok_or_else(|| anyhow!("no COSMIC toplevel matched window_id {window_id}"))?;
        let cosmic = record
            .cosmic
            .as_ref()
            .ok_or_else(|| anyhow!("matched window has no COSMIC activation handle"))?;
        let manager = self
            .app_data
            .toplevel_manager
            .as_ref()
            .ok_or_else(|| anyhow!("COSMIC toplevel management protocol not advertised"))?;
        manager.activate(cosmic, &seat);
        self.event_queue
            .roundtrip(&mut self.app_data)
            .context("Wayland roundtrip after activation failed")?;
        Ok(())
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for AppData {
    fn event(
        app_data: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version: _,
        } = event
        {
            match interface.as_str() {
                "ext_foreign_toplevel_list_v1" => {
                    registry.bind::<ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1, _, _>(
                        name,
                        1,
                        qh,
                        (),
                    );
                }
                "zcosmic_toplevel_info_v1" => {
                    app_data.toplevel_info = Some(
                        registry.bind::<zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1, _, _>(
                            name,
                            3,
                            qh,
                            (),
                        ),
                    );
                }
                "zcosmic_toplevel_manager_v1" => {
                    app_data.toplevel_manager = Some(
                        registry
                            .bind::<zcosmic_toplevel_manager_v1::ZcosmicToplevelManagerV1, _, _>(
                                name,
                                4,
                                qh,
                                (),
                            ),
                    );
                }
                "wl_seat" => {
                    app_data
                        .seats
                        .push(registry.bind::<wl_seat::WlSeat, _, _>(name, 9, qh, ()));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1, ()> for AppData {
    fn event(
        app_data: &mut Self,
        _list: &ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } => {
                let foreign_id = toplevel.id().protocol_id();
                let mut record = ToplevelRecord {
                    foreign: Some(toplevel.clone()),
                    ..Default::default()
                };
                if let Some(info) = app_data.toplevel_info.as_ref() {
                    let cosmic = info.get_cosmic_toplevel(&toplevel, qh, ());
                    app_data
                        .by_cosmic_id
                        .insert(cosmic.id().protocol_id(), app_data.records.len());
                    record.cosmic = Some(cosmic);
                }
                app_data
                    .by_foreign_id
                    .insert(foreign_id, app_data.records.len());
                app_data.records.push(record);
            }
            ext_foreign_toplevel_list_v1::Event::Finished => {}
            _ => unreachable!(),
        }
    }

    event_created_child!(
        AppData,
        ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1,
        [
            ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1, ()),
        ]
    );
}

impl Dispatch<ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1, ()> for AppData {
    fn event(
        app_data: &mut Self,
        handle: &ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let Some(index) = app_data
            .by_foreign_id
            .get(&handle.id().protocol_id())
            .copied()
        else {
            return;
        };
        let record = &mut app_data.records[index];
        match event {
            ext_foreign_toplevel_handle_v1::Event::Identifier { identifier } => {
                record.identifier = Some(identifier);
            }
            ext_foreign_toplevel_handle_v1::Event::Title { title } => {
                record.title = Some(title);
            }
            ext_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                record.app_id = Some(app_id);
            }
            ext_foreign_toplevel_handle_v1::Event::Done => {}
            ext_foreign_toplevel_handle_v1::Event::Closed => {
                app_data.records[index].foreign = None;
                app_data.records[index].cosmic = None;
            }
            _ => unreachable!(),
        }
    }
}

impl Dispatch<zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1, ()> for AppData {
    fn event(
        _app_data: &mut Self,
        _info: &zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1,
        _event: zcosmic_toplevel_info_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }

    event_created_child!(
        AppData,
        zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1,
        [
            zcosmic_toplevel_info_v1::EVT_TOPLEVEL_OPCODE => (zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1, ()),
        ]
    );
}

impl Dispatch<zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1, ()> for AppData {
    fn event(
        app_data: &mut Self,
        handle: &zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1,
        event: zcosmic_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(index) = app_data
            .by_cosmic_id
            .get(&handle.id().protocol_id())
            .copied()
        else {
            return;
        };
        let record = &mut app_data.records[index];
        match event {
            zcosmic_toplevel_handle_v1::Event::State { state } => {
                record.focused = false;
                record.hidden = false;
                for value in state.chunks_exact(4) {
                    if let Ok(parsed) = zcosmic_toplevel_handle_v1::State::try_from(
                        u32::from_ne_bytes(value.try_into().unwrap()),
                    ) {
                        if parsed == zcosmic_toplevel_handle_v1::State::Activated {
                            record.focused = true;
                        }
                        if parsed == zcosmic_toplevel_handle_v1::State::Minimized {
                            record.hidden = true;
                        }
                    }
                }
            }
            zcosmic_toplevel_handle_v1::Event::Geometry { .. }
            | zcosmic_toplevel_handle_v1::Event::OutputEnter { .. }
            | zcosmic_toplevel_handle_v1::Event::OutputLeave { .. }
            | zcosmic_toplevel_handle_v1::Event::WorkspaceEnter { .. }
            | zcosmic_toplevel_handle_v1::Event::WorkspaceLeave { .. }
            | zcosmic_toplevel_handle_v1::Event::ExtWorkspaceEnter { .. }
            | zcosmic_toplevel_handle_v1::Event::ExtWorkspaceLeave { .. }
            | zcosmic_toplevel_handle_v1::Event::Title { .. }
            | zcosmic_toplevel_handle_v1::Event::AppId { .. }
            | zcosmic_toplevel_handle_v1::Event::Done
            | zcosmic_toplevel_handle_v1::Event::Closed => {}
            _ => unreachable!(),
        }
    }
}

impl Dispatch<zcosmic_toplevel_manager_v1::ZcosmicToplevelManagerV1, ()> for AppData {
    fn event(
        app_data: &mut Self,
        _manager: &zcosmic_toplevel_manager_v1::ZcosmicToplevelManagerV1,
        event: zcosmic_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zcosmic_toplevel_manager_v1::Event::Capabilities { capabilities } => {
                app_data.capabilities = capabilities
                    .chunks(4)
                    .map(|chunk| WEnum::from(u32::from_ne_bytes(chunk.try_into().unwrap())))
                    .collect();
            }
            _ => unreachable!(),
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for AppData {
    fn event(
        _app_data: &mut Self,
        _seat: &wl_seat::WlSeat,
        _event: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

fn stable_window_id(identifier: &str) -> u64 {
    fnv1a_64(identifier.as_bytes())
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn print_json<T: Serialize>(value: &T) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(value).context("failed to serialize JSON output")?
    );
    Ok(())
}

fn activation_state_path() -> PathBuf {
    if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        PathBuf::from(runtime_dir).join("computer-use-linux-cosmic-last-activation.json")
    } else {
        std::env::temp_dir().join("computer-use-linux-cosmic-last-activation.json")
    }
}

fn write_activation_state(window_id: u64) -> Result<()> {
    let state = ActivationState {
        window_id,
        timestamp_ms: now_timestamp_ms()?,
    };
    let path = activation_state_path();
    let json = serde_json::to_vec(&state).context("failed to serialize activation state")?;
    std::fs::write(&path, json)
        .with_context(|| format!("failed to write activation state to {}", path.display()))
}

fn read_activation_state() -> Option<ActivationState> {
    let path = activation_state_path();
    let contents = std::fs::read(&path).ok()?;
    serde_json::from_slice(&contents).ok()
}

fn clear_activation_state() {
    let _ = std::fs::remove_file(activation_state_path());
}

fn state_is_stale(state: &ActivationState) -> bool {
    let Ok(now_ms) = now_timestamp_ms() else {
        return false;
    };
    now_ms.saturating_sub(state.timestamp_ms) > ACTIVATION_STATE_TTL.as_millis() as u64
}

fn now_timestamp_ms() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before UNIX_EPOCH")?
        .as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_activate_window_args_requires_numeric_id() {
        let error = Command::parse(vec![
            "activate-window".to_string(),
            "--window-id".to_string(),
            "nope".to_string(),
        ])
        .unwrap_err()
        .to_string();

        assert!(error.contains("invalid window id"));
    }

    #[test]
    fn stable_window_id_is_stable() {
        assert_eq!(stable_window_id("window-1"), stable_window_id("window-1"));
    }

    #[test]
    fn activation_state_expires_after_ttl() {
        let state = ActivationState {
            window_id: 7,
            timestamp_ms: now_timestamp_ms().unwrap()
                - (ACTIVATION_STATE_TTL.as_millis() as u64 + 1),
        };

        assert!(state_is_stale(&state));
    }

    #[test]
    fn activation_state_is_fresh_within_ttl() {
        let state = ActivationState {
            window_id: 7,
            timestamp_ms: now_timestamp_ms().unwrap(),
        };

        assert!(!state_is_stale(&state));
    }
}

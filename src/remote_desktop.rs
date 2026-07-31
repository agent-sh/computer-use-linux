use crate::{command_runner, diagnostics::hydrate_session_bus_env};
use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
    time::Duration,
};
use tokio::process::Command;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use xkeysym::Keysym;
use zbus::{
    proxy::SignalStream,
    zvariant::{OwnedObjectPath, OwnedValue, Value},
    Connection, Proxy,
};

const PORTAL_DESKTOP_SERVICE: &str = "org.freedesktop.portal.Desktop";
const PORTAL_DESKTOP_PATH: &str = "/org/freedesktop/portal/desktop";
const PORTAL_REMOTE_DESKTOP_INTERFACE: &str = "org.freedesktop.portal.RemoteDesktop";
const PORTAL_SCREENCAST_INTERFACE: &str = "org.freedesktop.portal.ScreenCast";
const PORTAL_REQUEST_INTERFACE: &str = "org.freedesktop.portal.Request";
const PORTAL_SESSION_INTERFACE: &str = "org.freedesktop.portal.Session";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const PORTAL_CALL_TIMEOUT: Duration = Duration::from_secs(5);
const RELEASE_TIMEOUT: Duration = Duration::from_secs(1);
const INPUT_TIMEOUT: Duration = Duration::from_secs(3);

const DEVICE_KEYBOARD: u32 = 1;
const DEVICE_POINTER: u32 = 2;
const SOURCE_MONITOR: u32 = 1;
const CURSOR_MODE_HIDDEN: u32 = 1;

const KEY_RELEASED: u32 = 0;
const KEY_PRESSED: u32 = 1;

const POINTER_BUTTON_RELEASED: u32 = 0;
const POINTER_BUTTON_PRESSED: u32 = 1;

const AXIS_VERTICAL: u32 = 0;
const AXIS_HORIZONTAL: u32 = 1;

const BTN_LEFT: i32 = 0x110;
const BTN_RIGHT: i32 = 0x111;
const BTN_MIDDLE: i32 = 0x112;
const BTN_SIDE: i32 = 0x113;
const BTN_EXTRA: i32 = 0x114;
const BTN_FORWARD: i32 = 0x115;
const BTN_BACK: i32 = 0x116;

#[derive(Clone)]
pub struct PortalPointerSession {
    connection: Connection,
    session_handle: OwnedObjectPath,
    streams: Vec<PortalStream>,
    desktop_layout: Option<Vec<LogicalMonitor>>,
    input_lock: Arc<AsyncMutex<()>>,
    valid: Arc<AtomicBool>,
}

#[derive(Clone)]
pub struct PortalKeyboardSession {
    connection: Connection,
    session_handle: OwnedObjectPath,
    input_lock: Arc<AsyncMutex<()>>,
    valid: Arc<AtomicBool>,
}

#[derive(Debug, Clone)]
struct PortalStream {
    node_id: u32,
    position: Option<(i32, i32)>,
    size: Option<(i32, i32)>,
}

#[derive(Debug, Clone)]
struct LogicalMonitor {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    scale: f64,
}

#[derive(Debug, Clone, Copy)]
enum PressedKey {
    Keysym(i32),
    Keycode(i32),
}

struct PointerReleaseGuard {
    connection: Connection,
    session_handle: OwnedObjectPath,
    button: Option<i32>,
    input_guard: Option<OwnedMutexGuard<()>>,
    valid: Arc<AtomicBool>,
}

struct KeyboardReleaseGuard {
    connection: Connection,
    session_handle: OwnedObjectPath,
    pressed: Vec<PressedKey>,
    input_guard: Option<OwnedMutexGuard<()>>,
    valid: Arc<AtomicBool>,
}

struct PortalSessionCleanup {
    connection: Connection,
    session_handle: OwnedObjectPath,
    armed: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum PointerButton {
    Left,
    Right,
    Middle,
    Side,
    Extra,
    Forward,
    Back,
}

#[derive(Debug, Clone, Copy)]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

pub async fn start_portal_pointer_session() -> Result<PortalPointerSession> {
    hydrate_session_bus_env();

    let connection = Connection::session()
        .await
        .context("failed to connect to session bus for remote desktop portal")?;
    let session_handle = create_remote_desktop_session(&connection).await?;
    let mut cleanup = PortalSessionCleanup::new(connection.clone(), session_handle.clone());
    select_pointer_devices(&connection, &session_handle).await?;
    select_monitor_sources(&connection, &session_handle).await?;
    let (devices, streams) = start_remote_desktop_session(&connection, &session_handle).await?;

    if devices & DEVICE_POINTER == 0 {
        bail!("remote desktop portal session started without pointer access");
    }
    if streams.is_empty() {
        bail!("remote desktop portal session started without any monitor streams");
    }

    let desktop_layout = logical_desktop_layout().await;
    cleanup.disarm();

    Ok(PortalPointerSession {
        connection,
        session_handle,
        streams,
        desktop_layout,
        input_lock: portal_input_lock(),
        valid: Arc::new(AtomicBool::new(true)),
    })
}

pub async fn start_portal_keyboard_session() -> Result<PortalKeyboardSession> {
    hydrate_session_bus_env();

    let connection = Connection::session()
        .await
        .context("failed to connect to session bus for remote desktop portal")?;
    let session_handle = create_remote_desktop_session(&connection).await?;
    let mut cleanup = PortalSessionCleanup::new(connection.clone(), session_handle.clone());
    select_keyboard_devices(&connection, &session_handle).await?;
    let (devices, _) = start_remote_desktop_session(&connection, &session_handle).await?;

    if devices & DEVICE_KEYBOARD == 0 {
        bail!("remote desktop portal session started without keyboard access");
    }
    cleanup.disarm();

    Ok(PortalKeyboardSession {
        connection,
        session_handle,
        input_lock: portal_input_lock(),
        valid: Arc::new(AtomicBool::new(true)),
    })
}

fn portal_input_lock() -> Arc<AsyncMutex<()>> {
    static INPUT_LOCK: OnceLock<Arc<AsyncMutex<()>>> = OnceLock::new();
    Arc::clone(INPUT_LOCK.get_or_init(|| Arc::new(AsyncMutex::new(()))))
}

async fn logical_desktop_layout() -> Option<Vec<LogicalMonitor>> {
    if env_token_contains("XDG_CURRENT_DESKTOP", "gnome") {
        if let Some(layout) = crate::windowing::backends::gnome::extension_monitor_layout()
            .await
            .ok()
            .filter(|monitors| !monitors.is_empty())
            .map(|monitors| {
                monitors
                    .into_iter()
                    .map(|monitor| LogicalMonitor {
                        x: monitor.x,
                        y: monitor.y,
                        width: monitor.width,
                        height: monitor.height,
                        scale: monitor.scale,
                    })
                    .collect::<Vec<_>>()
            })
        {
            if layout.iter().all(|monitor| {
                monitor.width > 0
                    && monitor.height > 0
                    && monitor.scale.is_finite()
                    && monitor.scale >= 0.0
            }) {
                return Some(layout);
            }
        }
    }

    if env_token_contains("XDG_CURRENT_DESKTOP", "hyprland") {
        let mut command = Command::new("hyprctl");
        command.args(["monitors", "-j"]);
        if let Ok(output) = command_runner::output(command, "query Hyprland monitor layout").await {
            if output.status.success() {
                if let Some(layout) = parse_hyprland_monitor_layout(&output.stdout) {
                    return Some(layout);
                }
            }
        }
    }

    if env_token_contains("XDG_CURRENT_DESKTOP", "kde")
        || env_token_contains("XDG_CURRENT_DESKTOP", "plasma")
    {
        let mut command = Command::new("kscreen-doctor");
        command.arg("-j");
        if let Ok(output) = command_runner::output(command, "query KDE monitor layout").await {
            if output.status.success() {
                if let Some(layout) = parse_kscreen_monitor_layout(&output.stdout) {
                    return Some(layout);
                }
            }
        }
    }

    if env_token_contains("XDG_CURRENT_DESKTOP", "sway") || std::env::var_os("SWAYSOCK").is_some() {
        let mut command = Command::new("swaymsg");
        command.args(["-t", "get_outputs", "-r"]);
        if let Ok(output) = command_runner::output(command, "query Sway output layout").await {
            if output.status.success() {
                if let Some(layout) = parse_sway_monitor_layout(&output.stdout) {
                    return Some(layout);
                }
            }
        }
    }

    if env_token_contains("XDG_CURRENT_DESKTOP", "cosmic") {
        if let Ok(monitors) = crate::cosmic_helper::monitor_layout().await {
            let layout = monitors
                .into_iter()
                .map(|monitor| LogicalMonitor {
                    x: monitor.x,
                    y: monitor.y,
                    width: monitor.width,
                    height: monitor.height,
                    scale: monitor.scale,
                })
                .collect::<Vec<_>>();
            if !layout.is_empty()
                && layout.iter().all(|monitor| {
                    monitor.width > 0
                        && monitor.height > 0
                        && monitor.scale.is_finite()
                        && monitor.scale > 0.0
                })
            {
                return Some(layout);
            }
        }
    }

    let mut command = Command::new("xrandr");
    command.arg("--listactivemonitors");
    let output = command_runner::output(command, "query XRandR monitor layout")
        .await
        .ok()?;
    output
        .status
        .success()
        .then(|| parse_xrandr_monitor_layout(&String::from_utf8_lossy(&output.stdout)))
        .flatten()
}

#[derive(serde::Deserialize)]
struct HyprlandMonitorLayout {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    scale: f64,
    #[serde(default)]
    transform: i32,
}

fn parse_hyprland_monitor_layout(json: &[u8]) -> Option<Vec<LogicalMonitor>> {
    let monitors: Vec<HyprlandMonitorLayout> = serde_json::from_slice(json).ok()?;
    let layout = monitors
        .into_iter()
        .map(|monitor| {
            if !monitor.scale.is_finite()
                || monitor.scale <= 0.0
                || monitor.width <= 0
                || monitor.height <= 0
            {
                return None;
            }
            let (width, height) = if monitor.transform.rem_euclid(2) == 1 {
                (monitor.height, monitor.width)
            } else {
                (monitor.width, monitor.height)
            };
            let width = (f64::from(width) / monitor.scale).round() as i32;
            let height = (f64::from(height) / monitor.scale).round() as i32;
            (width > 0 && height > 0).then_some(LogicalMonitor {
                x: monitor.x,
                y: monitor.y,
                width,
                height,
                scale: monitor.scale,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    (!layout.is_empty()).then_some(layout)
}

#[derive(serde::Deserialize)]
struct KscreenConfig {
    outputs: Vec<KscreenOutput>,
}

#[derive(serde::Deserialize)]
struct KscreenOutput {
    pos: KscreenPoint,
    size: KscreenSize,
    scale: f64,
    connected: bool,
    enabled: bool,
}

#[derive(serde::Deserialize)]
struct KscreenPoint {
    x: i32,
    y: i32,
}

#[derive(serde::Deserialize)]
struct KscreenSize {
    width: i32,
    height: i32,
}

fn parse_kscreen_monitor_layout(json: &[u8]) -> Option<Vec<LogicalMonitor>> {
    let config: KscreenConfig = serde_json::from_slice(json).ok()?;
    let layout = config
        .outputs
        .into_iter()
        .filter(|output| output.connected && output.enabled)
        .map(|output| {
            if !output.scale.is_finite()
                || output.scale <= 0.0
                || output.size.width <= 0
                || output.size.height <= 0
            {
                return None;
            }
            let width = (f64::from(output.size.width) / output.scale).round() as i32;
            let height = (f64::from(output.size.height) / output.scale).round() as i32;
            (width > 0 && height > 0).then_some(LogicalMonitor {
                x: output.pos.x,
                y: output.pos.y,
                width,
                height,
                scale: output.scale,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    (!layout.is_empty()).then_some(layout)
}

#[derive(serde::Deserialize)]
struct SwayOutput {
    active: bool,
    rect: SwayRect,
    scale: f64,
}

#[derive(serde::Deserialize)]
struct SwayRect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

fn parse_sway_monitor_layout(json: &[u8]) -> Option<Vec<LogicalMonitor>> {
    let outputs: Vec<SwayOutput> = serde_json::from_slice(json).ok()?;
    let layout = outputs
        .into_iter()
        .filter(|output| output.active)
        .map(|output| {
            (output.scale.is_finite()
                && output.scale > 0.0
                && output.rect.width > 0
                && output.rect.height > 0)
                .then_some(LogicalMonitor {
                    x: output.rect.x,
                    y: output.rect.y,
                    width: output.rect.width,
                    height: output.rect.height,
                    scale: output.scale,
                })
        })
        .collect::<Option<Vec<_>>>()?;
    (!layout.is_empty()).then_some(layout)
}

fn parse_xrandr_monitor_layout(output: &str) -> Option<Vec<LogicalMonitor>> {
    let mut lines = output.lines();
    let monitor_count = lines
        .next()?
        .strip_prefix("Monitors:")?
        .trim()
        .parse::<usize>()
        .ok()?;
    let layout = lines
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            line.split_whitespace()
                .find_map(parse_xrandr_monitor_geometry)
        })
        .collect::<Option<Vec<_>>>()?;
    (monitor_count > 0 && layout.len() == monitor_count).then_some(layout)
}

fn parse_xrandr_monitor_geometry(value: &str) -> Option<LogicalMonitor> {
    let (width, rest) = value.split_once('/')?;
    let (_, rest) = rest.split_once('x')?;
    let (height, rest) = rest.split_once('/')?;
    let offset_start = rest.find(['+', '-'])?;
    let offsets = &rest[offset_start..];
    let second_sign = offsets[1..].find(['+', '-'])? + 1;
    let (x, y) = offsets.split_at(second_sign);
    let width = width.parse().ok()?;
    let height = height.parse().ok()?;
    let x = x.parse().ok()?;
    let y = y.parse().ok()?;
    (width > 0 && height > 0).then_some(LogicalMonitor {
        x,
        y,
        width,
        height,
        scale: 0.0,
    })
}

pub fn keysyms_for_text(text: &str) -> Result<Vec<i32>> {
    text.chars()
        .map(|ch| {
            let keysym = Keysym::from_char(ch);
            if keysym == Keysym::NoSymbol {
                bail!(
                    "character U+{:04X} cannot be represented as an X11 keysym",
                    ch as u32
                );
            }
            i32::try_from(keysym.raw()).context("X11 keysym did not fit in D-Bus int32")
        })
        .collect()
}

pub async fn click(
    session: &PortalPointerSession,
    x: i32,
    y: i32,
    button: PointerButton,
    click_count: u32,
) -> Result<()> {
    let input_guard = Arc::clone(&session.input_lock).lock_owned().await;
    session.ensure_current_layout().await?;
    let proxy = remote_desktop_proxy(&session.connection).await?;
    let mut release_guard = PointerReleaseGuard::new(session, input_guard);
    let (stream_id, x, y) = session.map_absolute_point(x, y)?;
    notify_pointer_motion_absolute(&proxy, &session.session_handle, stream_id, x, y).await?;
    for _ in 0..click_count.max(1) {
        release_guard.arm(button.evdev_code());
        notify_pointer_button(
            &proxy,
            &session.session_handle,
            button.evdev_code(),
            POINTER_BUTTON_PRESSED,
        )
        .await?;
        tokio::time::sleep(Duration::from_millis(35)).await;
        notify_pointer_button(
            &proxy,
            &session.session_handle,
            button.evdev_code(),
            POINTER_BUTTON_RELEASED,
        )
        .await?;
        release_guard.disarm();
    }
    Ok(())
}

pub async fn scroll(
    session: &PortalPointerSession,
    target_point: Option<(i32, i32)>,
    direction: ScrollDirection,
    steps: i32,
) -> Result<()> {
    let _input_guard = Arc::clone(&session.input_lock).lock_owned().await;
    if target_point.is_some() {
        session.ensure_current_layout().await?;
    } else {
        session.ensure_valid()?;
    }
    let proxy = remote_desktop_proxy(&session.connection).await?;
    if let Some((x, y)) = target_point {
        let (stream_id, x, y) = session.map_absolute_point(x, y)?;
        notify_pointer_motion_absolute(&proxy, &session.session_handle, stream_id, x, y).await?;
    }

    let (axis, steps) = portal_scroll_axis_steps(direction, steps, portal_scroll_polarity());
    notify_pointer_axis_discrete(&proxy, &session.session_handle, axis, steps).await
}

/// Native portal discrete-axis polarity for `NotifyPointerAxisDiscrete`.
///
/// Positive vertical steps mean "scroll up" (same convention as ydotool
/// `mousemove --wheel` and Linux `REL_WHEEL`). xdg-desktop-portal-kde's
/// discrete path forwards the signed step without the vertical negation its
/// continuous path applies, so on Plasma the portal must invert vertical
/// steps to keep `direction: "up"|"down"` matching viewport motion.
/// Horizontal is left unchanged (KDE only special-cases continuous vertical).
///
/// Override with `COMPUTER_USE_LINUX_PORTAL_SCROLL_INVERT=1|0|true|false`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PortalScrollPolarity {
    /// Default / GNOME / generic: match ydotool / REL_WHEEL signs.
    Standard,
    /// Invert vertical discrete steps (KDE Plasma portal discrete path).
    InvertVertical,
}

fn portal_scroll_polarity() -> PortalScrollPolarity {
    if let Ok(value) = std::env::var("COMPUTER_USE_LINUX_PORTAL_SCROLL_INVERT") {
        let value = value.trim();
        if value.eq_ignore_ascii_case("1")
            || value.eq_ignore_ascii_case("true")
            || value.eq_ignore_ascii_case("yes")
            || value.eq_ignore_ascii_case("on")
        {
            return PortalScrollPolarity::InvertVertical;
        }
        if value.eq_ignore_ascii_case("0")
            || value.eq_ignore_ascii_case("false")
            || value.eq_ignore_ascii_case("no")
            || value.eq_ignore_ascii_case("off")
        {
            return PortalScrollPolarity::Standard;
        }
    }

    if desktop_env_is_kde_plasma() {
        PortalScrollPolarity::InvertVertical
    } else {
        PortalScrollPolarity::Standard
    }
}

fn desktop_env_is_kde_plasma() -> bool {
    env_token_contains("XDG_CURRENT_DESKTOP", "kde")
        || env_token_contains("XDG_CURRENT_DESKTOP", "plasma")
        || env_token_contains("DESKTOP_SESSION", "plasma")
        || env_token_contains("DESKTOP_SESSION", "kde")
}

fn env_token_contains(key: &str, needle: &str) -> bool {
    std::env::var(key)
        .map(|value| {
            value
                .split([':', ';', ','])
                .any(|part| part.trim().eq_ignore_ascii_case(needle))
        })
        .unwrap_or(false)
}

/// Map a semantic scroll direction to `(axis, signed_steps)` for the portal.
pub(crate) fn portal_scroll_axis_steps(
    direction: ScrollDirection,
    steps: i32,
    polarity: PortalScrollPolarity,
) -> (u32, i32) {
    let magnitude = steps.max(1);
    let (axis, standard_signed) = match direction {
        ScrollDirection::Up => (AXIS_VERTICAL, magnitude),
        ScrollDirection::Down => (AXIS_VERTICAL, -magnitude),
        ScrollDirection::Left => (AXIS_HORIZONTAL, magnitude),
        ScrollDirection::Right => (AXIS_HORIZONTAL, -magnitude),
    };

    let signed = match (polarity, axis) {
        (PortalScrollPolarity::InvertVertical, AXIS_VERTICAL) => -standard_signed,
        _ => standard_signed,
    };
    (axis, signed)
}

pub async fn drag(
    session: &PortalPointerSession,
    start_x: i32,
    start_y: i32,
    end_x: i32,
    end_y: i32,
) -> Result<()> {
    let input_guard = Arc::clone(&session.input_lock).lock_owned().await;
    session.ensure_current_layout().await?;
    let proxy = remote_desktop_proxy(&session.connection).await?;
    let mut release_guard = PointerReleaseGuard::new(session, input_guard);
    let (start_stream, start_x, start_y) = session.map_absolute_point(start_x, start_y)?;
    let (end_stream, end_x, end_y) = session.map_absolute_point(end_x, end_y)?;
    notify_pointer_motion_absolute(
        &proxy,
        &session.session_handle,
        start_stream,
        start_x,
        start_y,
    )
    .await?;
    release_guard.arm(BTN_LEFT);
    notify_pointer_button(
        &proxy,
        &session.session_handle,
        BTN_LEFT,
        POINTER_BUTTON_PRESSED,
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(35)).await;
    notify_pointer_motion_absolute(&proxy, &session.session_handle, end_stream, end_x, end_y)
        .await?;
    tokio::time::sleep(Duration::from_millis(35)).await;
    notify_pointer_button(
        &proxy,
        &session.session_handle,
        BTN_LEFT,
        POINTER_BUTTON_RELEASED,
    )
    .await?;
    release_guard.disarm();
    Ok(())
}

pub async fn type_text_with_keysyms(
    session: &PortalKeyboardSession,
    keysyms: &[i32],
) -> Result<()> {
    let input_guard = Arc::clone(&session.input_lock).lock_owned().await;
    session.ensure_valid()?;
    let proxy = remote_desktop_proxy(&session.connection).await?;
    let mut release_guard = KeyboardReleaseGuard::new(session, input_guard);
    for keysym in keysyms {
        release_guard.push(PressedKey::Keysym(*keysym));
        notify_keyboard_keysym(&proxy, &session.session_handle, *keysym, KEY_PRESSED).await?;
        tokio::time::sleep(Duration::from_millis(5)).await;
        notify_keyboard_keysym(&proxy, &session.session_handle, *keysym, KEY_RELEASED).await?;
        release_guard.pop();
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    Ok(())
}

pub async fn press_keycode_chord(
    session: &PortalKeyboardSession,
    modifiers: &[i32],
    keycode: i32,
) -> Result<()> {
    let input_guard = Arc::clone(&session.input_lock).lock_owned().await;
    session.ensure_valid()?;
    let proxy = remote_desktop_proxy(&session.connection).await?;
    let mut release_guard = KeyboardReleaseGuard::new(session, input_guard);
    for modifier in modifiers {
        release_guard.push(PressedKey::Keycode(*modifier));
        notify_keyboard_keycode(&proxy, &session.session_handle, *modifier, KEY_PRESSED).await?;
    }
    release_guard.push(PressedKey::Keycode(keycode));
    notify_keyboard_keycode(&proxy, &session.session_handle, keycode, KEY_PRESSED).await?;
    tokio::time::sleep(Duration::from_millis(35)).await;
    notify_keyboard_keycode(&proxy, &session.session_handle, keycode, KEY_RELEASED).await?;
    release_guard.pop();
    for modifier in modifiers.iter().rev() {
        notify_keyboard_keycode(&proxy, &session.session_handle, *modifier, KEY_RELEASED).await?;
        release_guard.pop();
    }
    Ok(())
}

impl PointerReleaseGuard {
    fn new(session: &PortalPointerSession, input_guard: OwnedMutexGuard<()>) -> Self {
        Self {
            connection: session.connection.clone(),
            session_handle: session.session_handle.clone(),
            button: None,
            input_guard: Some(input_guard),
            valid: Arc::clone(&session.valid),
        }
    }

    fn arm(&mut self, button: i32) {
        self.button = Some(button);
    }

    fn disarm(&mut self) {
        self.button = None;
    }
}

impl Drop for PointerReleaseGuard {
    fn drop(&mut self) {
        let input_guard = self.input_guard.take();
        let Some(button) = self.button else {
            return;
        };
        self.valid.store(false, Ordering::Release);
        let connection = self.connection.clone();
        let session_handle = self.session_handle.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = tokio::time::timeout(RELEASE_TIMEOUT, async {
                    if let Ok(proxy) = remote_desktop_proxy(&connection).await {
                        let _ = notify_pointer_button(
                            &proxy,
                            &session_handle,
                            button,
                            POINTER_BUTTON_RELEASED,
                        )
                        .await;
                    }
                })
                .await;
                let _ = tokio::time::timeout(
                    RELEASE_TIMEOUT,
                    close_portal_session(&connection, &session_handle),
                )
                .await;
                drop(input_guard);
            });
        }
    }
}

impl KeyboardReleaseGuard {
    fn new(session: &PortalKeyboardSession, input_guard: OwnedMutexGuard<()>) -> Self {
        Self {
            connection: session.connection.clone(),
            session_handle: session.session_handle.clone(),
            pressed: Vec::new(),
            input_guard: Some(input_guard),
            valid: Arc::clone(&session.valid),
        }
    }

    fn push(&mut self, key: PressedKey) {
        self.pressed.push(key);
    }

    fn pop(&mut self) {
        self.pressed.pop();
    }
}

impl PortalSessionCleanup {
    fn new(connection: Connection, session_handle: OwnedObjectPath) -> Self {
        Self {
            connection,
            session_handle,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PortalSessionCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let connection = self.connection.clone();
        let session_handle = self.session_handle.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = tokio::time::timeout(
                    RELEASE_TIMEOUT,
                    close_portal_session(&connection, &session_handle),
                )
                .await;
            });
        }
    }
}

impl Drop for KeyboardReleaseGuard {
    fn drop(&mut self) {
        let input_guard = self.input_guard.take();
        if self.pressed.is_empty() {
            return;
        }
        self.valid.store(false, Ordering::Release);
        let connection = self.connection.clone();
        let session_handle = self.session_handle.clone();
        let pressed = std::mem::take(&mut self.pressed);
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = tokio::time::timeout(RELEASE_TIMEOUT, async {
                    if let Ok(proxy) = remote_desktop_proxy(&connection).await {
                        for key in pressed.into_iter().rev() {
                            match key {
                                PressedKey::Keysym(keysym) => {
                                    let _ = notify_keyboard_keysym(
                                        &proxy,
                                        &session_handle,
                                        keysym,
                                        KEY_RELEASED,
                                    )
                                    .await;
                                }
                                PressedKey::Keycode(keycode) => {
                                    let _ = notify_keyboard_keycode(
                                        &proxy,
                                        &session_handle,
                                        keycode,
                                        KEY_RELEASED,
                                    )
                                    .await;
                                }
                            }
                        }
                    }
                })
                .await;
                let _ = tokio::time::timeout(
                    RELEASE_TIMEOUT,
                    close_portal_session(&connection, &session_handle),
                )
                .await;
                drop(input_guard);
            });
        }
    }
}

impl PortalPointerSession {
    pub(crate) fn is_valid(&self) -> bool {
        self.valid.load(Ordering::Acquire)
    }

    pub(crate) fn invalidate_and_close(&self) {
        invalidate_and_close(&self.valid, &self.connection, &self.session_handle);
    }

    pub(crate) fn same_session(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.valid, &other.valid)
    }

    fn ensure_valid(&self) -> Result<()> {
        if !self.is_valid() {
            bail!("remote desktop portal pointer session is no longer valid");
        }
        Ok(())
    }

    async fn ensure_current_layout(&self) -> Result<()> {
        self.ensure_valid()?;
        let expected = self
            .desktop_layout
            .as_deref()
            .context("remote desktop portal session has no authoritative monitor layout")?;
        let current = logical_desktop_layout()
            .await
            .context("could not revalidate the current monitor layout")?;
        if !same_monitor_layout(expected, &current) {
            self.invalidate_and_close();
            bail!("desktop monitor layout changed after the remote desktop portal session started");
        }
        Ok(())
    }

    pub(crate) fn logical_point_from_capture(
        &self,
        x: i32,
        y: i32,
        capture_size: Option<(u32, u32)>,
    ) -> Option<(i32, i32)> {
        let (width, height) = capture_size?;
        map_capture_point_to_stream_layout(
            &self.streams,
            self.desktop_layout.as_deref()?,
            x,
            y,
            width,
            height,
        )
    }

    fn map_absolute_point(&self, x: i32, y: i32) -> Result<(u32, f64, f64)> {
        if let Some(stream) = self
            .streams
            .iter()
            .find(|stream| stream.contains_global_point(x, y))
        {
            return Ok(stream.relative_point(x, y));
        }

        bail!("point ({x}, {y}) is outside the monitors shared with the remote desktop portal")
    }
}

impl PortalKeyboardSession {
    pub(crate) fn is_valid(&self) -> bool {
        self.valid.load(Ordering::Acquire)
    }

    pub(crate) fn invalidate_and_close(&self) {
        invalidate_and_close(&self.valid, &self.connection, &self.session_handle);
    }

    pub(crate) fn same_session(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.valid, &other.valid)
    }

    fn ensure_valid(&self) -> Result<()> {
        if !self.is_valid() {
            bail!("remote desktop portal keyboard session is no longer valid");
        }
        Ok(())
    }
}

fn same_monitor_layout(expected: &[LogicalMonitor], current: &[LogicalMonitor]) -> bool {
    if expected.len() != current.len() {
        return false;
    }
    let mut expected = expected.to_vec();
    let mut current = current.to_vec();
    let sort_key = |monitor: &LogicalMonitor| (monitor.x, monitor.y, monitor.width, monitor.height);
    expected.sort_by_key(sort_key);
    current.sort_by_key(sort_key);
    expected.iter().zip(current.iter()).all(|(left, right)| {
        sort_key(left) == sort_key(right) && (left.scale - right.scale).abs() <= 0.01
    })
}

fn map_capture_point_to_stream_layout(
    streams: &[PortalStream],
    desktop_layout: &[LogicalMonitor],
    x: i32,
    y: i32,
    capture_width: u32,
    capture_height: u32,
) -> Option<(i32, i32)> {
    if capture_width == 0
        || capture_height == 0
        || x < 0
        || y < 0
        || x >= i32::try_from(capture_width).unwrap_or(i32::MAX)
        || y >= i32::try_from(capture_height).unwrap_or(i32::MAX)
    {
        return None;
    }

    let mut stream_rects = streams
        .iter()
        .map(|stream| {
            let (x, y) = stream.position?;
            let (width, height) = stream.size?;
            (width > 0 && height > 0).then_some((x, y, width, height))
        })
        .collect::<Option<Vec<_>>>()?;
    if desktop_layout.is_empty()
        || desktop_layout
            .iter()
            .any(|monitor| monitor.width <= 0 || monitor.height <= 0)
    {
        return None;
    }
    let mut monitor_rects = desktop_layout
        .iter()
        .map(|monitor| (monitor.x, monitor.y, monitor.width, monitor.height))
        .collect::<Vec<_>>();
    stream_rects.sort_unstable();
    monitor_rects.sort_unstable();
    if stream_rects != monitor_rects {
        return None;
    }

    let unknown_multi_monitor_scale = desktop_layout.len() > 1
        && desktop_layout
            .iter()
            .any(|monitor| !monitor.scale.is_finite() || monitor.scale <= 0.0);
    if desktop_layout.len() > 1 && !unknown_multi_monitor_scale {
        let first_scale = desktop_layout.first()?.scale;
        if desktop_layout
            .iter()
            .any(|monitor| (monitor.scale - first_scale).abs() > 0.01)
        {
            return None;
        }
    }

    let mut bounds = monitor_rects.iter().map(|(x, y, width, height)| {
        (
            i64::from(*x),
            i64::from(*y),
            i64::from(*x) + i64::from(*width),
            i64::from(*y) + i64::from(*height),
        )
    });
    let (mut min_x, mut min_y, mut max_x, mut max_y) = bounds.next()?;
    for (left, top, right, bottom) in bounds {
        min_x = min_x.min(left);
        min_y = min_y.min(top);
        max_x = max_x.max(right);
        max_y = max_y.max(bottom);
    }
    let logical_width = max_x - min_x;
    let logical_height = max_y - min_y;
    if logical_width <= 0 || logical_height <= 0 {
        return None;
    }
    if unknown_multi_monitor_scale
        && (i64::from(capture_width) != logical_width
            || i64::from(capture_height) != logical_height)
    {
        return None;
    }
    let scale_x = f64::from(capture_width) / logical_width as f64;
    let scale_y = f64::from(capture_height) / logical_height as f64;
    if !scale_x.is_finite() || !scale_y.is_finite() || (scale_x - scale_y).abs() > 0.01 {
        return None;
    }

    let point = (
        map_capture_axis(x, capture_width, min_x, max_x - min_x),
        map_capture_axis(y, capture_height, min_y, max_y - min_y),
    );
    streams
        .iter()
        .any(|stream| stream.contains_global_point(point.0, point.1))
        .then_some(point)
}

fn map_capture_axis(value: i32, capture_size: u32, target_origin: i64, target_size: i64) -> i32 {
    let scaled = i64::from(value).saturating_mul(target_size) / i64::from(capture_size);
    target_origin
        .saturating_add(scaled)
        .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

impl PortalStream {
    fn contains_global_point(&self, x: i32, y: i32) -> bool {
        let Some((stream_x, stream_y)) = self.position else {
            return false;
        };
        let Some((width, height)) = self.size else {
            return false;
        };
        x >= stream_x && y >= stream_y && x < stream_x + width && y < stream_y + height
    }

    fn relative_point(&self, x: i32, y: i32) -> (u32, f64, f64) {
        let (stream_x, stream_y) = self.position.unwrap_or((0, 0));
        let (width, height) = self.size.unwrap_or((i32::MAX, i32::MAX));
        let rel_x = (x - stream_x).clamp(0, width.saturating_sub(1)) as f64;
        let rel_y = (y - stream_y).clamp(0, height.saturating_sub(1)) as f64;
        (self.node_id, rel_x, rel_y)
    }
}

impl PointerButton {
    pub fn from_name(name: Option<&str>) -> Self {
        match name.unwrap_or("left").to_ascii_lowercase().as_str() {
            "right" => Self::Right,
            "middle" => Self::Middle,
            "side" => Self::Side,
            "extra" => Self::Extra,
            "forward" => Self::Forward,
            "back" => Self::Back,
            _ => Self::Left,
        }
    }

    fn evdev_code(self) -> i32 {
        match self {
            Self::Left => BTN_LEFT,
            Self::Right => BTN_RIGHT,
            Self::Middle => BTN_MIDDLE,
            Self::Side => BTN_SIDE,
            Self::Extra => BTN_EXTRA,
            Self::Forward => BTN_FORWARD,
            Self::Back => BTN_BACK,
        }
    }
}

async fn create_remote_desktop_session(connection: &Connection) -> Result<OwnedObjectPath> {
    let remote_proxy = remote_desktop_proxy(connection).await?;
    let (request_path, mut response_stream) =
        portal_request_stream(connection, "rd_create").await?;
    let session_token = request_token("rd_session");
    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert(
        "handle_token",
        Value::from(last_path_component(&request_path)),
    );
    options.insert("session_handle_token", Value::from(session_token.as_str()));

    let handle: OwnedObjectPath = tokio::time::timeout(
        PORTAL_CALL_TIMEOUT,
        remote_proxy.call("CreateSession", &(options)),
    )
    .await
    .context("RemoteDesktop CreateSession call timed out")?
    .context("RemoteDesktop CreateSession call failed")?;
    let (response_code, results) =
        await_portal_response(connection, handle, &request_path, &mut response_stream).await?;
    if response_code != 0 {
        bail!("RemoteDesktop CreateSession denied or cancelled with response code {response_code}");
    }

    let session_handle: String = results
        .get("session_handle")
        .context("RemoteDesktop CreateSession response did not include session_handle")?
        .try_clone()
        .context("failed to clone session_handle")?
        .try_into()
        .context("RemoteDesktop session_handle was not a string")?;
    OwnedObjectPath::try_from(session_handle)
        .context("RemoteDesktop session_handle was not a valid object path")
}

async fn select_pointer_devices(connection: &Connection, session: &OwnedObjectPath) -> Result<()> {
    select_devices(connection, session, DEVICE_POINTER, "rd_devices").await
}

async fn select_keyboard_devices(connection: &Connection, session: &OwnedObjectPath) -> Result<()> {
    select_devices(connection, session, DEVICE_KEYBOARD, "rd_keyboard_devices").await
}

async fn select_devices(
    connection: &Connection,
    session: &OwnedObjectPath,
    device_types: u32,
    request_prefix: &str,
) -> Result<()> {
    let remote_proxy = remote_desktop_proxy(connection).await?;
    let (request_path, mut response_stream) =
        portal_request_stream(connection, request_prefix).await?;
    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert(
        "handle_token",
        Value::from(last_path_component(&request_path)),
    );
    options.insert("types", Value::from(device_types));

    let handle: OwnedObjectPath = tokio::time::timeout(
        PORTAL_CALL_TIMEOUT,
        remote_proxy.call("SelectDevices", &(session, options)),
    )
    .await
    .context("RemoteDesktop SelectDevices call timed out")?
    .context("RemoteDesktop SelectDevices call failed")?;
    let (response_code, _) =
        await_portal_response(connection, handle, &request_path, &mut response_stream).await?;
    if response_code != 0 {
        bail!("RemoteDesktop SelectDevices denied or cancelled with response code {response_code}");
    }
    Ok(())
}

async fn select_monitor_sources(connection: &Connection, session: &OwnedObjectPath) -> Result<()> {
    let screencast_proxy = screencast_proxy(connection).await?;
    let (request_path, mut response_stream) =
        portal_request_stream(connection, "rd_sources").await?;
    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert(
        "handle_token",
        Value::from(last_path_component(&request_path)),
    );
    options.insert("types", Value::from(SOURCE_MONITOR));
    options.insert("multiple", Value::from(true));
    options.insert("cursor_mode", Value::from(CURSOR_MODE_HIDDEN));

    let handle: OwnedObjectPath = tokio::time::timeout(
        PORTAL_CALL_TIMEOUT,
        screencast_proxy.call("SelectSources", &(session, options)),
    )
    .await
    .context("ScreenCast SelectSources call timed out for remote desktop session")?
    .context("ScreenCast SelectSources call failed for remote desktop session")?;
    let (response_code, _) =
        await_portal_response(connection, handle, &request_path, &mut response_stream).await?;
    if response_code != 0 {
        bail!("ScreenCast SelectSources denied or cancelled with response code {response_code}");
    }
    Ok(())
}

async fn start_remote_desktop_session(
    connection: &Connection,
    session: &OwnedObjectPath,
) -> Result<(u32, Vec<PortalStream>)> {
    let remote_proxy = remote_desktop_proxy(connection).await?;
    let (request_path, mut response_stream) = portal_request_stream(connection, "rd_start").await?;
    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert(
        "handle_token",
        Value::from(last_path_component(&request_path)),
    );

    let handle: OwnedObjectPath = tokio::time::timeout(
        PORTAL_CALL_TIMEOUT,
        remote_proxy.call("Start", &(session, "", options)),
    )
    .await
    .context("RemoteDesktop Start call timed out")?
    .context("RemoteDesktop Start call failed")?;
    let (response_code, results) =
        await_portal_response(connection, handle, &request_path, &mut response_stream).await?;
    if response_code != 0 {
        bail!("RemoteDesktop Start denied or cancelled with response code {response_code}");
    }

    let devices = results
        .get("devices")
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or_default();
    let streams = results
        .get("streams")
        .map(parse_streams)
        .transpose()?
        .unwrap_or_default();
    Ok((devices, streams))
}

async fn notify_pointer_motion_absolute(
    proxy: &Proxy<'_>,
    session: &OwnedObjectPath,
    stream_id: u32,
    x: f64,
    y: f64,
) -> Result<()> {
    let options: HashMap<&str, Value<'_>> = HashMap::new();
    let _: () = tokio::time::timeout(
        INPUT_TIMEOUT,
        proxy.call(
            "NotifyPointerMotionAbsolute",
            &(session, options, stream_id, x, y),
        ),
    )
    .await
    .context("RemoteDesktop NotifyPointerMotionAbsolute timed out")?
    .context("RemoteDesktop NotifyPointerMotionAbsolute failed")?;
    Ok(())
}

async fn notify_pointer_button(
    proxy: &Proxy<'_>,
    session: &OwnedObjectPath,
    button: i32,
    state: u32,
) -> Result<()> {
    let options: HashMap<&str, Value<'_>> = HashMap::new();
    let _: () = tokio::time::timeout(
        INPUT_TIMEOUT,
        proxy.call("NotifyPointerButton", &(session, options, button, state)),
    )
    .await
    .context("RemoteDesktop NotifyPointerButton timed out")?
    .context("RemoteDesktop NotifyPointerButton failed")?;
    Ok(())
}

async fn notify_pointer_axis_discrete(
    proxy: &Proxy<'_>,
    session: &OwnedObjectPath,
    axis: u32,
    steps: i32,
) -> Result<()> {
    let options: HashMap<&str, Value<'_>> = HashMap::new();
    let _: () = tokio::time::timeout(
        INPUT_TIMEOUT,
        proxy.call(
            "NotifyPointerAxisDiscrete",
            &(session, options, axis, steps),
        ),
    )
    .await
    .context("RemoteDesktop NotifyPointerAxisDiscrete timed out")?
    .context("RemoteDesktop NotifyPointerAxisDiscrete failed")?;
    Ok(())
}

async fn notify_keyboard_keysym(
    proxy: &Proxy<'_>,
    session: &OwnedObjectPath,
    keysym: i32,
    state: u32,
) -> Result<()> {
    let options: HashMap<&str, Value<'_>> = HashMap::new();
    let _: () = tokio::time::timeout(
        INPUT_TIMEOUT,
        proxy.call("NotifyKeyboardKeysym", &(session, options, keysym, state)),
    )
    .await
    .context("RemoteDesktop NotifyKeyboardKeysym timed out")?
    .context("RemoteDesktop NotifyKeyboardKeysym failed")?;
    Ok(())
}

async fn notify_keyboard_keycode(
    proxy: &Proxy<'_>,
    session: &OwnedObjectPath,
    keycode: i32,
    state: u32,
) -> Result<()> {
    let options: HashMap<&str, Value<'_>> = HashMap::new();
    let _: () = tokio::time::timeout(
        INPUT_TIMEOUT,
        proxy.call("NotifyKeyboardKeycode", &(session, options, keycode, state)),
    )
    .await
    .context("RemoteDesktop NotifyKeyboardKeycode timed out")?
    .context("RemoteDesktop NotifyKeyboardKeycode failed")?;
    Ok(())
}

async fn close_portal_session(connection: &Connection, session: &OwnedObjectPath) {
    if let Ok(proxy) = Proxy::new(
        connection,
        PORTAL_DESKTOP_SERVICE,
        session.as_str(),
        PORTAL_SESSION_INTERFACE,
    )
    .await
    {
        let _: Result<(), _> = proxy.call("Close", &()).await;
    }
}

fn invalidate_and_close(valid: &AtomicBool, connection: &Connection, session: &OwnedObjectPath) {
    if !valid.swap(false, Ordering::AcqRel) {
        return;
    }
    let connection = connection.clone();
    let session = session.clone();
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        runtime.spawn(async move {
            let _ =
                tokio::time::timeout(RELEASE_TIMEOUT, close_portal_session(&connection, &session))
                    .await;
        });
    }
}

async fn remote_desktop_proxy(connection: &Connection) -> Result<Proxy<'_>> {
    Proxy::new(
        connection,
        PORTAL_DESKTOP_SERVICE,
        PORTAL_DESKTOP_PATH,
        PORTAL_REMOTE_DESKTOP_INTERFACE,
    )
    .await
    .context("failed to create RemoteDesktop portal proxy")
}

async fn screencast_proxy(connection: &Connection) -> Result<Proxy<'_>> {
    Proxy::new(
        connection,
        PORTAL_DESKTOP_SERVICE,
        PORTAL_DESKTOP_PATH,
        PORTAL_SCREENCAST_INTERFACE,
    )
    .await
    .context("failed to create ScreenCast portal proxy")
}

async fn portal_request_stream<'a>(
    connection: &'a Connection,
    prefix: &str,
) -> Result<(String, SignalStream<'a>)> {
    let unique_name = connection
        .unique_name()
        .context("session bus connection has no unique name")?;
    let token = request_token(prefix);
    let request_path = request_path(unique_name.as_str(), &token);
    let request_proxy = Proxy::new(
        connection,
        PORTAL_DESKTOP_SERVICE,
        request_path.as_str(),
        PORTAL_REQUEST_INTERFACE,
    )
    .await
    .context("failed to create portal request proxy")?;
    let response_stream = request_proxy
        .receive_signal("Response")
        .await
        .context("failed to subscribe to portal request response")?;
    Ok((request_path, response_stream))
}

async fn await_portal_response(
    connection: &Connection,
    handle: OwnedObjectPath,
    expected_request_path: &str,
    response_stream: &mut SignalStream<'_>,
) -> Result<(u32, HashMap<String, OwnedValue>)> {
    if handle.as_str() != expected_request_path {
        *response_stream = Proxy::new(
            connection,
            PORTAL_DESKTOP_SERVICE,
            handle.as_str(),
            PORTAL_REQUEST_INTERFACE,
        )
        .await
        .context("failed to create returned portal request proxy")?
        .receive_signal("Response")
        .await
        .context("failed to subscribe to returned portal response")?;
    }

    let response = tokio::time::timeout(REQUEST_TIMEOUT, response_stream.next())
        .await
        .context("timed out waiting for portal response")?
        .context("portal response stream ended")?;
    response
        .body()
        .deserialize()
        .context("failed to decode portal response")
}

fn parse_streams(value: &OwnedValue) -> Result<Vec<PortalStream>> {
    let streams: Vec<(u32, HashMap<String, OwnedValue>)> = value
        .try_clone()
        .context("failed to clone streams response")?
        .try_into()
        .context("portal streams response had unexpected type")?;
    Ok(streams
        .into_iter()
        .map(|(node_id, properties)| PortalStream {
            node_id,
            position: get_pair_i32(&properties, "position"),
            size: get_pair_i32(&properties, "size"),
        })
        .collect())
}

fn get_pair_i32(properties: &HashMap<String, OwnedValue>, key: &str) -> Option<(i32, i32)> {
    properties.get(key).and_then(|value| {
        value
            .try_clone()
            .ok()
            .and_then(|owned| <(i32, i32)>::try_from(owned).ok())
            .or_else(|| {
                value
                    .try_clone()
                    .ok()
                    .and_then(|owned| <(u32, u32)>::try_from(owned).ok())
                    .map(|(left, right)| (left as i32, right as i32))
            })
    })
}

fn request_path(unique_name: &str, token: &str) -> String {
    format!(
        "/org/freedesktop/portal/desktop/request/{}/{}",
        unique_name.trim_start_matches(':').replace('.', "_"),
        token
    )
}

fn last_path_component(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn request_token(prefix: &str) -> String {
    format!(
        "{prefix}_{}_{:?}",
        std::process::id(),
        std::time::SystemTime::now()
    )
    .chars()
    .map(|ch| match ch {
        'a'..='z' | 'A'..='Z' | '0'..='9' | '_' => ch,
        _ => '_',
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use xkeysym::key;

    #[test]
    fn keysyms_for_url_text_round_trips_to_literal_characters() {
        let text = "https://example.com:8080/page#anchor";
        let keysyms = keysyms_for_text(text).expect("URL should map to keysyms");

        let round_tripped = keysyms
            .iter()
            .map(|keysym| {
                Keysym::new(*keysym as u32)
                    .key_char()
                    .expect("keysym should map back to a character")
            })
            .collect::<String>();

        assert_eq!(round_tripped, text);
    }

    #[test]
    fn keysyms_for_layout_sensitive_ascii_use_literal_symbols() {
        assert_eq!(
            keysyms_for_text(":#/?@").expect("symbols should map to keysyms"),
            vec![
                key::colon as i32,
                key::numbersign as i32,
                key::slash as i32,
                key::question as i32,
                key::at as i32,
            ]
        );
    }

    #[test]
    fn keysyms_for_non_ascii_use_legacy_and_unicode_mapped_values() {
        assert_eq!(
            keysyms_for_text("ä€😉").expect("non-ASCII text should map to keysyms"),
            vec![key::adiaeresis as i32, key::EuroSign as i32, 0x0101_F609]
        );
    }

    #[test]
    fn keysyms_for_text_rejects_unicode_non_symbols_before_input() {
        let error = keysyms_for_text("\u{FDD0}")
            .expect_err("Unicode non-characters should not be emitted through the portal")
            .to_string();

        assert!(error.contains("U+FDD0"));
    }

    #[test]
    fn portal_scroll_standard_polarity_matches_ydotool_rel_wheel_signs() {
        assert_eq!(
            portal_scroll_axis_steps(ScrollDirection::Up, 1, PortalScrollPolarity::Standard),
            (AXIS_VERTICAL, 1)
        );
        assert_eq!(
            portal_scroll_axis_steps(ScrollDirection::Down, 3, PortalScrollPolarity::Standard),
            (AXIS_VERTICAL, -3)
        );
        assert_eq!(
            portal_scroll_axis_steps(ScrollDirection::Left, 2, PortalScrollPolarity::Standard),
            (AXIS_HORIZONTAL, 2)
        );
        assert_eq!(
            portal_scroll_axis_steps(ScrollDirection::Right, 2, PortalScrollPolarity::Standard),
            (AXIS_HORIZONTAL, -2)
        );
    }

    #[test]
    fn portal_scroll_kde_inverts_vertical_only() {
        assert_eq!(
            portal_scroll_axis_steps(ScrollDirection::Up, 1, PortalScrollPolarity::InvertVertical),
            (AXIS_VERTICAL, -1)
        );
        assert_eq!(
            portal_scroll_axis_steps(
                ScrollDirection::Down,
                3,
                PortalScrollPolarity::InvertVertical
            ),
            (AXIS_VERTICAL, 3)
        );
        // Horizontal must stay standard so a KDE flip does not invent L/R bugs.
        assert_eq!(
            portal_scroll_axis_steps(
                ScrollDirection::Left,
                2,
                PortalScrollPolarity::InvertVertical
            ),
            (AXIS_HORIZONTAL, 2)
        );
        assert_eq!(
            portal_scroll_axis_steps(
                ScrollDirection::Right,
                2,
                PortalScrollPolarity::InvertVertical
            ),
            (AXIS_HORIZONTAL, -2)
        );
    }

    #[test]
    fn portal_scroll_clamps_zero_or_negative_steps_to_one() {
        assert_eq!(
            portal_scroll_axis_steps(ScrollDirection::Down, 0, PortalScrollPolarity::Standard),
            (AXIS_VERTICAL, -1)
        );
        assert_eq!(
            portal_scroll_axis_steps(
                ScrollDirection::Down,
                -5,
                PortalScrollPolarity::InvertVertical
            ),
            (AXIS_VERTICAL, 1)
        );
    }

    #[test]
    fn parses_xrandr_monitor_layout_with_negative_origins() {
        let layout = parse_xrandr_monitor_layout(
            "Monitors: 2\n 0: +*eDP-1 1920/344x1080/194+0+0 eDP-1\n 1: +DP-2 2560/600x1440/340-2560+0 DP-2\n",
        )
        .expect("XRandR layout should parse");

        assert_eq!(layout.len(), 2);
        assert_eq!(
            (layout[0].x, layout[0].y, layout[0].width, layout[0].height),
            (0, 0, 1920, 1080)
        );
        assert_eq!(
            (layout[1].x, layout[1].y, layout[1].width, layout[1].height),
            (-2560, 0, 2560, 1440)
        );
    }

    #[test]
    fn parses_hyprland_monitor_layout_in_logical_coordinates() {
        let layout = parse_hyprland_monitor_layout(
            br#"[
                {"x":0,"y":0,"width":3840,"height":2160,"scale":2.0,"transform":0},
                {"x":1920,"y":0,"width":2560,"height":1440,"scale":1.0,"transform":1}
            ]"#,
        )
        .expect("Hyprland layout should parse");

        assert_eq!(
            (layout[0].x, layout[0].y, layout[0].width, layout[0].height),
            (0, 0, 1920, 1080)
        );
        assert_eq!(
            (layout[1].x, layout[1].y, layout[1].width, layout[1].height),
            (1920, 0, 1440, 2560)
        );
    }

    #[test]
    fn parses_kscreen_physical_sizes_in_logical_coordinates() {
        let layout = parse_kscreen_monitor_layout(
            br#"{
                "outputs":[
                    {"pos":{"x":0,"y":0},"size":{"width":3840,"height":2160},"scale":2.0,"connected":true,"enabled":true},
                    {"pos":{"x":1920,"y":0},"size":{"width":1440,"height":2560},"scale":1.0,"connected":true,"enabled":true},
                    {"pos":{"x":0,"y":0},"size":{"width":0,"height":0},"scale":0.0,"connected":false,"enabled":false}
                ]
            }"#,
        )
        .expect("KScreen layout should parse");

        assert_eq!(layout.len(), 2);
        assert_eq!(
            (layout[0].x, layout[0].y, layout[0].width, layout[0].height),
            (0, 0, 1920, 1080)
        );
        assert_eq!(
            (layout[1].x, layout[1].y, layout[1].width, layout[1].height),
            (1920, 0, 1440, 2560)
        );
    }

    #[test]
    fn parses_sway_logical_output_rectangles() {
        let layout = parse_sway_monitor_layout(
            br#"[
                {"active":true,"rect":{"x":-1536,"y":0,"width":1536,"height":864},"scale":1.25},
                {"active":true,"rect":{"x":0,"y":0,"width":1920,"height":1080},"scale":1.0},
                {"active":false,"rect":{"x":0,"y":0,"width":0,"height":0},"scale":0.0}
            ]"#,
        )
        .expect("Sway layout should parse");

        assert_eq!(layout.len(), 2);
        assert_eq!(
            (
                layout[0].x,
                layout[0].y,
                layout[0].width,
                layout[0].height,
                layout[0].scale,
            ),
            (-1536, 0, 1536, 864, 1.25)
        );
    }

    #[test]
    fn monitor_parsers_reject_partial_active_layouts() {
        assert!(parse_hyprland_monitor_layout(
            br#"[
                {"x":0,"y":0,"width":1920,"height":1080,"scale":1.0},
                {"x":1920,"y":0,"width":1920,"height":1080,"scale":0.0}
            ]"#,
        )
        .is_none());
        assert!(parse_kscreen_monitor_layout(
            br#"{"outputs":[
                {"pos":{"x":0,"y":0},"size":{"width":1920,"height":1080},"scale":1.0,"connected":true,"enabled":true},
                {"pos":{"x":1920,"y":0},"size":{"width":0,"height":1080},"scale":1.0,"connected":true,"enabled":true}
            ]}"#,
        )
        .is_none());
        assert!(parse_sway_monitor_layout(
            br#"[
                {"active":true,"rect":{"x":0,"y":0,"width":1920,"height":1080},"scale":1.0},
                {"active":true,"rect":{"x":1920,"y":0,"width":1920,"height":1080},"scale":0.0}
            ]"#,
        )
        .is_none());
        assert!(parse_sway_monitor_layout(
            br#"[{"rect":{"x":0,"y":0,"width":1920,"height":1080},"scale":1.0}]"#,
        )
        .is_none());
        assert!(parse_xrandr_monitor_layout(
            "Monitors: 2\n 0: +*eDP-1 1920/344x1080/194+0+0 eDP-1\n"
        )
        .is_none());
    }

    #[test]
    fn monitor_layout_comparison_detects_reconfiguration() {
        let original = [LogicalMonitor {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            scale: 1.0,
        }];
        let mut changed = original.clone();
        changed[0].scale = 1.25;
        assert!(!same_monitor_layout(&original, &changed));
        changed[0].scale = 1.0;
        changed[0].width = 1600;
        assert!(!same_monitor_layout(&original, &changed));
        assert!(same_monitor_layout(&original, &original));
    }

    #[test]
    fn capture_points_scale_to_portal_logical_stream_space() {
        let streams = [PortalStream {
            node_id: 1,
            position: Some((0, 0)),
            size: Some((1920, 1200)),
        }];
        let layout = [LogicalMonitor {
            x: 0,
            y: 0,
            width: 1920,
            height: 1200,
            scale: 4.0 / 3.0,
        }];

        assert_eq!(
            map_capture_point_to_stream_layout(&streams, &layout, 1280, 800, 2560, 1600),
            Some((960, 600))
        );
    }

    #[test]
    fn capture_points_preserve_negative_stream_layout_origins() {
        let streams = [
            PortalStream {
                node_id: 1,
                position: Some((-1920, 0)),
                size: Some((1920, 1080)),
            },
            PortalStream {
                node_id: 2,
                position: Some((0, 0)),
                size: Some((1920, 1080)),
            },
        ];
        let layout = [
            LogicalMonitor {
                x: -1920,
                y: 0,
                width: 1920,
                height: 1080,
                scale: 2.0,
            },
            LogicalMonitor {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                scale: 2.0,
            },
        ];

        assert_eq!(
            map_capture_point_to_stream_layout(&streams, &layout, 0, 0, 7680, 2160),
            Some((-1920, 0))
        );
        assert_eq!(
            map_capture_point_to_stream_layout(&streams, &layout, 3840, 1080, 7680, 2160),
            Some((0, 540))
        );
    }

    #[test]
    fn capture_point_mapping_requires_usable_stream_metadata() {
        let streams = [PortalStream {
            node_id: 1,
            position: None,
            size: None,
        }];
        let layout = [LogicalMonitor {
            x: 0,
            y: 0,
            width: 1920,
            height: 1200,
            scale: 1.0,
        }];

        assert_eq!(
            map_capture_point_to_stream_layout(&streams, &layout, 100, 200, 2560, 1600),
            None
        );
    }

    #[test]
    fn capture_point_mapping_rejects_invalid_desktop_monitors() {
        let streams = [PortalStream {
            node_id: 1,
            position: Some((0, 0)),
            size: Some((1920, 1080)),
        }];
        let layout = [
            LogicalMonitor {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                scale: 1.0,
            },
            LogicalMonitor {
                x: 1920,
                y: 0,
                width: 0,
                height: 1080,
                scale: 1.0,
            },
        ];

        assert_eq!(
            map_capture_point_to_stream_layout(&streams, &layout, 100, 200, 1920, 1080),
            None
        );
    }

    #[test]
    fn capture_point_mapping_rejects_partially_shared_desktops() {
        let streams = [PortalStream {
            node_id: 1,
            position: Some((0, 0)),
            size: Some((1920, 1080)),
        }];
        let layout = [
            LogicalMonitor {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                scale: 1.0,
            },
            LogicalMonitor {
                x: 1920,
                y: 0,
                width: 1920,
                height: 1080,
                scale: 1.0,
            },
        ];

        assert_eq!(
            map_capture_point_to_stream_layout(&streams, &layout, 3000, 500, 3840, 1080),
            None
        );
    }

    #[test]
    fn capture_point_mapping_rejects_mixed_scale_desktops() {
        let streams = [
            PortalStream {
                node_id: 1,
                position: Some((0, 0)),
                size: Some((1920, 1080)),
            },
            PortalStream {
                node_id: 2,
                position: Some((1920, 0)),
                size: Some((1920, 1080)),
            },
        ];
        let layout = [
            LogicalMonitor {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                scale: 1.0,
            },
            LogicalMonitor {
                x: 1920,
                y: 0,
                width: 1920,
                height: 1080,
                scale: 2.0,
            },
        ];

        assert_eq!(
            map_capture_point_to_stream_layout(&streams, &layout, 2000, 500, 3840, 1080),
            None
        );
    }

    #[test]
    fn unknown_multi_monitor_scale_only_allows_identity_mapping() {
        let streams = [
            PortalStream {
                node_id: 1,
                position: Some((0, 0)),
                size: Some((1920, 1080)),
            },
            PortalStream {
                node_id: 2,
                position: Some((1920, 0)),
                size: Some((1920, 1080)),
            },
        ];
        let layout = [
            LogicalMonitor {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                scale: 0.0,
            },
            LogicalMonitor {
                x: 1920,
                y: 0,
                width: 1920,
                height: 1080,
                scale: 0.0,
            },
        ];

        assert_eq!(
            map_capture_point_to_stream_layout(&streams, &layout, 2500, 500, 3840, 1080),
            Some((2500, 500))
        );
        assert_eq!(
            map_capture_point_to_stream_layout(&streams, &layout, 5000, 1000, 7680, 2160),
            None
        );
    }
}

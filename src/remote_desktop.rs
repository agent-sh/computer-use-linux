use crate::diagnostics::hydrate_session_bus_env;
use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use std::{collections::HashMap, time::Duration};
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
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

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
}

#[derive(Clone)]
pub struct PortalKeyboardSession {
    connection: Connection,
    session_handle: OwnedObjectPath,
}

#[derive(Debug, Clone)]
struct PortalStream {
    node_id: u32,
    position: Option<(i32, i32)>,
    size: Option<(i32, i32)>,
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
    select_pointer_devices(&connection, &session_handle).await?;
    select_monitor_sources(&connection, &session_handle).await?;
    let (devices, streams) = start_remote_desktop_session(&connection, &session_handle).await?;

    if devices & DEVICE_POINTER == 0 {
        bail!("remote desktop portal session started without pointer access");
    }
    if streams.is_empty() {
        bail!("remote desktop portal session started without any monitor streams");
    }

    Ok(PortalPointerSession {
        connection,
        session_handle,
        streams,
    })
}

pub async fn start_portal_keyboard_session() -> Result<PortalKeyboardSession> {
    hydrate_session_bus_env();

    let connection = Connection::session()
        .await
        .context("failed to connect to session bus for remote desktop portal")?;
    let session_handle = create_remote_desktop_session(&connection).await?;
    select_keyboard_devices(&connection, &session_handle).await?;
    let (devices, _) = start_remote_desktop_session(&connection, &session_handle).await?;

    if devices & DEVICE_KEYBOARD == 0 {
        bail!("remote desktop portal session started without keyboard access");
    }

    Ok(PortalKeyboardSession {
        connection,
        session_handle,
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
    let proxy = remote_desktop_proxy(&session.connection).await?;
    let (stream_id, x, y) = session.map_absolute_point(x, y)?;
    notify_pointer_motion_absolute(&proxy, &session.session_handle, stream_id, x, y).await?;
    for _ in 0..click_count.max(1) {
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
    }
    Ok(())
}

pub async fn scroll(
    session: &PortalPointerSession,
    target_point: Option<(i32, i32)>,
    direction: ScrollDirection,
    steps: i32,
) -> Result<()> {
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
    match std::env::var("COMPUTER_USE_LINUX_PORTAL_SCROLL_INVERT") {
        Ok(value) => {
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
        Err(_) => {}
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
    let proxy = remote_desktop_proxy(&session.connection).await?;
    let (start_stream, start_x, start_y) = session.map_absolute_point(start_x, start_y)?;
    notify_pointer_motion_absolute(
        &proxy,
        &session.session_handle,
        start_stream,
        start_x,
        start_y,
    )
    .await?;
    notify_pointer_button(
        &proxy,
        &session.session_handle,
        BTN_LEFT,
        POINTER_BUTTON_PRESSED,
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(35)).await;
    let (end_stream, end_x, end_y) = session.map_absolute_point(end_x, end_y)?;
    notify_pointer_motion_absolute(&proxy, &session.session_handle, end_stream, end_x, end_y)
        .await?;
    tokio::time::sleep(Duration::from_millis(35)).await;
    notify_pointer_button(
        &proxy,
        &session.session_handle,
        BTN_LEFT,
        POINTER_BUTTON_RELEASED,
    )
    .await
}

pub async fn type_text_with_keysyms(
    session: &PortalKeyboardSession,
    keysyms: &[i32],
) -> Result<()> {
    let proxy = remote_desktop_proxy(&session.connection).await?;
    for keysym in keysyms {
        notify_keyboard_keysym(&proxy, &session.session_handle, *keysym, KEY_PRESSED).await?;
        tokio::time::sleep(Duration::from_millis(5)).await;
        notify_keyboard_keysym(&proxy, &session.session_handle, *keysym, KEY_RELEASED).await?;
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    Ok(())
}

pub async fn press_keycode_chord(
    session: &PortalKeyboardSession,
    modifiers: &[i32],
    keycode: i32,
) -> Result<()> {
    let proxy = remote_desktop_proxy(&session.connection).await?;
    for modifier in modifiers {
        notify_keyboard_keycode(&proxy, &session.session_handle, *modifier, KEY_PRESSED).await?;
    }
    notify_keyboard_keycode(&proxy, &session.session_handle, keycode, KEY_PRESSED).await?;
    tokio::time::sleep(Duration::from_millis(35)).await;
    notify_keyboard_keycode(&proxy, &session.session_handle, keycode, KEY_RELEASED).await?;
    for modifier in modifiers.iter().rev() {
        notify_keyboard_keycode(&proxy, &session.session_handle, *modifier, KEY_RELEASED).await?;
    }
    Ok(())
}

impl PortalPointerSession {
    fn map_absolute_point(&self, x: i32, y: i32) -> Result<(u32, f64, f64)> {
        if let Some(stream) = self
            .streams
            .iter()
            .find(|stream| stream.contains_global_point(x, y))
        {
            return Ok(stream.relative_point(x, y));
        }

        self.streams
            .first()
            .map(|stream| stream.relative_point(x, y))
            .context("remote desktop portal session had no usable streams")
    }
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

    let handle: OwnedObjectPath = remote_proxy
        .call("CreateSession", &(options))
        .await
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

    let handle: OwnedObjectPath = remote_proxy
        .call("SelectDevices", &(session, options))
        .await
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
    options.insert("multiple", Value::from(false));
    options.insert("cursor_mode", Value::from(CURSOR_MODE_HIDDEN));

    let handle: OwnedObjectPath = screencast_proxy
        .call("SelectSources", &(session, options))
        .await
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

    let handle: OwnedObjectPath = remote_proxy
        .call("Start", &(session, "", options))
        .await
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
    let _: () = proxy
        .call(
            "NotifyPointerMotionAbsolute",
            &(session, options, stream_id, x, y),
        )
        .await
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
    let _: () = proxy
        .call("NotifyPointerButton", &(session, options, button, state))
        .await
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
    let _: () = proxy
        .call(
            "NotifyPointerAxisDiscrete",
            &(session, options, axis, steps),
        )
        .await
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
    let _: () = proxy
        .call("NotifyKeyboardKeysym", &(session, options, keysym, state))
        .await
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
    let _: () = proxy
        .call("NotifyKeyboardKeycode", &(session, options, keycode, state))
        .await
        .context("RemoteDesktop NotifyKeyboardKeycode failed")?;
    Ok(())
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
            portal_scroll_axis_steps(
                ScrollDirection::Up,
                1,
                PortalScrollPolarity::InvertVertical
            ),
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
}

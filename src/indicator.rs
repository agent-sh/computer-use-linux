//! On-screen indicator for agent activity.
//!
//! The MCP server reports each desktop action to the
//! `computer-use-linux-indicator` overlay as one JSON datagram on a socket in
//! `$XDG_RUNTIME_DIR`. The overlay draws a software cursor that glides to the
//! target, keycaps for key presses, an edge glow and a status pill. The server
//! starts the overlay on the first action. Set `COMPUTER_USE_LINUX_INDICATOR=0`
//! to turn it off.
//!
//! Before a screen capture the server sends `hide` and the overlay answers
//! with [`HIDE_REPLY_SHOWN`] or [`HIDE_REPLY_EMPTY`], so captures wait for the
//! compositor only when something was actually on screen.

use std::{
    env,
    io::ErrorKind,
    os::linux::net::SocketAddrExt,
    os::unix::net::{SocketAddr, UnixDatagram},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicU32, Ordering},
        Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

use rmcp::{Peer, RoleServer};
use serde::{Deserialize, Serialize};

/// Overlay binary name, resolved next to the server binary or on `PATH`.
pub const INDICATOR_BINARY: &str = "computer-use-linux-indicator";
/// Socket file name inside `$XDG_RUNTIME_DIR`.
pub const SOCKET_NAME: &str = "computer-use-linux-indicator.sock";
/// Reply to `hide` when the overlay had something on screen.
pub const HIDE_REPLY_SHOWN: &[u8] = b"shown";
/// Reply to `hide` when nothing was on screen.
pub const HIDE_REPLY_EMPTY: &[u8] = b"empty";

const DISABLE_ENV: &str = "COMPUTER_USE_LINUX_INDICATOR";
const HIDE_TEXT_ENV: &str = "COMPUTER_USE_LINUX_INDICATOR_HIDE_TEXT";
const AGENT_ENV: &str = "COMPUTER_USE_LINUX_AGENT_NAME";
const BINARY_ENV: &str = "COMPUTER_USE_LINUX_INDICATOR_BIN";

/// How long the overlay cursor takes to reach a target. Pointer actions wait
/// this long so the cursor lands before the real input happens.
pub const GLIDE: Duration = Duration::from_millis(350);
/// Time for the compositor to drop overlay surfaces before a capture.
const HIDE_SETTLE: Duration = Duration::from_millis(120);
/// How long to wait for the overlay to answer `hide`.
const HIDE_REPLY_WAIT: Duration = Duration::from_millis(100);
const SPAWN_WAIT: Duration = Duration::from_secs(1);
const TEXT_LIMIT: usize = 64;

/// One overlay update.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndicatorEvent {
    /// Remove the overlay immediately, e.g. right before a screen capture.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hide: bool,
    /// Display name of the agent, e.g. `Claude`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub agent: String,
    /// MCP tool name, e.g. `click`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool: String,
    /// Target in screenshot pixels, the coordinate space of the click tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<i32>,
    /// Size of the full-desktop capture that `x`/`y` refer to. The overlay
    /// spreads it over the logical output layout, as the compositor does for
    /// the absolute pointer. Without it `x`/`y` are taken as logical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space: Option<(u32, u32)>,
    /// Key chord parts for `press_key`, e.g. `["ctrl", "l"]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<String>,
    /// Typed text (tail only, possibly masked).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// Path of the overlay socket, or `None` without `XDG_RUNTIME_DIR`.
pub fn socket_path() -> Option<PathBuf> {
    env::var_os("XDG_RUNTIME_DIR")
        .filter(|dir| !dir.is_empty())
        .map(|dir| Path::new(&dir).join(SOCKET_NAME))
}

/// Maps an MCP `clientInfo` to the name shown on screen.
pub fn agent_display_name(name: &str, title: Option<&str>) -> String {
    let lower = name.to_ascii_lowercase();
    let known = [
        ("claude", "Claude"),
        ("codex", "Codex"),
        ("opencode", "OpenCode"),
        ("gemini", "Gemini"),
        ("antigravity", "Antigravity"),
        ("hermes", "Hermes"),
        ("cursor", "Cursor"),
        ("goose", "Goose"),
    ];
    if let Some((_, display)) = known.iter().find(|(needle, _)| lower.contains(needle)) {
        return (*display).to_string();
    }
    // Pi's native extension identifies as `computer-use-linux-pi`.
    if lower == "pi" || lower.starts_with("pi-") || lower.ends_with("-pi") {
        return "Pi".to_string();
    }
    if let Some(title) = title.map(str::trim).filter(|title| !title.is_empty()) {
        return title.to_string();
    }
    match name.trim() {
        "" => "Agent".to_string(),
        other => other.to_string(),
    }
}

fn disabled_value(value: Option<&str>) -> bool {
    matches!(
        value
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        Some("0" | "false" | "off" | "no")
    )
}

/// Last `TEXT_LIMIT` characters, masked when requested.
fn shown_text(text: &str, mask: bool) -> String {
    let tail: String = {
        let chars: Vec<char> = text.chars().collect();
        chars[chars.len().saturating_sub(TEXT_LIMIT)..]
            .iter()
            .collect()
    };
    if mask {
        "•".repeat(tail.chars().count())
    } else {
        tail
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Overlay {
    NotStarted,
    Started(Instant),
    /// No Wayland session, no binary, or the overlay could not start (for
    /// example GNOME, which has no layer-shell). Not retried by this server.
    Unavailable,
}

/// Server-side sender. Every method is a cheap no-op when disabled.
#[derive(Debug)]
pub(crate) struct Indicator {
    enabled: bool,
    mask_text: bool,
    agent_override: Option<String>,
    peer: OnceLock<Peer<RoleServer>>,
    socket: Option<UnixDatagram>,
    overlay: Mutex<Overlay>,
}

impl Default for Indicator {
    fn default() -> Self {
        // Unit tests build servers freely; they must not drive a real overlay.
        let enabled = !cfg!(test) && !disabled_value(env::var(DISABLE_ENV).ok().as_deref());
        let socket = enabled
            .then(client_socket)
            .flatten()
            .filter(|socket| socket.set_nonblocking(true).is_ok());
        Self {
            enabled: socket.is_some(),
            mask_text: env::var(HIDE_TEXT_ENV).ok().as_deref() == Some("1"),
            agent_override: env::var(AGENT_ENV)
                .ok()
                .filter(|name| !name.trim().is_empty()),
            peer: OnceLock::new(),
            socket,
            overlay: Mutex::new(Overlay::NotStarted),
        }
    }
}

impl Indicator {
    /// Remembers the MCP client so events carry its name.
    pub(crate) fn attach(&self, peer: Peer<RoleServer>) {
        let _ = self.peer.set(peer);
    }

    fn agent(&self) -> String {
        if let Some(name) = &self.agent_override {
            return name.clone();
        }
        self.peer
            .get()
            .and_then(Peer::peer_info)
            .map(|info| {
                agent_display_name(&info.client_info.name, info.client_info.title.as_deref())
            })
            .unwrap_or_else(|| "Agent".to_string())
    }

    fn event(&self, tool: &str) -> IndicatorEvent {
        IndicatorEvent {
            agent: self.agent(),
            tool: tool.to_string(),
            ..IndicatorEvent::default()
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    /// Moves the overlay cursor to a point in screenshot pixels and waits until
    /// it lands. Without the capture size the point cannot be placed, so only
    /// the action is shown.
    pub(crate) async fn pointer(&self, tool: &str, (x, y): (i32, i32), space: Option<(u32, u32)>) {
        let Some(space) = space else {
            return self.action(tool).await;
        };
        let event = IndicatorEvent {
            x: Some(x),
            y: Some(y),
            space: Some(space),
            ..self.event(tool)
        };
        if self.send(&event).await {
            tokio::time::sleep(GLIDE).await;
        }
    }

    /// Moves the overlay cursor without waiting, e.g. to a drop point.
    pub(crate) async fn follow(&self, tool: &str, (x, y): (i32, i32), space: Option<(u32, u32)>) {
        let Some(space) = space else {
            return;
        };
        let event = IndicatorEvent {
            x: Some(x),
            y: Some(y),
            space: Some(space),
            ..self.event(tool)
        };
        self.send(&event).await;
    }

    /// Reports an action that moves no pointer.
    pub(crate) async fn action(&self, tool: &str) {
        self.send(&self.event(tool)).await;
    }

    pub(crate) async fn keys(&self, chord: &str) {
        let event = IndicatorEvent {
            keys: chord
                .split('+')
                .map(str::trim)
                .filter(|key| !key.is_empty())
                .map(str::to_string)
                .collect(),
            ..self.event("press_key")
        };
        self.send(&event).await;
    }

    /// Shows entered text; `secret` masks it regardless of the opt-in.
    pub(crate) async fn text(&self, tool: &str, text: &str, secret: bool) {
        let event = IndicatorEvent {
            text: Some(shown_text(text, self.mask_text || secret)),
            ..self.event(tool)
        };
        self.send(&event).await;
    }

    /// Hides the overlay so a screen capture never contains it. The overlay is
    /// shared by every agent in the session, so this asks it directly instead
    /// of trusting what this server last sent.
    pub(crate) async fn before_capture(&self) {
        let Some(socket) = self.socket.as_ref().filter(|_| self.enabled) else {
            return;
        };
        let mut reply = [0u8; 16];
        // Drop a late answer to an earlier request.
        while socket.recv(&mut reply).is_ok() {}
        if !self.send_raw(&IndicatorEvent {
            hide: true,
            ..Default::default()
        }) {
            // No overlay is running, so there is nothing to hide.
            return;
        }
        let deadline = Instant::now() + HIDE_REPLY_WAIT;
        while Instant::now() < deadline {
            if let Ok(len) = socket.recv(&mut reply) {
                if &reply[..len] == HIDE_REPLY_EMPTY {
                    return;
                }
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        // Shown, or no answer (an older overlay): assume it was visible.
        tokio::time::sleep(HIDE_SETTLE).await;
    }

    /// Announces a finished capture ("looking at the screen").
    pub(crate) async fn after_capture(&self, tool: &str) {
        self.send(&self.event(tool)).await;
    }

    fn send_raw(&self, event: &IndicatorEvent) -> bool {
        let (Some(socket), Some(path)) = (&self.socket, socket_path()) else {
            return false;
        };
        let Ok(payload) = serde_json::to_vec(event) else {
            return false;
        };
        socket.send_to(&payload, path).is_ok()
    }

    /// Sends an event, starting the overlay first if it is not running.
    async fn send(&self, event: &IndicatorEvent) -> bool {
        if !self.enabled {
            return false;
        }
        let (Some(socket), Some(path)) = (&self.socket, socket_path()) else {
            return false;
        };
        let Ok(payload) = serde_json::to_vec(event) else {
            return false;
        };
        let sent = match socket.send_to(&payload, &path) {
            Ok(_) => true,
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::NotFound | ErrorKind::ConnectionRefused
                ) =>
            {
                self.start_overlay(&path).await && socket.send_to(&payload, &path).is_ok()
            }
            Err(_) => false,
        };
        sent
    }

    async fn start_overlay(&self, path: &Path) -> bool {
        {
            let Ok(mut overlay) = self.overlay.lock() else {
                return false;
            };
            match *overlay {
                Overlay::Unavailable => return false,
                // Still starting, or exited after idling: allow a restart later.
                Overlay::Started(at) if at.elapsed() < SPAWN_WAIT => return false,
                _ => {}
            }
            if env::var_os("WAYLAND_DISPLAY").is_none() || spawn_overlay().is_err() {
                *overlay = Overlay::Unavailable;
                return false;
            }
            *overlay = Overlay::Started(Instant::now());
        }
        let deadline = Instant::now() + SPAWN_WAIT;
        while Instant::now() < deadline {
            if UnixDatagram::unbound()
                .and_then(|probe| probe.connect(path))
                .is_ok()
            {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        if let Ok(mut overlay) = self.overlay.lock() {
            *overlay = Overlay::Unavailable;
        }
        false
    }
}

/// A socket the overlay can answer: bound to a unique abstract address. Falls
/// back to an unbound socket, which only loses the `hide` replies.
fn client_socket() -> Option<UnixDatagram> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let name = format!(
        "computer-use-linux-indicator-client.{}.{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    SocketAddr::from_abstract_name(name.as_bytes())
        .and_then(|address| UnixDatagram::bind_addr(&address))
        .or_else(|_| UnixDatagram::unbound())
        .ok()
}

/// Whether an AT-SPI role is a password field (`password text`, or the
/// `PasswordText` fallback name).
pub(crate) fn is_secret_role(role: &str) -> bool {
    role.to_ascii_lowercase().contains("password")
}

fn overlay_binary() -> PathBuf {
    if let Some(path) = env::var_os(BINARY_ENV).filter(|path| !path.is_empty()) {
        return PathBuf::from(path);
    }
    env::current_exe()
        .ok()
        .map(|exe| exe.with_file_name(INDICATOR_BINARY))
        .filter(|sibling| sibling.is_file())
        .unwrap_or_else(|| PathBuf::from(INDICATOR_BINARY))
}

fn spawn_overlay() -> std::io::Result<()> {
    let mut child = tokio::process::Command::new(overlay_binary())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // Own process group: a Ctrl-C aimed at the agent must not kill the
        // overlay that other agents may be sharing.
        .process_group(0)
        .spawn()?;
    // Reap it when it exits (it quits on its own after idling).
    tokio::spawn(async move {
        let _ = child.wait().await;
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_names_come_from_client_info() {
        assert_eq!(agent_display_name("claude-code", None), "Claude");
        assert_eq!(agent_display_name("codex-mcp-client", None), "Codex");
        assert_eq!(agent_display_name("opencode", None), "OpenCode");
        assert_eq!(agent_display_name("pi", None), "Pi");
        assert_eq!(agent_display_name("computer-use-linux-pi", None), "Pi");
        assert_eq!(
            agent_display_name("pipeline", Some("Pipeline Bot")),
            "Pipeline Bot"
        );
        assert_eq!(agent_display_name("my-agent", None), "my-agent");
        assert_eq!(agent_display_name(" ", None), "Agent");
    }

    #[test]
    fn password_fields_are_secret() {
        assert!(is_secret_role("password text"));
        assert!(is_secret_role("PasswordText"));
        assert!(!is_secret_role("text"));
        assert!(!is_secret_role("entry"));
    }

    #[test]
    fn hide_is_answered_through_the_client_socket() {
        let overlay_path =
            std::env::temp_dir().join(format!("cul-indicator-test-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&overlay_path);
        let overlay = UnixDatagram::bind(&overlay_path).unwrap();
        let client = client_socket().unwrap();
        client.send_to(br#"{"hide":true}"#, &overlay_path).unwrap();
        let mut buf = [0u8; 64];
        let (len, from) = overlay.recv_from(&mut buf).unwrap();
        assert_eq!(&buf[..len], br#"{"hide":true}"#);
        overlay.send_to_addr(HIDE_REPLY_EMPTY, &from).unwrap();
        let len = client.recv(&mut buf).unwrap();
        assert_eq!(&buf[..len], HIDE_REPLY_EMPTY);
        let _ = std::fs::remove_file(&overlay_path);
    }

    #[test]
    fn opt_out_values() {
        for value in ["0", "false", "OFF", "no"] {
            assert!(disabled_value(Some(value)), "{value}");
        }
        for value in [None, Some("1"), Some("")] {
            assert!(!disabled_value(value), "{value:?}");
        }
    }

    #[test]
    fn text_is_trimmed_to_its_tail_and_masked() {
        let long = "a".repeat(80) + "end";
        assert_eq!(shown_text(&long, false).chars().count(), TEXT_LIMIT);
        assert!(shown_text(&long, false).ends_with("end"));
        assert_eq!(shown_text("пароль", true), "••••••");
    }

    #[test]
    fn events_serialize_compactly() {
        let event = IndicatorEvent {
            agent: "Claude".into(),
            tool: "click".into(),
            x: Some(10),
            y: Some(20),
            ..Default::default()
        };
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(json, r#"{"agent":"Claude","tool":"click","x":10,"y":20}"#);
        assert_eq!(
            serde_json::from_str::<IndicatorEvent>(&json).unwrap(),
            event
        );
        let scaled = IndicatorEvent {
            space: Some((3840, 2160)),
            ..event
        };
        let json = serde_json::to_string(&scaled).unwrap();
        assert!(json.ends_with(r#""space":[3840,2160]}"#), "{json}");
        assert_eq!(
            serde_json::from_str::<IndicatorEvent>(&json).unwrap(),
            scaled
        );
        let hide = serde_json::to_string(&IndicatorEvent {
            hide: true,
            ..Default::default()
        });
        assert_eq!(hide.unwrap(), r#"{"hide":true}"#);
    }
}

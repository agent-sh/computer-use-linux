use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const TOKEN: &str = "cul-secret";

/// A private temp dir with stub desktop tools, so a server under test cannot
/// reach the developer's session: diagnostics only fills empty variables
/// from the process tree, so every desktop variable gets a dead value here.
struct Hermetic {
    dir: PathBuf,
}

impl Hermetic {
    fn new() -> Hermetic {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("cul-http-{}-{nanos}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        for program in [
            "ydotool",
            "xdotool",
            "wtype",
            "systemctl",
            "gnome-screenshot",
            "busctl",
            "gsettings",
        ] {
            let path = dir.join(program);
            std::fs::write(
                &path,
                // Read-only probes (environment, capability help) fail silently;
                // anything else these tools run would be input, so record it.
                r#"#!/bin/sh
case "${0##*/} $1" in
  "systemctl "*|"busctl "*|"gsettings "*|"ydotool help"|"ydotool --help") exit 1 ;;
esac
printf '%s %s\n' "${0##*/}" "$*" >> "${0%/*}/commands.log"
exit 1
"#,
            )
            .unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        Hermetic { dir }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_computer-use-linux"));
        command
            .env_clear()
            .env("PATH", &self.dir)
            .env("HOME", &self.dir)
            .env("XDG_RUNTIME_DIR", &self.dir)
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!("unix:path={}/no-bus", self.dir.display()),
            )
            .env("DISPLAY", ":9876")
            .env("WAYLAND_DISPLAY", "no-wayland")
            .env("XDG_SESSION_TYPE", "wayland")
            .env("YDOTOOL_SOCKET", self.dir.join("no-ydotool"))
            .env("NIRI_SOCKET", self.dir.join("no-niri"))
            .env("CU_DISABLE_ABS_POINTER", "1");
        // Keep in step with DESKTOP_ENV_KEYS in src/diagnostics.rs: the server
        // fills any empty one from the process tree; checked by
        // hermetic_sets_every_desktop_key.
        for key in [
            "DESKTOP_SESSION",
            "HYPRLAND_INSTANCE_SIGNATURE",
            "XAUTHORITY",
            "XDG_SESSION_DESKTOP",
            "XDG_CURRENT_DESKTOP",
        ] {
            command.env(key, "hermetic-test");
        }
        command
    }

    fn recorded(&self) -> String {
        std::fs::read_to_string(self.dir.join("commands.log")).unwrap_or_default()
    }
}

#[test]
fn hermetic_sets_every_desktop_key() {
    let source = include_str!("../src/diagnostics.rs");
    let list = source
        .split_once("const DESKTOP_ENV_KEYS: &[&str] = &[")
        .and_then(|(_, rest)| rest.split_once("];"))
        .expect("DESKTOP_ENV_KEYS not found in src/diagnostics.rs")
        .0;
    let hermetic = Hermetic::new();
    let command = hermetic.command();
    let keys: Vec<&str> = list.split('"').skip(1).step_by(2).collect();
    assert!(!keys.is_empty());
    for key in keys {
        let set = command
            .get_envs()
            .any(|(name, value)| name == key && value.is_some_and(|v| !v.is_empty()));
        assert!(set, "Hermetic::command() leaves {key} empty");
    }
}

impl Drop for Hermetic {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn spawn(hermetic: &Hermetic, token: Option<&str>) -> std::process::Child {
    let mut command = hermetic.command();
    command
        .args(["mcp", "--listen", "127.0.0.1:0"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if let Some(token) = token {
        command.env("COMPUTER_USE_LINUX_MCP_TOKEN", token);
    }
    command.spawn().unwrap()
}

fn bound_port(child: &mut std::process::Child) -> u16 {
    let stderr = child.stderr.as_mut().unwrap();
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        if stderr.read(&mut byte).unwrap() == 0 {
            break;
        }
        buf.push(byte[0]);
        if buf.ends_with(b"\n") {
            let line = String::from_utf8_lossy(&buf);
            if let Some(addr) = line.trim().strip_prefix("listening ") {
                return addr.rsplit(':').next().unwrap().parse().unwrap();
            }
            buf.clear();
        }
    }
    panic!(
        "server did not print a listen address: {}",
        String::from_utf8_lossy(&buf)
    );
}

fn post(
    port: u16,
    token: Option<&str>,
    body: &str,
    session: Option<&str>,
) -> (u16, String, Option<String>) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(12)))
        .unwrap();
    let mut headers = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(token) = token {
        headers.push_str(&format!("Authorization: Bearer {token}\r\n"));
    }
    if let Some(session) = session {
        headers.push_str(&format!("Mcp-Session-Id: {session}\r\n"));
    }
    headers.push_str("\r\n");
    stream.write_all(headers.as_bytes()).unwrap();
    stream.write_all(body.as_bytes()).unwrap();
    let mut raw = Vec::new();
    let _ = stream.read_to_end(&mut raw);
    let text = String::from_utf8_lossy(&raw).to_string();
    let status = text
        .split_whitespace()
        .nth(1)
        .unwrap_or("0")
        .parse()
        .unwrap_or(0);
    let session = text.lines().find_map(|line| {
        line.split_once(':')
            .filter(|(name, _)| name.eq_ignore_ascii_case("mcp-session-id"))
            .map(|(_, value)| value.trim().to_string())
    });
    (status, text, session)
}

fn initialize(port: u16, name: &str) -> String {
    let body = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2024-11-05","capabilities":{{}},"clientInfo":{{"name":"{name}","version":"0"}}}}}}"#
    );
    let (status, text, session) = post(port, Some(TOKEN), &body, None);
    assert_eq!(status, 200, "{text}");
    session.expect("session id")
}

#[test]
fn listen_requires_token() {
    let hermetic = Hermetic::new();
    for token in [None, Some("")] {
        let mut child = spawn(&hermetic, token);
        let status = child.wait().unwrap();
        let mut err = String::new();
        child.stderr.unwrap().read_to_string(&mut err).unwrap();
        assert!(!status.success(), "{err}");
        assert!(err.contains("COMPUTER_USE_LINUX_MCP_TOKEN"), "{err}");
    }
}

#[test]
fn http_initialize_and_list_tools() {
    let hermetic = Hermetic::new();
    let mut child = spawn(&hermetic, Some(TOKEN));
    let port = bound_port(&mut child);
    let session = initialize(port, "cul");
    let _ = post(
        port,
        Some(TOKEN),
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        Some(&session),
    );
    let (status, body, _) = post(
        port,
        Some(TOKEN),
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
        Some(&session),
    );
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("get_app_state"), "{body}");
    for bad in [None, Some("wrong")] {
        let (status, body, _) = post(
            port,
            bad,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            None,
        );
        assert_eq!(status, 401, "{body}");
        assert!(!body.contains(TOKEN), "{body}");
    }
    let _ = child.kill();
    assert_eq!(hermetic.recorded(), "");
}

#[test]
fn http_client_disconnect_does_not_wedge() {
    let hermetic = Hermetic::new();
    let mut child = spawn(&hermetic, Some(TOKEN));
    let port = bound_port(&mut child);
    let session = initialize(port, "a");
    let mut dropped = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let body = r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"click","arguments":{"x":1,"y":1}}}"#;
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nAuthorization: Bearer {TOKEN}\r\nMcp-Session-Id: {session}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    dropped.write_all(request.as_bytes()).unwrap();
    drop(dropped);
    let (status, body, _) = post(
        port,
        Some(TOKEN),
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"click","arguments":{"x":3,"y":3}}}"#,
        Some(&session),
    );
    assert_eq!(status, 200, "{body}");
    let _ = child.kill();
    assert_eq!(hermetic.recorded(), "");
}

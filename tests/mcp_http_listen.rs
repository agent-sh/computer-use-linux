use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const TOKEN: &str = "cul-secret";

fn spawn(token: Option<&str>, hold_ms: Option<&str>) -> std::process::Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_computer-use-linux"));
    command
        .args(["mcp", "--listen", "127.0.0.1:0"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if let Some(token) = token {
        command.env("COMPUTER_USE_LINUX_MCP_TOKEN", token);
    }
    if let Some(ms) = hold_ms {
        command.env("CUL_INPUT_LOCK_HOLD_MS", ms);
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
    for token in [None, Some("")] {
        let mut child = spawn(token, None);
        let status = child.wait().unwrap();
        let mut err = String::new();
        child.stderr.unwrap().read_to_string(&mut err).unwrap();
        assert!(!status.success(), "{err}");
        assert!(err.contains("COMPUTER_USE_LINUX_MCP_TOKEN"), "{err}");
    }
}

#[test]
fn http_initialize_and_list_tools() {
    let mut child = spawn(Some(TOKEN), None);
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
}

// Needs CUL_INPUT_LOCK_HOLD_MS, which only debug builds read.
#[cfg(debug_assertions)]
#[test]
fn http_sessions_share_input_lock() {
    let mut child = spawn(Some(TOKEN), Some("400"));
    let port = bound_port(&mut child);
    let session_a = initialize(port, "a");
    let session_b = initialize(port, "b");
    let started = Instant::now();
    let left = std::thread::spawn(move || {
        post(
            port,
            Some(TOKEN),
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"click","arguments":{"x":1,"y":1}}}"#,
            Some(&session_a),
        )
    });
    let right = std::thread::spawn(move || {
        post(
            port,
            Some(TOKEN),
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"click","arguments":{"x":2,"y":2}}}"#,
            Some(&session_b),
        )
    });
    let (left_status, left_body, _) = left.join().unwrap();
    let (right_status, right_body, _) = right.join().unwrap();
    assert!(
        started.elapsed() >= Duration::from_millis(700),
        "clicks overlapped: left={left_body} right={right_body}"
    );
    assert_eq!(left_status, 200, "{left_body}");
    assert_eq!(right_status, 200, "{right_body}");
    let _ = child.kill();
}

#[test]
fn http_client_disconnect_does_not_wedge() {
    let mut child = spawn(Some(TOKEN), Some("200"));
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
}

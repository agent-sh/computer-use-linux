use anyhow::{bail, Context, Result};
use schemars::JsonSchema;
use serde::Serialize;
use std::process::Command;

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ClipboardContents {
    pub text: String,
    pub backend: String,
}

pub fn get_clipboard() -> Result<ClipboardContents> {
    if let Ok(text) = run_capture(&["wl-paste", "--no-newline"]) {
        return Ok(ClipboardContents {
            text,
            backend: "wl-clipboard".to_string(),
        });
    }
    if let Ok(text) = run_capture(&["xclip", "-selection", "clipboard", "-o"]) {
        return Ok(ClipboardContents {
            text,
            backend: "xclip".to_string(),
        });
    }
    if let Ok(text) = run_capture(&["xsel", "--clipboard", "--output"]) {
        return Ok(ClipboardContents {
            text,
            backend: "xsel".to_string(),
        });
    }
    bail!("clipboard read failed: install wl-clipboard (Wayland) or xclip/xsel (X11)")
}

pub fn set_clipboard(text: &str) -> Result<String> {
    if run_paste_stdin(&["wl-copy"], text).is_ok() {
        return Ok("wl-clipboard".to_string());
    }
    if run_paste_stdin(&["xclip", "-selection", "clipboard"], text).is_ok() {
        return Ok("xclip".to_string());
    }
    if run_paste_stdin(&["xsel", "--clipboard", "--input"], text).is_ok() {
        return Ok("xsel".to_string());
    }
    bail!("clipboard write failed: install wl-clipboard (Wayland) or xclip/xsel (X11)")
}

fn run_capture(command: &[&str]) -> Result<String> {
    let (program, args) = command
        .split_first()
        .context("clipboard command must include a program")?;
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("failed to run {program}"))?;
    if !output.status.success() {
        bail!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn run_paste_stdin(command: &[&str], text: &str) -> Result<()> {
    let (program, args) = command
        .split_first()
        .context("clipboard command must include a program")?;
    let mut child = Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn {program}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin
            .write_all(text.as_bytes())
            .with_context(|| format!("failed to write clipboard payload to {program}"))?;
    }
    let status = child
        .wait()
        .with_context(|| format!("failed waiting for {program}"))?;
    if status.success() {
        Ok(())
    } else {
        bail!("{program} exited with {status}")
    }
}
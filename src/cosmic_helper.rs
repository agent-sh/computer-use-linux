use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

pub const COSMIC_HELPER_BINARY: &str = "computer-use-linux-cosmic";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CosmicHelperProbe {
    pub ok: bool,
    pub can_list_windows: bool,
    pub can_activate_windows: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CosmicHelperActivation {
    pub ok: bool,
    pub detail: String,
}

pub fn resolve_helper_binary() -> Result<PathBuf> {
    if let Some(path) = env_var("COMPUTER_USE_LINUX_COSMIC_HELPER") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Ok(path);
        }
    }

    if let Ok(current_exe) = env::current_exe() {
        let sibling = current_exe.with_file_name(COSMIC_HELPER_BINARY);
        if sibling.exists() {
            return Ok(sibling);
        }
    }

    if let Some(path) = command_path(COSMIC_HELPER_BINARY) {
        return Ok(path);
    }

    bail!("COSMIC helper binary {COSMIC_HELPER_BINARY} not found")
}

fn env_var(key: &str) -> Option<String> {
    env::var(key).ok().filter(|value| !value.trim().is_empty())
}

pub fn probe() -> Result<CosmicHelperProbe> {
    run_json_command(["probe"])
}

pub fn list_windows_json() -> Result<String> {
    run_text_command(["list-windows"])
}

pub fn focused_window_json() -> Result<String> {
    run_text_command(["focused-window"])
}

pub fn activate_window(window_id: u64) -> Result<CosmicHelperActivation> {
    run_json_command(["activate-window", "--window-id", &window_id.to_string()])
}

fn run_json_command<T, I, S>(args: I) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let output = run_command(args)?;
    serde_json::from_str(&output)
        .with_context(|| format!("failed to parse {COSMIC_HELPER_BINARY} JSON output"))
}

fn run_text_command<I, S>(args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    run_command(args)
}

fn run_command<I, S>(args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let helper = resolve_helper_binary()?;
    let args = args
        .into_iter()
        .map(|arg| arg.as_ref().to_string())
        .collect::<Vec<_>>();
    let output = Command::new(&helper)
        .args(&args)
        .output()
        .with_context(|| format!("failed to run {}", helper.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        bail!(
            "{} {} failed{}",
            helper.display(),
            args.join(" "),
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        );
    }
    String::from_utf8(output.stdout)
        .map(|text| text.trim().to_string())
        .context("helper output was not valid UTF-8")
}

fn command_path(binary: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|entry| entry.join(binary))
        .find(|candidate| candidate.is_file() && is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|metadata| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                metadata.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                metadata.is_file()
            }
        })
        .unwrap_or(false)
}

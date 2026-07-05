//! Generic X11 / EWMH window backend.
//!
//! Unlike the compositor-specific backends (GNOME Shell, KWin, Hyprland, i3),
//! this one talks plain [EWMH]/[ICCCM] through `wmctrl` + `xprop`, so it works
//! on any reasonably standards-compliant X11 window manager that does not have
//! a dedicated backend — Cinnamon/Muffin, MATE/Marco, Xfce/xfwm4, Openbox, etc.
//! It is intentionally registered last so a session-native backend always wins
//! when one is present.
//!
//! [EWMH]: https://specifications.freedesktop.org/wm-spec/latest/
//! [ICCCM]: https://tronche.com/gui/x/icccm/

use crate::terminal::enrich_terminal_windows;
use crate::windowing::registry::BackendProbe;
use crate::windowing::types::{WindowBounds, WindowInfo};
use anyhow::{bail, Context, Result};
use std::env;
use std::process::Command;

pub const X11_BACKEND: &str = "x11";

/// True when this looks like a plain X11 session we can drive over EWMH.
///
/// Requires an X `DISPLAY` and either an explicit `x11` session type or the
/// absence of a Wayland display, so we never hijack XWayland under a Wayland
/// compositor (where a native backend should answer instead).
fn is_x11_session() -> bool {
    if env_nonempty("DISPLAY").is_none() {
        return false;
    }
    match env_nonempty("XDG_SESSION_TYPE").as_deref() {
        Some("x11") => true,
        Some("wayland") => false,
        _ => env_nonempty("WAYLAND_DISPLAY").is_none(),
    }
}

pub fn probe() -> BackendProbe {
    if !is_x11_session() {
        return probe_fail("no X11 session (needs DISPLAY on an X11, not Wayland, session)");
    }
    match wmctrl().args(["-l", "-p", "-G", "-x"]).output() {
        Ok(output) if output.status.success() => {
            // Listing only needs wmctrl, but the `focused` flag (and therefore
            // focused_window() and activate_window's focus verification) comes
            // from `_NET_ACTIVE_WINDOW` read via xprop. Without xprop we can list
            // but cannot verify focus, so don't advertise focus capabilities.
            let can_focus = command_on_path("xprop");
            BackendProbe {
                id: X11_BACKEND,
                ok: true,
                can_list_windows: true,
                can_focus_apps: can_focus,
                can_focus_windows: can_focus,
                detail: if can_focus {
                    "wmctrl listed X11/EWMH windows".to_string()
                } else {
                    "wmctrl listed X11/EWMH windows; xprop missing, so focused-window verification is unavailable".to_string()
                },
            }
        }
        Ok(output) => probe_fail(&format!(
            "wmctrl -l failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(error) => probe_fail(&format!("wmctrl unavailable: {error}")),
    }
}

fn probe_fail(detail: &str) -> BackendProbe {
    BackendProbe {
        id: X11_BACKEND,
        ok: false,
        can_list_windows: false,
        can_focus_apps: false,
        can_focus_windows: false,
        detail: detail.to_string(),
    }
}

pub fn list_windows() -> Result<Vec<WindowInfo>> {
    // Guard the session too, not just probe(): registry::list_windows() tries
    // each backend directly, so without this a Wayland session with no native
    // backend would fall through here and return XWayland-only windows.
    if !is_x11_session() {
        bail!("not an X11 session (needs DISPLAY on an X11, not Wayland, session)");
    }
    let output = wmctrl()
        .args(["-l", "-p", "-G", "-x"])
        .output()
        .context("failed to run wmctrl -l -p -G -x")?;
    if !output.status.success() {
        bail!(
            "wmctrl -l -p -G -x failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let active_id = active_window_id();
    let mut windows = parse_wmctrl_windows(&String::from_utf8_lossy(&output.stdout), active_id);
    enrich_terminal_windows(&mut windows);
    Ok(windows)
}

pub fn activate_window(window_id: u64) -> Result<()> {
    let id = window_id_arg(window_id);
    run_wmctrl(&["-i", "-a", id.as_str()], "activate window", window_id)
}

pub fn move_window(window_id: u64, x: i32, y: i32) -> Result<String> {
    unmaximize(window_id)?;
    let id = window_id_arg(window_id);
    // wmctrl -e is `gravity,x,y,width,height`; the trailing -1,-1 keep the size.
    // wmctrl also reads -1 in the x/y fields as "preserve current position", so a
    // literal -1 target would be dropped — nudge it to -2 so the move still lands.
    let geometry = format!("0,{},{},-1,-1", wmctrl_move_coord(x), wmctrl_move_coord(y));
    run_wmctrl(
        &["-i", "-r", id.as_str(), "-e", geometry.as_str()],
        "move window",
        window_id,
    )?;
    Ok(format!("Moved window to ({x}, {y}) via X11/EWMH (wmctrl)."))
}

/// `wmctrl -e` treats -1 in any field as "keep current value", so a literal -1
/// coordinate is silently ignored. Map it to -2 so the window actually moves
/// (a 1px difference at the screen edge is harmless).
fn wmctrl_move_coord(value: i32) -> i32 {
    if value == -1 {
        -2
    } else {
        value
    }
}

pub fn resize_window(window_id: u64, width: i32, height: i32) -> Result<String> {
    // wmctrl -e reads -1 (and rejects <= 0) as "preserve current value" per
    // field, so a non-positive size would silently leave a dimension unchanged
    // while reporting success. Reject it up front.
    if width <= 0 || height <= 0 {
        bail!("resize requires positive width and height (got {width}x{height})");
    }
    unmaximize(window_id)?;
    let id = window_id_arg(window_id);
    let geometry = format!("0,-1,-1,{width},{height}");
    run_wmctrl(
        &["-i", "-r", id.as_str(), "-e", geometry.as_str()],
        "resize window",
        window_id,
    )?;
    Ok(format!(
        "Resized window to {width}x{height} via X11/EWMH (wmctrl)."
    ))
}

/// EWMH move/resize only take effect on unmaximized windows, so drop the
/// maximized state first (mirrors the GNOME extension backend behaviour).
fn unmaximize(window_id: u64) -> Result<()> {
    let id = window_id_arg(window_id);
    run_wmctrl(
        &[
            "-i",
            "-r",
            id.as_str(),
            "-b",
            "remove,maximized_vert,maximized_horz",
        ],
        "unmaximize window",
        window_id,
    )
}

fn run_wmctrl(args: &[&str], action: &str, window_id: u64) -> Result<()> {
    let output = wmctrl()
        .args(args)
        .output()
        .with_context(|| format!("failed to run wmctrl to {action} 0x{window_id:x}"))?;
    if !output.status.success() {
        bail!(
            "wmctrl failed to {action} 0x{window_id:x}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn window_id_arg(window_id: u64) -> String {
    format!("0x{window_id:08x}")
}

fn wmctrl() -> Command {
    Command::new("wmctrl")
}

fn active_window_id() -> Option<u64> {
    let output = Command::new("xprop")
        .args(["-root", "-notype", "_NET_ACTIVE_WINDOW"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_active_window_id(&String::from_utf8_lossy(&output.stdout))
}

/// Parse the window id out of an `xprop _NET_ACTIVE_WINDOW` line, e.g.
/// `_NET_ACTIVE_WINDOW: window id # 0x7000004`. Returns `None` for `0x0`.
pub(crate) fn parse_active_window_id(xprop_output: &str) -> Option<u64> {
    let after = xprop_output.split("0x").nth(1)?;
    let hex: String = after
        .chars()
        .take_while(|character| character.is_ascii_hexdigit())
        .collect();
    let id = u64::from_str_radix(&hex, 16).ok()?;
    (id != 0).then_some(id)
}

/// Parse `wmctrl -l -p -G -x` output into window records.
///
/// Each line is: `id desktop pid x y w h wm_class client_machine title...`,
/// where `title` is the free-form remainder (may contain spaces).
pub(crate) fn parse_wmctrl_windows(list_output: &str, active_id: Option<u64>) -> Vec<WindowInfo> {
    let mut windows: Vec<WindowInfo> = list_output
        .lines()
        .filter_map(|line| parse_wmctrl_line(line, active_id))
        .collect();
    windows.sort_by_key(|window| window.window_id);
    windows
}

fn parse_wmctrl_line(line: &str, active_id: Option<u64>) -> Option<WindowInfo> {
    let mut rest = line;
    let id_field = next_field(&mut rest)?;
    let desktop_field = next_field(&mut rest)?;
    let pid_field = next_field(&mut rest)?;
    let x_field = next_field(&mut rest)?;
    let y_field = next_field(&mut rest)?;
    let width_field = next_field(&mut rest)?;
    let height_field = next_field(&mut rest)?;
    let class_field = next_field(&mut rest)?;
    let _client_machine = next_field(&mut rest);
    let title = rest.trim();

    let window_id = u64::from_str_radix(id_field.trim_start_matches("0x"), 16).ok()?;
    let desktop = desktop_field.parse::<i32>().ok()?;
    let pid = pid_field.parse::<u32>().ok().filter(|pid| *pid != 0);
    let x = x_field.parse::<i32>().ok()?;
    let y = y_field.parse::<i32>().ok()?;
    let width = width_field.parse::<u32>().ok()?;
    let height = height_field.parse::<u32>().ok()?;

    let (app_id, wm_class) = split_wm_class(class_field);

    Some(WindowInfo {
        window_id,
        title: clean(title),
        app_id,
        wm_class,
        pid,
        bounds: Some(WindowBounds {
            x: Some(x),
            y: Some(y),
            width,
            height,
        }),
        workspace: (desktop >= 0).then_some(desktop),
        focused: active_id == Some(window_id),
        hidden: false,
        client_type: Some("x11".to_string()),
        backend: X11_BACKEND.to_string(),
        terminal: None,
    })
}

/// `wmctrl -x` prints `WM_CLASS` as `instance.Class`. Map `instance` to
/// `app_id` and `Class` to `wm_class`, mirroring the i3 backend.
fn split_wm_class(value: &str) -> (Option<String>, Option<String>) {
    match value.split_once('.') {
        Some((instance, class)) => (clean(instance), clean(class)),
        None => (clean(value), clean(value)),
    }
}

/// Consume the next whitespace-delimited field, advancing `rest` past it.
fn next_field<'a>(rest: &mut &'a str) -> Option<&'a str> {
    *rest = rest.trim_start();
    if rest.is_empty() {
        return None;
    }
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let (field, tail) = rest.split_at(end);
    *rest = tail;
    Some(field)
}

fn clean(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty() && !value.eq_ignore_ascii_case("N/A")).then(|| value.to_string())
}

fn env_nonempty(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// True if `cmd` is an executable found on `$PATH`. Used to gate focus
/// capabilities on `xprop` without spawning it (xprop with no args would block
/// reading a window interactively).
fn command_on_path(cmd: &str) -> bool {
    env::var_os("PATH")
        .is_some_and(|paths| env::split_paths(&paths).any(|dir| dir.join(cmd).is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_active_window_id_from_xprop() {
        assert_eq!(
            parse_active_window_id("_NET_ACTIVE_WINDOW: window id # 0x7000004\n"),
            Some(0x7000004)
        );
        assert_eq!(
            parse_active_window_id("_NET_ACTIVE_WINDOW(WINDOW): window id # 0x03a00017\n"),
            Some(0x03a00017)
        );
        assert_eq!(
            parse_active_window_id("_NET_ACTIVE_WINDOW: window id # 0x0\n"),
            None
        );
    }

    #[test]
    fn parses_wmctrl_windows_as_window_info() {
        // Real `wmctrl -l -p -G -x` lines: id desktop pid x y w h class host title.
        let output = "\
0x03000003  0 3843   0    0    5120 1400 nemo-desktop.Nemo-desktop  rog nemo-desktop
0x03a00017  2 4564   -40  -40  2600 1440 Navigator.firefox     rog Nouvel onglet — Mozilla Firefox
0x05c00007  0 26606  0    72   2560 1368 terminator.Terminator  rog jo@rog: ~
0x07000004 -1 0      10   10   400  300  claude-desktop.claude-desktop  rog Claude
";
        let windows = parse_wmctrl_windows(output, Some(0x07000004));

        assert_eq!(windows.len(), 4);

        let firefox = windows
            .iter()
            .find(|window| window.window_id == 0x03a00017)
            .unwrap();
        assert_eq!(firefox.app_id.as_deref(), Some("Navigator"));
        assert_eq!(firefox.wm_class.as_deref(), Some("firefox"));
        assert_eq!(firefox.pid, Some(4564));
        assert_eq!(
            firefox.title.as_deref(),
            Some("Nouvel onglet — Mozilla Firefox")
        );
        assert_eq!(firefox.workspace, Some(2));
        let bounds = firefox.bounds.as_ref().unwrap();
        assert_eq!(
            (bounds.x, bounds.y, bounds.width, bounds.height),
            (Some(-40), Some(-40), 2600, 1440)
        );
        assert_eq!(firefox.client_type.as_deref(), Some("x11"));
        assert_eq!(firefox.backend, X11_BACKEND);
        assert!(!firefox.focused);

        // Active window is flagged; sticky desktop (-1) has no workspace; pid 0 -> None.
        let claude = windows
            .iter()
            .find(|window| window.window_id == 0x07000004)
            .unwrap();
        assert!(claude.focused);
        assert_eq!(claude.workspace, None);
        assert_eq!(claude.pid, None);
    }

    #[test]
    fn move_coord_avoids_wmctrl_preserve_sentinel() {
        assert_eq!(wmctrl_move_coord(-1), -2);
        assert_eq!(wmctrl_move_coord(0), 0);
        assert_eq!(wmctrl_move_coord(-40), -40);
        assert_eq!(wmctrl_move_coord(1920), 1920);
    }

    #[test]
    fn clean_drops_na_case_insensitively_and_blanks() {
        assert_eq!(clean("N/A"), None);
        assert_eq!(clean("n/a"), None);
        assert_eq!(clean("   "), None);
        assert_eq!(clean(" Firefox "), Some("Firefox".to_string()));
    }

    #[test]
    fn parses_title_with_multiple_spaces_and_empty_title() {
        let output = "0x00000001  0 100 0 0 800 600 term.Term  rog\n";
        let windows = parse_wmctrl_windows(output, None);
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].title, None);
        assert_eq!(windows[0].wm_class.as_deref(), Some("Term"));
    }
}

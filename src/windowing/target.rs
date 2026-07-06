use crate::windowing::registry::{self, WINDOW_PERMISSION_HINT};
use crate::windowing::types::{WindowFocusResult, WindowInfo, WindowTarget};
use anyhow::{bail, Result};
use tokio::time::{sleep, Duration};

const FOCUS_VERIFY_ATTEMPTS: usize = 6;
const FOCUS_VERIFY_DELAY: Duration = Duration::from_millis(50);

pub async fn list_windows() -> Result<Vec<WindowInfo>> {
    registry::list_windows().await
}

pub async fn focused_window() -> Result<Option<WindowInfo>> {
    current_focused_window().await
}

pub async fn focus_window_target(target: &WindowTarget) -> Result<WindowFocusResult> {
    if !target.has_target() {
        bail!("Pass window_id, pid, app_id, wm_class, title, tty, terminal_pid, terminal_command, or terminal_cwd to target a window.");
    }

    let windows = list_windows().await?;
    let requested_window = resolve_window_target(&windows, target)?.clone();
    ensure_backend_can_focus_target(target, &requested_window)?;

    registry::activate_window(&requested_window).await?;

    let focused_window = wait_for_focused_window(&requested_window).await;
    let exact_window_focused = focused_window
        .as_ref()
        .is_some_and(|window| window.window_id == requested_window.window_id);
    let app_focused = focused_window
        .as_ref()
        .is_some_and(|window| same_optional_string(&window.app_id, &requested_window.app_id));

    Ok(WindowFocusResult {
        backend: requested_window.backend.clone(),
        requested_window,
        focused_window,
        exact_window_focused,
        app_focused,
        note: "Computer Use activated the requested window through the available window backend, then verified focus through a fresh window query."
            .to_string(),
    })
}

pub(crate) fn ensure_backend_can_focus_target(
    target: &WindowTarget,
    window: &WindowInfo,
) -> Result<()> {
    if target.requires_exact_focus() && !registry::backend_can_exact_focus(&window.backend) {
        bail!(
            "Exact window targeting requires an exact-focus window backend; {} can list the matched window but cannot activate a specific window safely.",
            window.backend
        );
    }
    Ok(())
}

async fn current_focused_window() -> Result<Option<WindowInfo>> {
    if let Some(window) = registry::focused_window_override() {
        return Ok(Some(window));
    }

    Ok(list_windows()
        .await?
        .into_iter()
        .find(|window| window.focused))
}

async fn wait_for_focused_window(requested_window: &WindowInfo) -> Option<WindowInfo> {
    let mut last_focused_window = None;
    for attempt in 0..FOCUS_VERIFY_ATTEMPTS {
        if let Ok(focused_window) = current_focused_window().await {
            if focused_window
                .as_ref()
                .is_some_and(|window| window.window_id == requested_window.window_id)
            {
                return focused_window;
            }
            if focused_window.is_some() {
                last_focused_window = focused_window;
            }
        }

        if attempt + 1 < FOCUS_VERIFY_ATTEMPTS {
            sleep(FOCUS_VERIFY_DELAY).await;
        }
    }
    last_focused_window
}

pub fn resolve_window_target<'a>(
    windows: &'a [WindowInfo],
    target: &WindowTarget,
) -> Result<&'a WindowInfo> {
    if let Some(window_id) = target.window_id {
        return resolve_window_id_target(windows, target, window_id);
    }

    if target.has_terminal_target() {
        let matches = windows
            .iter()
            .filter(|window| window_matches_terminal_target(window, target))
            .filter(|window| target.pid.is_none_or(|pid| window.pid == Some(pid)))
            .filter(|window| optional_exact_match(&window.app_id, target.app_id.as_deref()))
            .filter(|window| optional_exact_match(&window.wm_class, target.wm_class.as_deref()))
            .filter(|window| optional_title_match(&window.title, target.title.as_deref()))
            .collect::<Vec<_>>();
        return unique_window_match(matches, "terminal target");
    }

    if let Some(pid) = target.pid {
        let matches = windows
            .iter()
            .filter(|window| window.pid == Some(pid))
            .collect::<Vec<_>>();
        return unique_window_match(matches, &format!("pid {pid}"));
    }

    if let Some(app_id) = normalized_target(target.app_id.as_deref()) {
        if let Some(window) = windows.iter().find(|window| {
            window
                .app_id
                .as_deref()
                .is_some_and(|value| value.eq_ignore_ascii_case(&app_id))
        }) {
            return Ok(window);
        }
        bail!("No window matched app_id {app_id}.");
    }

    if let Some(wm_class) = normalized_target(target.wm_class.as_deref()) {
        if let Some(window) = windows.iter().find(|window| {
            window
                .wm_class
                .as_deref()
                .is_some_and(|value| value.eq_ignore_ascii_case(&wm_class))
        }) {
            return Ok(window);
        }
        bail!("No window matched wm_class {wm_class}.");
    }

    if let Some(title) = normalized_target(target.title.as_deref()) {
        let title_lower = title.to_ascii_lowercase();
        if let Some(window) = windows.iter().find(|window| {
            window
                .title
                .as_deref()
                .is_some_and(|value| value.to_ascii_lowercase().contains(&title_lower))
        }) {
            return Ok(window);
        }
        bail!("No window title contained {title}.");
    }

    bail!("Pass window_id, pid, app_id, wm_class, title, tty, terminal_pid, terminal_command, or terminal_cwd to target a window.");
}

fn resolve_window_id_target<'a>(
    windows: &'a [WindowInfo],
    target: &WindowTarget,
    window_id: u64,
) -> Result<&'a WindowInfo> {
    if let Some(window) = windows.iter().find(|window| window.window_id == window_id) {
        return Ok(window);
    }

    let matches = windows
        .iter()
        .filter(|window| window_id_matches_json_number(window.window_id, window_id))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [window] => Ok(*window),
        [] => Err(anyhow::anyhow!("No window matched window_id {window_id}.")),
        windows => resolve_rounded_window_id_matches(windows, target, window_id),
    }
}

fn resolve_rounded_window_id_matches<'a>(
    windows: &[&'a WindowInfo],
    target: &WindowTarget,
    window_id: u64,
) -> Result<&'a WindowInfo> {
    let ids = windows
        .iter()
        .map(|window| window.window_id.to_string())
        .collect::<Vec<_>>()
        .join(", ");

    if has_window_id_disambiguator(target) {
        let matches = windows
            .iter()
            .copied()
            .filter(|window| window_id_disambiguators_match(window, target))
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [window] => return Ok(*window),
            [] => bail!(
                "window_id {window_id} matched multiple windows after JSON number rounding ({ids}), but none matched the provided title, pid, app_id, or wm_class disambiguators."
            ),
            _ => {}
        }
    }

    bail!(
        "window_id {window_id} matched multiple windows after JSON number rounding ({ids}); add title, pid, app_id, or wm_class to disambiguate."
    );
}

fn has_window_id_disambiguator(target: &WindowTarget) -> bool {
    target.pid.is_some()
        || normalized_target(target.app_id.as_deref()).is_some()
        || normalized_target(target.wm_class.as_deref()).is_some()
        || normalized_target(target.title.as_deref()).is_some()
}

fn window_id_disambiguators_match(window: &WindowInfo, target: &WindowTarget) -> bool {
    target.pid.is_none_or(|pid| window.pid == Some(pid))
        && optional_exact_match(&window.app_id, target.app_id.as_deref())
        && optional_exact_match(&window.wm_class, target.wm_class.as_deref())
        && optional_title_match(&window.title, target.title.as_deref())
}

fn window_id_matches_json_number(actual: u64, requested: u64) -> bool {
    const JS_SAFE_INTEGER_MAX: u64 = (1_u64 << 53) - 1;
    (actual > JS_SAFE_INTEGER_MAX || requested > JS_SAFE_INTEGER_MAX)
        && (actual as f64) == (requested as f64)
}

fn unique_window_match<'a>(
    matches: Vec<&'a WindowInfo>,
    description: &str,
) -> Result<&'a WindowInfo> {
    match matches.as_slice() {
        [window] => Ok(*window),
        [] => bail!("No window matched {description}."),
        windows => {
            let ids = windows
                .iter()
                .map(|window| window.window_id.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "{description} matched multiple windows ({ids}); add window_id, tty, title, or terminal_command to disambiguate."
            );
        }
    }
}

fn window_matches_terminal_target(window: &WindowInfo, target: &WindowTarget) -> bool {
    let Some(terminal) = &window.terminal else {
        return false;
    };

    if let Some(tty) = normalized_target(target.tty.as_deref()) {
        if !tty_matches(&terminal.tty, &tty) {
            return false;
        }
    }

    if let Some(pid) = target.terminal_pid {
        let active_pid = terminal.active_process.as_ref().map(|process| process.pid);
        if active_pid != Some(pid) && terminal.root_process.pid != pid {
            return false;
        }
    }

    if let Some(command) = normalized_target(target.terminal_command.as_deref()) {
        let command = command.to_ascii_lowercase();
        let active_matches = terminal
            .active_process
            .as_ref()
            .is_some_and(|process| terminal_process_matches_command(process, &command));
        if !active_matches && !terminal_process_matches_command(&terminal.root_process, &command) {
            return false;
        }
    }

    if let Some(cwd) = normalized_target(target.terminal_cwd.as_deref()) {
        let active_matches = terminal
            .active_process
            .as_ref()
            .is_some_and(|process| terminal_process_matches_cwd(process, &cwd));
        if !active_matches && !terminal_process_matches_cwd(&terminal.root_process, &cwd) {
            return false;
        }
    }

    true
}

fn terminal_process_matches_command(
    process: &crate::terminal::TerminalProcess,
    command_lower: &str,
) -> bool {
    process
        .command_name
        .to_ascii_lowercase()
        .contains(command_lower)
        || process
            .command_line
            .to_ascii_lowercase()
            .contains(command_lower)
}

fn terminal_process_matches_cwd(process: &crate::terminal::TerminalProcess, cwd: &str) -> bool {
    let requested = cwd.trim_end_matches('/');
    process.cwd.as_deref().is_some_and(|value| {
        let actual = value.trim_end_matches('/');
        actual == requested
            || (!requested.starts_with('/')
                && actual
                    .strip_suffix(requested)
                    .is_some_and(|prefix| prefix.ends_with('/')))
    })
}

fn tty_matches(actual: &str, requested: &str) -> bool {
    actual == requested
        || actual
            .strip_prefix("/dev/")
            .is_some_and(|value| value == requested)
        || actual
            .strip_prefix("/dev/pts/")
            .is_some_and(|value| value == requested)
}

fn optional_exact_match(actual: &Option<String>, requested: Option<&str>) -> bool {
    normalized_target(requested).is_none_or(|requested| {
        actual
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case(&requested))
    })
}

fn optional_title_match(actual: &Option<String>, requested: Option<&str>) -> bool {
    normalized_target(requested).is_none_or(|requested| {
        let requested = requested.to_ascii_lowercase();
        actual
            .as_deref()
            .is_some_and(|value| value.to_ascii_lowercase().contains(&requested))
    })
}

pub fn window_permission_hint(error: &str) -> Option<String> {
    let lower = error.to_ascii_lowercase();
    if lower.contains("accessdenied")
        || lower.contains("access denied")
        || lower.contains("not allowed")
        || lower.contains("operation not permitted")
        || lower.contains("failed to connect to session bus")
    {
        Some(WINDOW_PERMISSION_HINT.to_string())
    } else {
        None
    }
}

fn normalized_target(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn same_optional_string(left: &Option<String>, right: &Option<String>) -> bool {
    match (left.as_deref(), right.as_deref()) {
        (Some(left), Some(right)) => left.eq_ignore_ascii_case(right),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::{TerminalProcess, TerminalWindowContext};
    use crate::windowing::types::WindowBounds;

    fn window(window_id: u64) -> WindowInfo {
        WindowInfo {
            window_id,
            title: Some("Editor".to_string()),
            app_id: Some("dev.example.Editor".to_string()),
            wm_class: Some("Editor".to_string()),
            pid: Some(1000),
            bounds: Some(WindowBounds {
                x: Some(0),
                y: Some(0),
                width: 800,
                height: 600,
            }),
            workspace: Some(1),
            focused: false,
            hidden: false,
            client_type: Some("wayland".to_string()),
            backend: "test".to_string(),
            terminal: None,
        }
    }

    fn terminal_process(pid: u32, command: &str, cwd: Option<&str>) -> TerminalProcess {
        TerminalProcess {
            pid,
            command_name: command.to_string(),
            command_line: format!("{command} --flag"),
            cwd: cwd.map(ToOwned::to_owned),
        }
    }

    fn terminal_context(
        tty: &str,
        root: TerminalProcess,
        active: Option<TerminalProcess>,
    ) -> TerminalWindowContext {
        TerminalWindowContext {
            tty: tty.to_string(),
            root_process: root,
            active_process: active,
            process_count: 1,
            confidence: "high".to_string(),
            match_reason: "test".to_string(),
        }
    }

    #[test]
    fn resolve_window_id_matches_exact() {
        let windows = vec![window(10), window(20)];
        let target = WindowTarget {
            window_id: Some(20),
            ..Default::default()
        };
        let resolved = resolve_window_target(&windows, &target).expect("exact id resolves");
        assert_eq!(resolved.window_id, 20);
    }

    #[test]
    fn resolve_window_id_missing_errors() {
        let windows = vec![window(10)];
        let target = WindowTarget {
            window_id: Some(999),
            ..Default::default()
        };
        let error = resolve_window_target(&windows, &target)
            .unwrap_err()
            .to_string();
        assert!(error.contains("No window matched window_id 999"), "{error}");
    }

    #[test]
    fn resolve_by_pid_unique() {
        let mut other = window(11);
        other.pid = Some(2222);
        let windows = vec![window(10), other];
        let target = WindowTarget {
            pid: Some(2222),
            ..Default::default()
        };
        let resolved = resolve_window_target(&windows, &target).expect("pid resolves");
        assert_eq!(resolved.window_id, 11);
    }

    #[test]
    fn resolve_by_pid_ambiguous_lists_ids() {
        let mut a = window(10);
        a.pid = Some(4242);
        let mut b = window(11);
        b.pid = Some(4242);
        let windows = vec![a, b];
        let target = WindowTarget {
            pid: Some(4242),
            ..Default::default()
        };
        let error = resolve_window_target(&windows, &target)
            .unwrap_err()
            .to_string();
        assert!(error.contains("matched multiple windows"), "{error}");
        assert!(error.contains("10") && error.contains("11"), "{error}");
    }

    #[test]
    fn resolve_by_app_id_is_case_insensitive() {
        let windows = vec![window(10)];
        let target = WindowTarget {
            app_id: Some("dev.example.editor".to_string()),
            ..Default::default()
        };
        let resolved = resolve_window_target(&windows, &target).expect("app_id resolves");
        assert_eq!(resolved.window_id, 10);
    }

    #[test]
    fn resolve_by_wm_class_no_match_errors() {
        let windows = vec![window(10)];
        let target = WindowTarget {
            wm_class: Some("Firefox".to_string()),
            ..Default::default()
        };
        let error = resolve_window_target(&windows, &target)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("No window matched wm_class Firefox"),
            "{error}"
        );
    }

    #[test]
    fn resolve_by_title_matches_substring_case_insensitive() {
        let mut w = window(10);
        w.title = Some("My Project — VS Code".to_string());
        let windows = vec![w];
        let target = WindowTarget {
            title: Some("vs code".to_string()),
            ..Default::default()
        };
        let resolved = resolve_window_target(&windows, &target).expect("title substring resolves");
        assert_eq!(resolved.window_id, 10);
    }

    #[test]
    fn resolve_without_any_target_errors() {
        let windows = vec![window(10)];
        let target = WindowTarget::default();
        assert!(resolve_window_target(&windows, &target).is_err());
    }

    #[test]
    fn blank_string_targets_are_treated_as_absent() {
        // A whitespace-only app_id must not match; with no other criteria it errors.
        let windows = vec![window(10)];
        let target = WindowTarget {
            app_id: Some("   ".to_string()),
            ..Default::default()
        };
        assert!(resolve_window_target(&windows, &target).is_err());
    }

    #[test]
    fn window_id_rounding_needs_disambiguator() {
        // Above 2^53, distinct u64 window ids collapse to the same f64 (JSON
        // number). `base`, `base + 1`, and `base + 2` all cast to `base` as f64.
        // A request for `base + 1` has no exact u64 window, so it falls through
        // to the rounding path and matches both `base` and `base + 2`. Without a
        // disambiguator this must fail loudly rather than pick one arbitrarily.
        let base = 1_u64 << 54;
        let mut a = window(base);
        a.pid = Some(100);
        let mut b = window(base + 2);
        b.pid = Some(200);
        let windows = vec![a, b];

        let ambiguous = WindowTarget {
            window_id: Some(base + 1),
            ..Default::default()
        };
        let error = resolve_window_target(&windows, &ambiguous)
            .unwrap_err()
            .to_string();
        assert!(error.contains("JSON number rounding"), "{error}");

        // Adding a pid disambiguator resolves to the intended window.
        let disambiguated = WindowTarget {
            window_id: Some(base + 1),
            pid: Some(200),
            ..Default::default()
        };
        let resolved =
            resolve_window_target(&windows, &disambiguated).expect("pid disambiguates rounded id");
        assert_eq!(resolved.window_id, base + 2);
    }

    #[test]
    fn terminal_target_matches_by_tty() {
        let mut w = window(10);
        w.terminal = Some(terminal_context(
            "/dev/pts/3",
            terminal_process(500, "bash", Some("/home/user")),
            None,
        ));
        let windows = vec![w];
        let target = WindowTarget {
            tty: Some("pts/3".to_string()),
            ..Default::default()
        };
        let resolved = resolve_window_target(&windows, &target).expect("tty resolves");
        assert_eq!(resolved.window_id, 10);
    }

    #[test]
    fn terminal_target_matches_active_process_command() {
        let mut w = window(10);
        w.terminal = Some(terminal_context(
            "/dev/pts/1",
            terminal_process(500, "bash", Some("/home/user")),
            Some(terminal_process(777, "vim", Some("/home/user/project"))),
        ));
        let windows = vec![w];
        let target = WindowTarget {
            terminal_command: Some("vim".to_string()),
            ..Default::default()
        };
        let resolved = resolve_window_target(&windows, &target).expect("command resolves");
        assert_eq!(resolved.window_id, 10);
    }

    #[test]
    fn terminal_target_non_terminal_window_is_skipped() {
        // A plain window (terminal: None) must never match a terminal target.
        let windows = vec![window(10)];
        let target = WindowTarget {
            terminal_pid: Some(500),
            ..Default::default()
        };
        assert!(resolve_window_target(&windows, &target).is_err());
    }

    #[test]
    fn tty_matches_accepts_bare_and_dev_forms() {
        assert!(tty_matches("/dev/pts/3", "pts/3"));
        assert!(tty_matches("/dev/tty1", "tty1"));
        assert!(tty_matches("/dev/pts/3", "3"));
        assert!(tty_matches("pts/3", "pts/3"));
        assert!(!tty_matches("/dev/pts/3", "pts/4"));
    }

    #[test]
    fn terminal_cwd_matches_exact_and_trailing_component() {
        let process = terminal_process(1, "bash", Some("/home/user/project"));
        assert!(terminal_process_matches_cwd(&process, "/home/user/project"));
        assert!(terminal_process_matches_cwd(
            &process,
            "/home/user/project/"
        ));
        // A relative suffix on a path boundary matches.
        assert!(terminal_process_matches_cwd(&process, "project"));
        // A non-boundary suffix must not match.
        assert!(!terminal_process_matches_cwd(&process, "roject"));
        assert!(!terminal_process_matches_cwd(&process, "/other"));
    }

    #[test]
    fn optional_matches_treat_absent_request_as_wildcard() {
        assert!(optional_exact_match(&Some("Editor".to_string()), None));
        assert!(optional_exact_match(
            &Some("Editor".to_string()),
            Some("editor")
        ));
        assert!(!optional_exact_match(&None, Some("editor")));
        assert!(optional_title_match(
            &Some("My Editor".to_string()),
            Some("editor")
        ));
        assert!(!optional_title_match(&None, Some("editor")));
    }

    #[test]
    fn window_permission_hint_recognizes_denied_errors() {
        assert!(window_permission_hint("org.freedesktop.DBus.Error.AccessDenied").is_some());
        assert!(window_permission_hint("Operation not permitted").is_some());
        assert!(window_permission_hint("Failed to connect to session bus").is_some());
        assert!(window_permission_hint("some unrelated timeout").is_none());
    }

    #[test]
    fn ensure_backend_can_focus_rejects_non_exact_backend_for_exact_target() {
        // A window_id target requires exact focus; a backend that cannot exact-focus
        // must be rejected. gnome-shell-introspect is a list-only backend.
        let mut w = window(10);
        w.backend = "gnome-shell-introspect".to_string();
        let target = WindowTarget {
            window_id: Some(10),
            ..Default::default()
        };
        assert!(ensure_backend_can_focus_target(&target, &w).is_err());
    }
}

---
name: computer-use-linux
description: "Linux desktop observation and control via native Pi tools or the computer-use-linux MCP server: accessibility trees, screenshots, window targeting, and input synthesis (click, type, scroll)."
author: agent-sh
license: MIT
platforms: [linux]
compatibility: "Native Pi tools require Pi 0.84.4+ and Node.js 22.19+; the standalone CLI/MCP server supports Node.js 18+."
---

# computer-use-linux

Use `computer-use-linux` when an agent needs to observe or operate a local Linux desktop: inspect the accessibility tree, list/focus windows, take screenshots, click, scroll, type, press keys, or invoke AT-SPI actions.

## When to Use

Use this skill when:

- The user wants the agent to control a Linux GUI app.
- You need desktop state from AT-SPI, screenshots, or compositor window metadata.
- You are configuring the `computer-use-linux` MCP server for your agent.
- A desktop action needs target-aware input instead of blind shell commands.

Do not use this for remote browsers, websites, or headless automation when a browser-specific tool is available. Do not assume desktop actions are safe just because the MCP connection works.

## Install

Pi users need only the package:

```bash
pi install npm:@agent-sh/computer-use-linux
```

Preferred install:

```bash
npm install -g @agent-sh/computer-use-linux
computer-use-linux doctor | jq .readiness
```

Rust users can install from crates.io:

```bash
cargo install computer-use-linux
computer-use-linux doctor | jq .readiness
```

If `doctor` reports missing input or accessibility support, run:

```bash
computer-use-linux setup
computer-use-linux setup-window-targeting
computer-use-linux doctor | jq .readiness
```

If `doctor` selects ydotool as the input backend, also enable its per-user daemon with `systemctl --user enable --now ydotoold`. Direct uinput, X11 xdotool, and RemoteDesktop portal input do not require `ydotoold`.

On GNOME Wayland, log out and back in after `setup-window-targeting` if the GNOME Shell extension was newly installed.

For MCP hosts with `COMPUTER_USE_LINUX_NOTIFY_ON_COMPLETE=1`, call the optional
`complete_interaction` tool once after finishing desktop interaction. A skipped
cue is not a task failure. This notification does not guarantee exclusive
desktop ownership or that other clients have stopped sending input.
This applies only to directly spawned MCP hosts, not the native Pi extension.

`setup_accessibility` verifies the saved GNOME `toolkit-accessibility` key
separately from runtime AT-SPI. Inspect its warning and readback before assuming
new apps can expose trees. Other accessibility tools may change the key later;
setup does not hold it enabled continuously.

### Optional foreground accessibility guard

Skip unless: the user explicitly wants GNOME's saved `toolkit-accessibility`
setting kept enabled while desktop automation runs.

Run `computer-use-linux guard-accessibility` in a foreground terminal. It
registers a passive AT-SPI window-activation listener and watches/reasserts the
saved key with readback. The setting affects all apps for the current user.
`mcp`, setup, and `get_app_state` never start this guard automatically.
Stop with Ctrl-C or SIGTERM before intentionally disabling accessibility.
Stopping ends writes and removes its listener without disabling other clients
or restoring a previous saved value. Apps launched during a reset/reassertion
race may still need restarting; do not claim a complete GNOME toggle fix.

## Configure Your Agent

The `computer-use-linux` binary is an MCP server. Configure it as a stdio MCP server in your agent of choice:

```json
{
  "command": "computer-use-linux",
  "args": ["mcp"]
}
```

If the binary is not on `PATH`, use the absolute path (typically `~/.local/bin/computer-use-linux` or the npm global bin directory).

### Host-specific guides

- [Hermes setup](references/hermes-setup.md)
- [Pi coding agent setup](references/pi-setup.md)

## Procedure

1. In Pi, call `computer_use_linux_tools` with the exact tools or capability you need. Enabled tools use the `computer_use_linux_<name>` prefix, appear starting on the next model turn, and remain active for the session.
2. Begin every desktop-control turn with `get_app_state`; use `include_screenshot: false` when the accessibility tree is sufficient. Its compact readiness block identifies missing setup.
3. Use `doctor` only when you need the full diagnostic report.
4. If `can_build_accessibility_tree` is false, run `setup_accessibility` and restart the target app.
5. If `can_query_windows` is false on GNOME Wayland, run `setup_window_targeting` and ask the user to log out and back in if setup says the shell extension needs a reload.
6. Before targeted input, call `list_windows` or `focused_window` and verify the intended window by title, app id, pid, or wm class.
7. Prefer semantic targeting from `get_app_state`: use element indices or role/name/text/states selectors.
8. Use coordinates only when the UI surface has no useful accessibility tree.
9. For text input, prefer `type_text` with a target selector (`window_id`, `pid`, `app_id`, `wm_class`, `title`, `tty`, `terminal_pid`, `terminal_command`, or `terminal_cwd`) rather than relying on current focus.
10. After mutating actions, re-check state with `get_app_state`, `focused_window`, or an app-specific readback.

Plain left element/index/selector `click` prefers native AT-SPI `click`,
`press`, or `toggle` over toolkit bounds, avoiding coordinate
conversion when available. This preference does not replace a coordinate click
with an arbitrary action name. Explicit `x`/`y`, right clicks, and double/multiple
clicks retain pointer semantics.
Use `perform_action` explicitly for entry `activate` or slider `jump`; `click`
never substitutes those actions, including when bounds are unavailable.

### Screenshot-relative coordinates

Skip unless: a coordinate `click` or `scroll` uses `relative: true`.

Select a target window and use its clipped screenshot crop origin. Divide
preview `x`/`y` by screenshot `scale` first. Widget-local and raw GDK surface
coordinates are not interchangeable with that origin; missing window targets
are rejected. For calibration, use the repository's
`examples/coordinate_probe.py`: select the green square from the screenshot
and require a delivered-event `hit: true`. Do not pass widget-local `(85, 85)`
directly to a window-relative click.

## Pitfalls

- Already-running GTK, Qt, and Electron apps may need a restart after AT-SPI is enabled.
- GNOME may show a portal prompt on the first screenshot or `get_app_state` call with screenshots enabled.
- Desktop input is stateful. Avoid concurrent tool calls against this MCP server.
- Pi serializes the native Computer Use tools and keeps one process for the session. If that process exits, do not replay an ambiguous mutating call; obtain a fresh `get_app_state` before another element-based action.
- `click`, `drag`, `press_key`, `type_text`, `perform_action`, and `set_value` can change real application state.
- When ydotool is selected, `ydotoold` should run as a per-user service with its socket under `/run/user/$UID`, not as a system-wide service.
- The optional ydotool backend requires version 1.0.3 or newer; `doctor` rejects older or semantically incompatible CLIs even when `ydotoold` is running.
- On COSMIC, the standard npm, Cargo, and install-script paths install the `computer-use-linux-cosmic` helper automatically. Manual binary installs must copy both binaries.

## Verification

Run:

```bash
computer-use-linux doctor | jq .readiness
```

Ready output should have:

- `can_register_mcp_tools: true`
- `can_build_accessibility_tree: true`
- `can_query_windows: true`
- `can_send_development_input: true`
- `blockers: []`

Then test with your agent by calling the `doctor` tool or asking the agent to list desktop windows.

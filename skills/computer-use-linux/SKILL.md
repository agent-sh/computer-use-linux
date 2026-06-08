---
name: computer-use-linux
description: "Use when Hermes needs Linux desktop observation or control through the computer-use-linux MCP server."
author: agent-sh
license: MIT
platforms: [linux]
---

# computer-use-linux

Use `computer-use-linux` when Hermes needs to observe or operate a local Linux desktop through MCP: inspect the accessibility tree, list/focus windows, take screenshots, click, scroll, type, press keys, or invoke AT-SPI actions.

## When to Use

Use this skill when:
- The user wants Hermes to control a Linux GUI app.
- You need desktop state from AT-SPI, screenshots, or compositor window metadata.
- You are configuring the `computer-use-linux` MCP server for Hermes.
- A desktop action needs target-aware input instead of blind shell commands.

Do not use this for remote browsers, websites, or headless automation when a browser-specific tool is available. Do not assume desktop actions are safe just because the MCP connection works.

## Install

Preferred install for Hermes users:

```bash
npm install -g @agent-sh/computer-use-linux
computer-use-linux doctor | jq .readiness
```

Rust users can install the same server from crates.io:

```bash
cargo install computer-use-linux
computer-use-linux doctor | jq .readiness
```

If `doctor` reports missing input or accessibility support, run:

```bash
computer-use-linux setup
systemctl --user enable --now ydotoold
computer-use-linux setup-window-targeting
computer-use-linux doctor | jq .readiness
```

On GNOME Wayland, log out and back in after `setup-window-targeting` if the GNOME Shell extension was newly installed.

Enable hybrid mode for Electron/Qt apps with broken trees:

```bash
export COMPUTER_USE_LINUX_HYBRID=1
```

## Configure Hermes

Add the server with the Hermes MCP CLI:

```bash
hermes mcp add computer-use-linux --command computer-use-linux --args mcp
hermes mcp test computer-use-linux
hermes mcp configure computer-use-linux
```

`configure` opens Hermes' tool-selection UI for this MCP server.

The generated config should look like this:

```yaml
mcp_servers:
  computer-use-linux:
    command: computer-use-linux
    args: ["mcp"]
    timeout: 120
    connect_timeout: 30
    env:
      COMPUTER_USE_LINUX_HYBRID: "1"
```

If the binary is not on `PATH`, pass the absolute path to `--command`.

Hermes registers tools using the `mcp_<server>_<tool>` pattern. With this config, tool names are prefixed as `mcp_computer_use_linux_`, for example:

| MCP tool | Hermes tool name |
| --- | --- |
| `doctor` | `mcp_computer_use_linux_doctor` |
| `get_app_state` | `mcp_computer_use_linux_get_app_state` |
| `find_element` | `mcp_computer_use_linux_find_element` |
| `hybrid_strategy` | `mcp_computer_use_linux_hybrid_strategy` |
| `list_windows` | `mcp_computer_use_linux_list_windows` |
| `click` | `mcp_computer_use_linux_click` |
| `type_text` | `mcp_computer_use_linux_type_text` |
| `screenshot_debug` | `mcp_computer_use_linux_screenshot_debug` |
| `get_clipboard` | `mcp_computer_use_linux_get_clipboard` |
| `set_clipboard` | `mcp_computer_use_linux_set_clipboard` |
| `start_recording` | `mcp_computer_use_linux_start_recording` |
| `stop_recording` | `mcp_computer_use_linux_stop_recording` |

Restart Hermes after changing MCP config.

## Accessibility-First + Hybrid Decision Tree

Follow this order on every desktop-control turn:

1. **`doctor`** — confirm `can_build_accessibility_tree`, `can_query_windows`, and `can_send_development_input`.
2. **`get_app_state`** — bounded screenshot + compacted AT-SPI tree. Cache `@eN` refs from `element_index`.
3. **`hybrid_strategy`** or check `find_element` output — when actionable nodes are sparse, enable hybrid fallback.
4. **Target windows** — `list_windows` / `focused_window` / `activate_window` before keyboard input.
5. **Prefer semantic refs** — `find_element "save button"` → `click` with `element_index`, or role/name/text selectors.
6. **Hybrid fallback** — when AT-SPI is empty or stale (`STALE_REF`), use `screenshot` or `screenshot_debug` with `highlight_refs`, then coordinate `click` using `coordinate_width` / `coordinate_height` / `scale`.
7. **Verify** — re-call `get_app_state` after mutating actions.

### Input fallback chain (automatic)

1. AT-SPI `element_index` or semantic selector
2. AT-SPI primary action (`perform_action`)
3. uinput absolute pointer (exact screenshot pixels)
4. Wayland remote desktop portal
5. ydotool relative input

Explain which strategy succeeded in your reply so the user can debug permission or compositor issues.

## Procedure

1. Start every desktop-control session with `doctor`.
2. If `can_build_accessibility_tree` is false, run `setup` and restart the target app.
3. If `can_query_windows` is false on GNOME Wayland, run `setup-window-targeting` and ask the user to log out and back in if setup says the shell extension needs a reload.
4. Before targeted input, call `list_windows` or `focused_window` and verify the intended window by title, app id, pid, or wm class.
5. Prefer semantic targeting: `find_element` for natural language, then `element_index` or role/name/text/states selectors.
6. Use coordinates only when the UI surface has no useful accessibility tree (hybrid mode).
7. For text input, prefer `type_text` with a target selector rather than relying on current focus.
8. Use `get_clipboard` / `set_clipboard` for paste-heavy workflows on Wayland.
9. Use `start_recording` / `stop_recording` to capture repeatable workflows; export the skill skeleton for Hermes.
10. After mutating actions, re-check state with `get_app_state`, `focused_window`, or an app-specific readback.

## Pitfalls

- Already-running GTK, Qt, and Electron apps may need a restart after AT-SPI is enabled.
- GNOME may show a portal prompt on the first screenshot or `get_app_state` call with screenshots enabled.
- Desktop input is stateful. Avoid concurrent tool calls against this MCP server.
- `click`, `drag`, `press_key`, `type_text`, `perform_action`, and `set_value` can change real application state.
- `ydotoold` should run as a per-user service with its socket under `/run/user/$UID`, not as a system-wide service.
- On COSMIC, the standard npm, Cargo, and install-script paths install the `computer-use-linux-cosmic` helper automatically. Manual binary installs must copy both binaries.
- Sway/wlroots users need `swaymsg` on PATH; `doctor` reports the active window backend.
- OCR (`screenshot_debug` with `ocr=true`) requires `tesseract-ocr` installed.

## Verification

Run:

```bash
computer-use-linux doctor | jq .readiness
hermes chat --toolsets mcp-computer-use-linux -q "List the current desktop windows."
```

Ready output should have:

- `can_register_mcp_tools: true`
- `can_build_accessibility_tree: true`
- `can_query_windows: true`
- `can_send_development_input: true`
- `blockers: []`

If Hermes does not expose the tools, check startup logs for MCP discovery errors and confirm the server name in `config.yaml` is exactly `computer-use-linux`.
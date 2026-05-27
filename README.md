<div align="center">
  <h1>computer-use-linux</h1>
  <p><strong>Control a real Linux desktop from any MCP host.</strong></p>
  <p>
    <a href="https://github.com/agent-sh/computer-use-linux/actions/workflows/ci.yml"><img src="https://github.com/agent-sh/computer-use-linux/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
    <a href="https://crates.io/crates/computer-use-linux"><img src="https://img.shields.io/crates/v/computer-use-linux.svg" alt="crates.io"></a>
    <a href="https://www.npmjs.com/package/@agent-sh/computer-use-linux"><img src="https://img.shields.io/npm/v/@agent-sh/computer-use-linux.svg" alt="npm"></a>
    <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-yellow.svg" alt="License: MIT"></a>
  </p>
</div>

`computer-use-linux` reads accessibility trees, takes screenshots, and drives clicks, scrolls, and keystrokes across GNOME, KDE/KWin, Hyprland, i3, and COSMIC — Wayland-first, X11 best-effort.

```bash
npm install -g @agent-sh/computer-use-linux
computer-use-linux doctor | jq .readiness
```

The Rust crate is published as [`computer-use-linux`](https://crates.io/crates/computer-use-linux) and the npm wrapper as [`@agent-sh/computer-use-linux`](https://www.npmjs.com/package/@agent-sh/computer-use-linux). Prebuilt binaries ship with the [latest release](https://github.com/agent-sh/computer-use-linux/releases/latest).

## What this is

`computer-use-linux` is a Rust MCP server and CLI for Linux desktop control. The crate ships the main `computer-use-linux` binary plus a small `computer-use-linux-cosmic` helper used only for COSMIC Wayland window management. Any MCP host — Codex Desktop's Linux build, Claude Desktop, [Hermes Agent](https://github.com/NousResearch/hermes-agent), or your own client — can spawn it and gain full control of the local Linux desktop: read accessibility trees, list and focus windows, take screenshots, click, drag, scroll, type, and invoke semantic accessibility actions.

Most computer-use MCP servers are macOS-only (they lean on AppKit, AXUIElement, CGEvent). The few that target Linux either drive `xdotool` against an X11 root window or shell out to OCR over screenshots. Four things set this one apart:

- **Wayland actually works.** Pointer actions can use the `org.freedesktop.portal.RemoteDesktop` interface on Wayland, with `ydotool` / `ydotoold` (uinput) as the deterministic fallback and keyboard/text path. Screenshots use the GNOME Shell DBus screenshot method when present and `org.freedesktop.portal.Screenshot` otherwise.
- **Window targeting is compositor-aware.** The window registry tries GNOME Shell extension, GNOME Shell Introspect, COSMIC Wayland helper, KWin DBus scripting, Hyprland `hyprctl`, and i3 IPC in order, then reports exactly which backend won or why each backend failed.
- **Semantic selectors, not pixel coordinates.** Tools like `click`, `perform_action`, and `set_value` accept `role` / `name` / `text` / `states` selectors backed by AT-SPI. Pixel coordinates remain available as a fallback for rendering-only surfaces (canvas, games, X clients without ATK).
- **One JSON readiness report.** `computer-use-linux doctor` returns a structured document covering platform, portals, AT-SPI, windowing, input, and a `readiness` summary with explicit blockers and a recommended next step. MCP hosts can render or surface that to the user without parsing prose.

The crate was extracted from [`codex-desktop-linux`](https://github.com/avifenesh/codex-desktop-linux) (the Linux distribution of Codex Desktop), which still bundles this binary as a built-in plugin. This standalone repo is the upstream.

## Features

MCP tools exposed by the server:

**Diagnostics**
- `doctor` — single-shot JSON readiness report (platform, portals, accessibility, windowing, input, readiness summary, and a capability map of available backends)
- `setup_accessibility` — enables GNOME's `org.gnome.desktop.interface toolkit-accessibility` setting so toolkit apps expose AT-SPI trees
- `setup_window_targeting` — installs and enables the bundled GNOME Shell extension when `org.gnome.Shell.Introspect` is locked down

**Discovery**
- `list_apps` — running desktop apps visible to the AT-SPI registry
- `list_windows` — compositor windows with title, app id, wm_class, focus state, client type (Wayland/X11), and bounds
- `focused_window` — the window currently holding keyboard focus
- `get_app_state` — combined screenshot + accessibility tree for a chosen app, with element indices that the input tools accept
- `screenshot` — capture the screen as a PNG; can target a window, which is raised to the front and cropped to just that window

**Input**
- `click` — by element index, semantic selector, or pixel coordinates
- `drag` — pixel-coordinate drag (start / end)
- `scroll` — page-based scroll on an element or at a pixel location
- `press_key` — keys / chords; can focus a window or terminal first
- `type_text` — literal text input, optionally targeted at a window or terminal

**Semantic actions**
- `perform_action` — invoke any AT-SPI action exposed by an element (`Press`, `Activate`, `Toggle`, …); defaults to the primary action
- `set_value` — write to a settable accessibility element (text fields, sliders, spinners)

**Navigation**
- `activate_window` — focus a window by `window_id`, `pid`, `app_id`, `wm_class`, `title`, or terminal selectors

### MCP safety contract

`computer-use-linux` is not a read-only data source. It can observe the local desktop and, when a mutating tool is called, can change real application state. The `tools/list` response includes MCP `ToolAnnotations` so hosts can surface this distinction before invocation:

| Class | Tools | Contract |
| --- | --- | --- |
| Read-only observation | `doctor`, `list_apps`, `list_windows`, `focused_window`, `get_app_state` | `readOnlyHint=true`; may reveal app, window, accessibility, and screenshot contents. `get_app_state` may trigger the desktop screenshot portal prompt. |
| Local setup mutators | `setup_accessibility`, `setup_window_targeting` | `readOnlyHint=false`, `destructiveHint=false`, `idempotentHint=true`; modifies user desktop configuration by enabling accessibility or installing/enabling the GNOME window-targeting extension. |
| UI state mutators | `activate_window`, `scroll`, `screenshot` | `readOnlyHint=false`, `destructiveHint=false`; changes focus or scroll position in the live desktop, or raises a window to capture it. |
| Desktop action mutators | `click`, `drag`, `press_key`, `type_text`, `perform_action`, `set_value` | `readOnlyHint=false`, `destructiveHint=true`, `openWorldHint=true`; can trigger arbitrary actions in whatever local application is targeted. |

Annotations are safety hints, not an authorization system. MCP hosts should still ask the user before calls that could submit, delete, send, purchase, overwrite, or otherwise commit state.

The binary also exposes the same capabilities from the CLI for scripting and debugging:

```
computer-use-linux mcp                                  # stdio MCP server
computer-use-linux doctor                               # JSON readiness report
computer-use-linux setup                                # enable AT-SPI
computer-use-linux setup-window-targeting               # install GNOME Shell extension
computer-use-linux apps
computer-use-linux state [APP_NAME]
computer-use-linux screenshot                           # JSON screenshot summary
computer-use-linux windows
```

## Support matrix

Validated manually on Ubuntu 25.10 (GNOME Shell 50.1, Wayland). Other compositor backends are implemented and covered by parser / contract tests, but real desktop behavior still depends on each session exposing its expected control API.

| Desktop/session | Window backend | Notes |
| --- | --- | --- |
| GNOME Wayland | GNOME Shell extension first, `org.gnome.Shell.Introspect` fallback | Full target. The extension provides exact window activation when GNOME blocks native introspection; Introspect can list windows and focus apps by `app_id` when allowed. |
| GNOME X11 | `org.gnome.Shell.Introspect` when allowed | AT-SPI and `ydotool` work; the bundled GNOME Shell extension is only needed for GNOME Wayland. Exact per-window focus may be unavailable without the extension backend. |
| KDE Plasma / KWin | temporary KWin DBus scripting | Lists and focuses windows through `org.kde.KWin` scripting when the session bus exposes it. |
| Hyprland | `hyprctl clients -j` and `hyprctl dispatch focuswindow` | Requires `hyprctl` in the desktop session. |
| i3 | `i3-msg`; optional `xprop` for PID hydration | Lists and focuses i3 windows over the active i3 IPC socket. |
| COSMIC Wayland | `computer-use-linux-cosmic` helper | Installed automatically by `./install.sh`, `cargo install`, and npm. For custom/manual layouts, put the helper next to the main binary, on `PATH`, or point `COMPUTER_USE_LINUX_COSMIC_HELPER` at it. |
| Sway / generic wlroots | no dedicated backend yet | AT-SPI, screenshots, and global `ydotool` input can still work; exact window list/focus is currently unavailable unless another backend applies. |
| Generic X11 / XFCE / other WMs | no dedicated backend yet | AT-SPI plus `ydotool` global input only, unless running under i3. |

If you run on a desktop not covered above, or a covered backend does not come up cleanly, please open an issue with the output of `computer-use-linux doctor` so we can extend the matrix honestly.

## Install

COSMIC users do not need a second package or a separate helper install when using `./install.sh`, `cargo install`, or the npm wrapper. Those paths install `computer-use-linux-cosmic` alongside the main binary automatically. Only manual prebuilt-binary installs need you to copy both release assets.

### Option A — `./install.sh` from a clone

Installs system packages on Debian/Ubuntu, Fedora/RHEL-like, or Arch-like distros; installs Rust if needed; builds both release binaries; installs them to `~/.local/bin`; enables `ydotoold` as a user service; enables GNOME AT-SPI settings when running under GNOME; and installs the bundled GNOME Shell extension on GNOME Wayland.

```bash
git clone https://github.com/agent-sh/computer-use-linux
cd computer-use-linux
./install.sh
# log out and back in if the GNOME extension was newly installed
computer-use-linux doctor | jq .readiness
```

### Option B — `cargo install` (Rust binaries, no system setup)

Installs the Rust binaries from crates.io. You still handle the system-level pieces yourself: `ydotoold`, AT-SPI, desktop portals, and the GNOME extension if you need the GNOME Wayland exact-focus backend.

```bash
cargo install computer-use-linux
computer-use-linux doctor
```

For unreleased changes from `main`, install directly from Git:

```bash
cargo install --git https://github.com/agent-sh/computer-use-linux
```

Then, as needed:

```bash
sudo apt install ydotool at-spi2-core         # or your distro's equivalent
systemctl --user enable --now ydotoold
computer-use-linux setup                      # gsettings AT-SPI bridge
computer-use-linux setup-window-targeting     # GNOME Shell extension
```

### Option C — npm wrapper (binary download)

Good for users who already have Node.js and want a no-Rust install. The npm package downloads and verifies the matching main and COSMIC helper binaries during install, then the wrapper sets `COMPUTER_USE_LINUX_COSMIC_HELPER` to the bundled helper automatically.

```bash
npm install -g @agent-sh/computer-use-linux
computer-use-linux doctor
```

You will still need `ydotoold` running and AT-SPI enabled (run `computer-use-linux setup` and the systemd commands above).

### Option D — prebuilt binaries

Linux x86_64 / aarch64 builds are published with each tag. Each binary ships a `.sha256` next to it.

- Latest release: <https://github.com/agent-sh/computer-use-linux/releases/latest>

```bash
target=x86_64-unknown-linux-gnu
base=https://github.com/agent-sh/computer-use-linux/releases/latest/download
for binary in computer-use-linux computer-use-linux-cosmic; do
  asset="$binary-$target"
  curl -L -O "$base/$asset"
  curl -L -O "$base/$asset.sha256"
  sha256sum -c "$asset.sha256"
  install -m 0755 "$asset" "$HOME/.local/bin/$binary"
done
```

You will still need `ydotoold` running and AT-SPI enabled (run `computer-use-linux setup` and the systemd commands above).

## Wire it into your MCP host

The binary speaks the `rmcp` 2024-11-05 stdio protocol. Pass `mcp` as the only argument; everything else is configured through MCP tool calls.

### Codex Desktop (Linux build)

The Linux build of Codex Desktop already bundles this binary as a plugin. You don't need to wire it up manually — the plugin definition lives in [`codex-desktop-linux`](https://github.com/avifenesh/codex-desktop-linux) under its `plugins/` directory and is enabled by default. To upgrade the plugin in place, replace the binary it ships with the one from this repo's release assets.

### Claude Code (CLI)

Use the `claude mcp add` command to register the binary as a stdio MCP server. Pick a scope:

- `--scope user` — available across all projects for your user.
- `--scope project` — written to `.mcp.json` at the project root for team sharing.
- `--scope local` (default) — only the current project, stored in `~/.claude.json`.

```bash
# User-wide install (recommended for desktop control)
claude mcp add --scope user computer-use-linux -- computer-use-linux mcp

# Verify the server is registered and reachable
claude mcp list
```

If `computer-use-linux` is not on `PATH`, pass the absolute path (e.g. `~/.local/bin/computer-use-linux`). Inside a Claude Code session, run `/mcp` to confirm the tools are loaded.

### Claude Desktop

Edit `~/.config/Claude/claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "computer-use-linux": {
      "command": "computer-use-linux",
      "args": ["mcp"]
    }
  }
}
```

Restart Claude Desktop. The tools should appear in the tools list.

### Hermes Agent

Install the companion Hermes skill so Hermes has the desktop-specific runbook:

```bash
hermes skills tap add agent-sh/computer-use-linux
hermes skills install agent-sh/computer-use-linux/computer-use-linux
```

The skill is optional but recommended for Hermes users. It teaches Hermes how to install, configure, verify, and call the Linux desktop MCP safely. It follows the same `skills/<name>/SKILL.md` tap layout used by Hermes community skills.

Then add the stdio MCP server:

```bash
hermes mcp add computer-use-linux --command computer-use-linux --args mcp
hermes mcp test computer-use-linux
hermes mcp configure computer-use-linux
```

`configure` opens Hermes' tool-selection UI for the server. The generated config should look like this:

```yaml
mcp_servers:
  computer-use-linux:
    command: computer-use-linux
    args: ["mcp"]
    timeout: 120
    connect_timeout: 30

# Optional: expose the tools to subagents as well.
inherit_mcp_toolsets: true
```

If you installed the binary somewhere that is not on `PATH`, pass the absolute path as `--command`.

Restart Hermes after editing the config. Hermes registers the tools as `mcp_computer_use_linux_<tool>` and creates the `mcp-computer-use-linux` runtime toolset.

You can verify both sides before asking Hermes to use the desktop:

```bash
computer-use-linux doctor | jq .readiness
hermes skills inspect agent-sh/computer-use-linux/computer-use-linux
hermes chat --toolsets mcp-computer-use-linux -q "List the current desktop windows."
```

For one-off installs without adding the tap first, Hermes also accepts `hermes skills install agent-sh/computer-use-linux/skills/computer-use-linux`.

### Generic MCP client

Spawn the binary with `["mcp"]` as the argv tail. It speaks JSON-RPC over stdio per the rmcp 2024-11-05 protocol; capability discovery happens through `tools/list` and the `doctor` tool. The server normally needs no MCP-specific configuration, but desktop runtime environment still matters (`DBUS_SESSION_BUS_ADDRESS`, `XDG_RUNTIME_DIR`, portals, AT-SPI, `ydotoold`, and optionally `COMPUTER_USE_LINUX_COSMIC_HELPER`).

## First-run checklist

1. **Run `doctor`.**

   ```bash
   computer-use-linux doctor | jq .readiness
   ```

   Aim for `can_register_mcp_tools`, `can_build_accessibility_tree`, `can_send_development_input`, and `can_query_windows` all `true`. The `blockers` array should be empty.

2. **If `accessibility.at_spi_bus.ok = false`** — run `computer-use-linux setup` (or call the `setup_accessibility` MCP tool). This sets:
   - `org.gnome.desktop.interface toolkit-accessibility true`

   You may need to restart toolkit-using apps for the change to take effect.

3. **If `windowing.can_list_windows = false`** — inspect `doctor.windowing.backends`. On GNOME Wayland, run `computer-use-linux setup-window-targeting` (or call `setup_window_targeting`) to install the bundled `computer-use-linux@avifenesh.dev` Shell extension, then log out and back in so GNOME Shell loads it. On KDE, Hyprland, i3, or COSMIC, install or expose the matching compositor tool/helper shown in the backend details.

4. **Grant the screencast portal on first screenshot.** The first time `get_app_state` or any screenshot subcommand runs, GNOME will pop a portal dialog asking to share the screen. Accept once and tick "remember" to make it sticky for the session.

5. **Confirm `ydotoold` is running.**

   ```bash
   systemctl --user status ydotoold
   ```

   Its socket should appear at `/run/user/$UID/.ydotool_socket`.

## Environment variables

Most setups need none of these — `doctor` and the installers pick sensible defaults. They exist for overriding auto-detected paths and input backends.

**Server runtime** (set in the MCP host's environment):

| Variable | Effect |
| --- | --- |
| `COMPUTER_USE_LINUX_COSMIC_HELPER` | Path to the `computer-use-linux-cosmic` helper when it isn't next to the binary or on `PATH`. |
| `CU_DISABLE_ABS_POINTER` | Disable the uinput absolute pointer and click through `ydotool` instead (for setups where the abs-pointer device misbehaves). |
| `COMPUTER_USE_LINUX_FORCE_PORTAL_POINTER` / `…_KEYBOARD` | Always route pointer / keyboard through the RemoteDesktop portal on Wayland, skipping auto-detection. |
| `COMPUTER_USE_LINUX_FORCE_YDOTOOL_POINTER` / `…_KEYBOARD` | Always route pointer / keyboard through `ydotool`, skipping the portal and KDE clipboard paths. |

**npm wrapper** (set during `npm install`, or before running):

| Variable | Effect |
| --- | --- |
| `COMPUTER_USE_LINUX_BIN` | Run this binary instead of the one bundled by the npm package. |
| `COMPUTER_USE_LINUX_DOWNLOAD_BASE` | Override the GitHub release base URL the installer downloads from (mirrors, air-gapped hosts). |
| `COMPUTER_USE_LINUX_SKIP_DOWNLOAD=1` | Skip the post-install binary download entirely. |
| `COMPUTER_USE_LINUX_LOCAL_BINARY` / `…_LOCAL_COSMIC_HELPER` | Install from a local build instead of downloading (used by CI and local testing). |

## Architecture

- **Accessibility tree** — [`atspi`](https://crates.io/crates/atspi) crate (tokio backend) talks to the AT-SPI registry on the user session bus. The tree is flattened to `(role, name, text, states, bounds)` tuples and indexed; element indices are stable for the duration of a `get_app_state` snapshot.
- **DBus where desktops expose it** — [`zbus`](https://crates.io/crates/zbus) for portal calls (`org.freedesktop.portal.Screenshot`, `…RemoteDesktop`, `…ScreenCast`), GNOME Shell screenshots (`org.gnome.Shell.Screenshot`), the bundled GNOME extension's `dev.avifenesh.ComputerUseLinux.WindowControl` service, and temporary KWin scripting.
- **MCP transport** — [`rmcp`](https://crates.io/crates/rmcp) with the `transport-io` feature; stdio framing, no network.
- **Input fallback** — when the remote-desktop portal isn't available or the host wants deterministic injection, the binary writes to `ydotoold`'s socket, which writes to `/dev/uinput`. `install.sh` can configure `ydotoold`; the `setup` command only enables the GNOME AT-SPI bridge.
- **Window registry** — `list_windows`, `focused_window`, `activate_window`, `press_key`, and `type_text` share a backend registry. It tries GNOME extension, GNOME Introspect, COSMIC helper, KWin scripting, Hyprland `hyprctl`, and i3 IPC in that order, skipping empty or failed backends so another compositor backend can answer.
- **GNOME extension fallback** — recent GNOME builds deny `org.gnome.Shell.Introspect.GetWindows` to non-blessed clients. The bundled Shell extension exposes window data and exact activation under `dev.avifenesh.ComputerUseLinux.WindowControl`.
- **COSMIC helper** — `computer-use-linux-cosmic` talks to COSMIC toplevel protocols and is resolved from `COMPUTER_USE_LINUX_COSMIC_HELPER`, next to the running binary, or from `PATH`.
- **Terminal enrichment** — `list_windows` cross-references each terminal window with its controlling TTY and the foreground process on that TTY, so `type_text` / `press_key` can target "the terminal where `pytest` is running" without the host ever knowing the window id.

## Security

Computer-use tooling is, by definition, a privilege-escalation surface. The threat model:

- **`ydotoold` runs as a per-user systemd service** with read/write access to `/dev/uinput`. Any process that can connect to its socket (`/run/user/$UID/.ydotool_socket`, mode `0600` by default) can synthesize arbitrary input — keypresses, clicks, anything. Keep the socket in the user runtime dir (the default), not in `/tmp` or any world-readable location. Do not run `ydotoold` as a system service.
- **The screencast portal asks for permission once per session.** Granting it lets the calling MCP host capture the screen for the rest of the session. If you don't want that, decline the portal dialog and use `get_app_state` with `include_screenshot: false`.
- **AT-SPI exposes window contents to any client on your session bus.** Enabling the AT-SPI bridge (`setup_accessibility`) is a prerequisite for this binary; it's also what screen readers use, and it shares the same trust boundary.
- **The GNOME Shell extension** is loaded only into your user's GNOME Shell, runs in the Shell's JS sandbox, and exposes a single DBus interface on the user session bus. It does not request any extra permissions.
- **No network.** This binary opens no TCP/UDP listener, makes no outbound Internet connections, and ships no telemetry. It does use local session transports such as DBus and the per-user `ydotoold` Unix socket.
- **Mutating tools are explicit.** The MCP tool list annotates read-only versus mutating tools, and CI fails if the published tool annotations drift from the table above. Treat those annotations as hints; the host is still responsible for user approval and policy.

If you're running this on a shared workstation, set `ydotoold`'s socket permissions to `0600` (the default) and audit which processes on your user can `connect()` to it.

## Troubleshooting

`computer-use-linux doctor` is the source of truth. Common failure modes and fixes:

- **`accessibility.at_spi_bus.ok = false`** — AT-SPI registry isn't running or the toolkit bridge is off. Fix: `computer-use-linux setup` (or call the `setup_accessibility` MCP tool). Restart the apps you want to drive.
- **`windowing.gnome_shell_introspect.ok = false` and `gnome_shell_extension_dbus.ok = false`** — GNOME blocks introspection and the extension isn't installed. Fix: `computer-use-linux setup-window-targeting`, then log out and log back in.
- **`input.ydotool_socket.ok = false`** — daemon isn't running. Fix: `systemctl --user enable --now ydotoold`. If the unit doesn't exist, install the `ydotool` package and rerun `./install.sh` (or copy the unit from `systemd/ydotoold.service` in this repo).
- **`input.uinput.ok = false`** — `/dev/uinput` isn't accessible to your user. Fix: add yourself to the `input` group (`sudo usermod -aG input $USER`) and re-login. On distros that ship `uinput` as a kernel module without auto-loading it, add `uinput` to `/etc/modules-load.d/`.
- **Portal calls hang or time out** — `xdg-desktop-portal` or its backend (`-gnome`, `-gtk`, `-kde`, `-wlr`) crashed. Fix: check `journalctl --user -u xdg-desktop-portal -u xdg-desktop-portal-gnome --since '5 min ago'` and restart the relevant unit.
- **KWin / Hyprland / i3 / COSMIC windowing is unavailable** — check `doctor.windowing.backends`. KWin needs session-bus scripting; Hyprland needs `hyprctl`; i3 needs `i3-msg` and its IPC socket. COSMIC needs `computer-use-linux-cosmic`, which the standard installers provide automatically; if you copied binaries by hand, copy the helper too or set `COMPUTER_USE_LINUX_COSMIC_HELPER`.
- **Screenshots return black frames on multi-monitor setups** — known portal / compositor edge case. Use `get_app_state` with `include_screenshot: false` and rely on AT-SPI until the portal backend is healthy.
- **`type_text` types into the wrong window** — pass an explicit target (`window_id`, `pid`, `wm_class`, `title`, or for terminals `tty` / `terminal_pid` / `terminal_command` / `terminal_cwd`). Without a target, input goes to whatever window currently has compositor focus.

If `doctor` is green and a specific tool still misbehaves, file an issue with the JSON output of `doctor` and the failing tool's request payload.

## Related

- [agent-workspace-linux](https://github.com/agent-sh/agent-workspace-linux) — the sibling MCP that gives an agent its **own** isolated Linux desktop (a hidden Xvfb display with its own apps and browser) instead of driving yours. It is the inverse of this project: `computer-use-linux` automates the desktop you are already on; `agent-workspace-linux` sandboxes the agent in a separate one. Use them together.

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for the local development workflow, CI gates, and PR expectations. Report security vulnerabilities through [SECURITY.md](SECURITY.md), not public issues.

## Credits

Extracted from [`codex-desktop-linux`](https://github.com/avifenesh/codex-desktop-linux), the Linux distribution of Codex Desktop, which continues to ship this same binary as a bundled plugin. Maintained by [Avi Fenesh](https://github.com/avifenesh).

Built on top of:

- [`atspi`](https://crates.io/crates/atspi) — AT-SPI bindings
- [`zbus`](https://crates.io/crates/zbus) — async DBus
- [`rmcp`](https://crates.io/crates/rmcp) — MCP runtime
- [`ydotool`](https://github.com/ReimuNotMoe/ydotool) — Wayland-friendly uinput driver
- [`cosmic-protocols`](https://crates.io/crates/cosmic-protocols) — COSMIC Wayland toplevel protocol bindings

## Publishing

Publishing is tag-driven from GitHub Actions. The repository needs these Actions secrets:

```bash
gh secret set CARGO_REGISTRY_TOKEN -R agent-sh/computer-use-linux
gh secret set NPM_TOKEN -R agent-sh/computer-use-linux
```

Then bump `Cargo.toml` and `package.json` together, update `CHANGELOG.md`, and push a `vX.Y.Z` tag. CI runs the full Rust and MCP safety gates, builds release assets for both architectures, publishes `computer-use-linux` to crates.io, and publishes the npm wrapper after the GitHub release binaries are available.

## License

MIT — see [LICENSE](LICENSE).

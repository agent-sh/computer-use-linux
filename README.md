<div align="center">
  <h1>computer-use-linux</h1>
  <p><strong>Native Linux desktop control for AI agents — over MCP, Pi native tools, and CLI.</strong></p>
  <p>
    <a href="https://github.com/agent-sh/computer-use-linux/actions/workflows/ci.yml"><img src="https://github.com/agent-sh/computer-use-linux/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
    <a href="https://crates.io/crates/computer-use-linux"><img src="https://img.shields.io/crates/v/computer-use-linux.svg" alt="crates.io"></a>
    <a href="https://www.npmjs.com/package/@agent-sh/computer-use-linux"><img src="https://img.shields.io/npm/v/@agent-sh/computer-use-linux.svg" alt="npm"></a>
    <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-yellow.svg" alt="License: MIT"></a>
  </p>
</div>

> Running open models for your company? [**Tiyuvta**](https://tiyuvta.ai/services/) helps with model choice, deployment and optimization, and fine-tuning on your hardware or cloud account.

`computer-use-linux` gives AI agents real, structured control of a local Linux desktop: semantic AT-SPI accessibility trees, bounded screenshots, compositor-aware window targeting, Wayland/X11 input synthesis, single-round-trip post-action verification, and a click-through on-screen activity indicator across **GNOME, KDE Plasma/KWin, Hyprland, niri, COSMIC, i3, and X11/EWMH**.

## Quick start

Pick the setup that matches your agent or shell workflow:

| Host / workflow | Install command |
| --- | --- |
| **Claude Code** (plugin + skill) | `claude plugin marketplace add agent-sh/computer-use-linux && claude plugin install computer-use-linux@computer-use-linux` |
| **Codex CLI** (plugin + skill) | `codex plugin marketplace add agent-sh/computer-use-linux && codex plugin add computer-use-linux@computer-use-linux` |
| **Pi Coding Agent** (native lazy-loaded tools) | `pi install npm:@agent-sh/computer-use-linux` |
| **Hermes Agent** (CLI + skill + MCP server) | `npm install -g @agent-sh/computer-use-linux && hermes skills tap add agent-sh/computer-use-linux && hermes skills install agent-sh/computer-use-linux/computer-use-linux && hermes mcp add computer-use-linux --command computer-use-linux --args mcp` |
| **CLI + any stdio MCP client** (npm) | `npm install -g @agent-sh/computer-use-linux` |
| **CLI + system setup from source** | `git clone https://github.com/agent-sh/computer-use-linux && cd computer-use-linux && ./install.sh` |
| **Rust crate (`crates.io`)** | `cargo install computer-use-linux` |

Verify desktop readiness from the shell (after installing the CLI on `PATH` via `npm install -g @agent-sh/computer-use-linux`, `./install.sh`, or `cargo install computer-use-linux`):

```bash
computer-use-linux doctor | jq .readiness
computer-use-linux setup                      # enable AT-SPI accessibility bridge if needed
computer-use-linux setup-window-targeting     # install GNOME Wayland window-control extension if needed
```

> **Plugin-only or Pi-only installs:** The Claude Code / Codex plugin launcher and `pi install` manage private cached binaries and do not add `computer-use-linux` to `PATH`. Either run the MCP `doctor` / `setup_accessibility` / `setup_window_targeting` tools (or `computer_use_linux_doctor` in Pi) directly from your agent, or also install the CLI above when you want shell subcommands on `PATH`.

The Rust crate is published as [`computer-use-linux`](https://crates.io/crates/computer-use-linux) and the npm package as [`@agent-sh/computer-use-linux`](https://www.npmjs.com/package/@agent-sh/computer-use-linux). Prebuilt `x86_64` and `aarch64` Linux binaries ship with the [latest release](https://github.com/agent-sh/computer-use-linux/releases/latest).

> Need an **isolated** desktop where the agent never touches your real mouse, keyboard, or focused window? Use the sibling project [**`agent-workspace-linux`**](https://github.com/agent-sh/agent-workspace-linux) alongside this one.

## What this is

`computer-use-linux` is a Rust MCP server and CLI for Linux desktop control. It ships three binaries — the main `computer-use-linux` server/CLI, the `computer-use-linux-cosmic` Wayland toplevel helper for COSMIC, and the `computer-use-linux-indicator` click-through activity overlay. Any MCP host — Claude Code, Codex CLI, Codex Desktop's Linux build, [Pi Coding Agent](#pi-coding-agent), [Hermes Agent](https://github.com/NousResearch/hermes-agent), Claude Desktop, or your own client — can spawn it to inspect accessibility trees, list and focus windows, take screenshots, click, drag, scroll, type, and invoke semantic accessibility actions.

Most computer-use MCP servers are macOS-only (they lean on AppKit, AXUIElement, CGEvent). The few that target Linux either drive `xdotool` against an X11 root window or shell out to OCR over screenshots. Five things set this one apart:

- **Wayland actually works.** Pointer actions use `/dev/uinput` or `org.freedesktop.portal.RemoteDesktop` on Wayland, with `ydotool` / `ydotoold` as the fallback. Literal text prefers KDE clipboard integration on Plasma, `wtype` on compatible Wayland compositors, and portal keysyms on other Wayland sessions, including GNOME with ydotool installed. Portal text startup or conversion errors fail the call without replaying text through ydotool. Raw ydotool text accepts printable ASCII, tab and newline only, and its physical US key positions can produce different text under the active layout. Screenshots use the GNOME Shell DBus screenshot method when present, `org.freedesktop.portal.Screenshot` otherwise, then on a native X11 session a root-window `GetImage` (device pixels, no toolkit scaling), and finally spawn `gnome-screenshot` for background/systemd contexts where the DBus paths are denied.
- **Window targeting is compositor-aware.** The window registry tries GNOME Shell extension, GNOME Shell Introspect, COSMIC Wayland helper, KWin DBus scripting, Hyprland `hyprctl`, the niri IPC socket, i3 IPC, and generic X11/EWMH in order, then reports exactly which backend won or why each backend failed. Terminal windows are enriched with their controlling TTY and foreground process so agents can target "the terminal running `pytest`" directly.
- **Semantic selectors first, pixel coordinates when needed.** Tools like `click`, `perform_action`, and `set_value` accept `role` / `name` / `text` / `states` selectors or `element_index` values backed by AT-SPI, and every mutating tool can return a post-action screenshot and updated accessibility tree in the same round trip. Pixel coordinates remain available for rendering-only surfaces (canvas, games, X clients without ATK).
- **Visible to humans, invisible to screenshots.** While an agent acts, `computer-use-linux-indicator` renders a click-through overlay with a per-agent cursor glide, click ripple, keycaps/typing banner, screen-edge glow, and status pill — and hides automatically under a shared capture lock before any screenshot is read.
- **One JSON readiness report.** `computer-use-linux doctor` returns a structured document covering platform, portals, AT-SPI, windowing, input, and a `readiness` summary with explicit blockers and a recommended next step. MCP hosts can render or surface that to the user without parsing prose.

The crate was extracted from [`codex-desktop-linux`](https://github.com/avifenesh/codex-desktop-linux) (the Linux distribution of Codex Desktop), which bundles this backend for its native Computer Use feature. This standalone repo is the upstream.

## MCP tools & features

| Category | Tool | What it does |
| --- | --- | --- |
| **Diagnostics & setup** | `doctor` | Single-shot JSON readiness report (platform, portals, accessibility, windowing, input, readiness summary, and capability map). |
| | `setup_accessibility` | Enables and verifies GNOME's `org.gnome.desktop.interface toolkit-accessibility` setting so toolkit apps expose AT-SPI trees. |
| | `setup_window_targeting` | Installs and enables the bundled GNOME Shell extension when `org.gnome.Shell.Introspect` is locked down. |
| **Discovery & observation** | `get_app_state` | Combined screenshot + compacted AT-SPI accessibility tree with stable `element_index` handles. Scope with `app_name_or_bundle_identifier` or a window target (`window_id`, `pid`, `app_id`, `wm_class`, `title`). |
| | `screenshot` | Bounded PNG or JPEG capture of the full desktop (`full_screen: true` or no target) or a raised and cropped target window (`window_id`, `pid`, `app_id`, `wm_class`, `title`, `raise_window`). |
| | `list_windows` | Compositor windows with `window_id`, title, `app_id`, `wm_class`, PID, focus state, client type (`wayland`/`x11`), bounds, and terminal TTY/process metadata. |
| | `focused_window` | The window currently holding keyboard focus. |
| | `list_apps` | Running desktop applications visible to the AT-SPI registry. |
| **Input** | `click` | Click by `element_index`, semantic selector (`role`/`name`/`text`/`states`), or desktop/window-relative coordinates (`left`/`right`/`middle`, `click_count`). |
| | `drag` | Desktop coordinate drag from `(start_x, start_y)` to `(end_x, end_y)`. |
| | `scroll` | Page-based vertical/horizontal scroll on an element, window, or pixel coordinate. |
| | `press_key` | Single keys or chords (`Ctrl+L`, `Alt+F4`, `Return`), optionally focusing a target window or terminal first. |
| | `type_text` | Literal text entry, optionally targeted at a window or terminal (`tty`, `terminal_pid`, `terminal_command`, `terminal_cwd`). |
| **Semantic actions** | `perform_action` | Invoke any AT-SPI action exposed by an element (`Press`, `Activate`, `Toggle`, `Jump`, …); defaults to the primary action. |
| | `set_value` | Write directly to a settable accessibility element (text fields, sliders, spinners). |
| **Window management** | `activate_window` | Focus a window by `window_id`, `pid`, `app_id`, `wm_class`, `title`, or terminal selectors. |
| | `move_window` / `resize_window` | Reposition or resize a window in desktop coordinates (GNOME Shell extension and X11/EWMH backends). |
| **Opt-in host tools** | `complete_interaction` | Parameter-free desktop completion notification (`notify-send`); registered only when `COMPUTER_USE_LINUX_NOTIFY_ON_COMPLETE=1`. |
| | `run_shell` | Bounded non-login `/bin/sh -c` execution; registered only when `COMPUTER_USE_LINUX_ENABLE_SHELL=1` (absent by default). |

### Post-action observation in one round trip

All mutating tools (`click`, `drag`, `scroll`, `press_key`, `type_text`, `perform_action`, `set_value`, `activate_window`, `move_window`, and `resize_window`) accept optional post-action observation parameters so an action and its verification complete in a single MCP tool call:

- `screenshot_after: true | "screen" | "window" | false` — capture after the action is dispatched and the indicator overlay is hidden (`true` or `"screen"` for full desktop, `"window"` cropped to the action's target window or the focused window when untargeted)
- `format`, `quality`, `scale`, `max_width`, `max_height`, `max_bytes` — same screenshot payload controls as `screenshot` and `get_app_state`
- `include_tree: true` — include a fresh compacted AT-SPI accessibility tree and update cached `element_index` values

When post-action screenshot capture succeeds, the image is returned as the first MCP `image` content block alongside the JSON action summary and `screenshot` metadata. If the action succeeds (`ok: true`) and only the post-action screenshot or tree capture fails, `ok` remains `true` and the error is reported in `screenshot_error` / `accessibility_error` and appended to `message`.

### Screenshot sizing & coordinate scaling

Screenshot payloads are size-bounded by default before they are returned to the MCP host: max 1920 px width/height and 2 MiB image bytes, with hard caps even when callers request more. Agents that need more detail can pass `max_width`, `max_height`, `max_bytes`, `scale`, `format: "jpeg"`, or `quality`, preferably with a window target or crop. PNG remains the default; JPEG lets callers trade lossless pixels for a smaller payload before the byte cap forces further resizing. Returned screenshot metadata includes `coordinate_width`, `coordinate_height`, `scale`, `format`, and `quality` so callers can convert from a downscaled preview to desktop coordinate pixels.

### Input & semantic targeting contracts

For a plain left `click` by element index or selector, a recognized native
AT-SPI `click`, `press`, or `toggle` action takes precedence
over bounds. Entry `activate` and slider `jump` must be requested explicitly
with `perform_action`; they are never a substitute for a pointer click, even
when bounds are missing.
This avoids pointer conversion for GTK3 HiDPI
extents and GTK4 zero-origin bounds when the element exposes such an action.
The preference does not substitute an arbitrary action name for a coordinate
click. Explicit `x`/`y`, right clicks, and double/multiple clicks retain pointer
semantics. Re-check application state after either kind of activation.

On KDE Wayland, element clicks that need pointer dispatch match the cached AT-SPI frame to a coherent KWin window/output snapshot, then map the target separately to physical capture pixels (uinput) and logical desktop coordinates (portal). Client-local and decorated frames, actual decoration offsets, and nonzero or negative output origins are supported when the geometry and uniform output scale can be verified. Ambiguous windows, mixed or unknown scales, overlapping outputs, or stale pointer geometry fail before input.

Explicit `x`/`y` defaults to `coordinate_space: "capture"` (raw screenshot pixels). To click a global AT-SPI screen point, pass `coordinate_space: "atspi"` together with its `element_index` or semantic selector; KDE Wayland then applies the same native KWin decoration and scale conversion as an element-bounds click, including when the AT-SPI window frame is client-local. This mode requires both coordinates and element identity and cannot be combined with `relative: true`. Other calls keep their existing coordinate behavior.

For coordinate `click` or `scroll` with `relative: true`, select a target window
and measure from its clipped screenshot crop origin. Divide preview `x` and
`y` by the returned screenshot `scale` before passing them. These are not raw
GDK surface or widget-local coordinates; decorations and clipping can change
the origin. A missing window target is rejected.

**X11 text fidelity:** `xdotool`/XTEST can silently substitute symbols under non-US XKB layouts (reported with Brazilian `br`/ABNT2) or changing keymap state, even when `type_text` returns `ok: true`. Every successful xdotool text dispatch includes a layout warning. XKB layout replacement events, persistent mapping/group changes, and keyboard focus/grab transitions detected across a call return an error after dispatch, without replaying the text. Keyboard grabs can divert characters without changing the polled focused-window ID. If monitoring cannot start, xdotool still dispatches and the response explicitly warns that layout-change and keyboard interception detection were unavailable. PointerRoot focus is unsupported because pointer motion can redirect input without a detectable focus event. This also detects a `setxkbmap` layout changed and restored during one call. Temporary raw remaps or group switches restored within a call and dropped text with a stable map remain unverified. Re-observe the actual text before pressing Enter, submitting it, or treating it as delivered. Use `include_tree: true` for AT-SPI readback when the application exposes the text, or `screenshot_after: true` to inspect it. These observations do not automatically compare the field to the requested text. Do not retry blindly.

Targeted `press_key`/`type_text` results append focused-element feedback from AT-SPI (role, name, editable) and warn when a concrete non-terminal focus is not editable. Terminals accept keyboard input without an EditableText interface; re-observe the contents to confirm input. Click/screenshot/input results warn when the target window or coordinate is partially or fully off-screen. `get_app_state` returns a compact readiness block by default; pass `verbose: true` for the full diagnostics report. It also reports `tree_scoped` (false when no app target narrowed the AT-SPI tree, with a warning in `message`) and `accessibility_tree_truncated` (true when the node, depth, or read budget stopped traversal with unread elements left).

An `app_id` may be a full desktop ID or its final dot-separated segment; matching is case-insensitive, ambiguous aliases are rejected, and an unresolved explicit `app_id` returns an empty scoped tree rather than the desktop.

`set_value` on editable text reads the full contents back after dispatch. `verified: true` means an exact match; mismatches and readback failures return `ok: false` without replaying the mutation. Password fields return `verified: false` because masked content cannot prove the write. Re-observe after any mismatch; the contents may have changed.

On X11, text input sends LF and CRLF line endings as Return, with one newline per CRLF pair.

On GNOME, a read-only keymap preflight rejects the whole string before input if a character is absent or requires a modifier level Mutter cannot synthesize. The background reader cannot establish the active layout group, so every configured group must support the text. This can reject text that works in one of several groups; use `set_value` on an editable field instead. The check requires `libxkbcommon.so.0` at runtime and stops after a two-second keymap read deadline. It does not cover held modifiers or a keymap changed concurrently with typing.

### On-screen indicator

While an agent acts, the server drives `computer-use-linux-indicator`, a click-through overlay that shows what it is doing: a software cursor tinted per agent that glides to each coordinate `click`, `drag`, or `scroll` target and ripples on click (AT-SPI actions such as `perform_action`, `set_value`, and element clicks show in the pill without moving it, since no pointer moves); keycaps for `press_key` and a typing line for `type_text`; a glow along every screen edge; and a status pill such as *Claude is using your computer · click*. Pointer actions wait 350 ms so the cursor lands before the real input. While `screenshot` or `get_app_state` captures the screen, the overlay draws nothing, so agents never see it: not their own, not one shown for another agent, and not one that another agent starts mid-capture. This holds with `COMPUTER_USE_LINUX_INDICATOR=0` too, which only stops a server from showing its own actions. The capture waits for the compositor only when something was on screen. The agent name comes from the MCP client's `clientInfo`.

The server starts the indicator on the first action and it exits after ten idle minutes. It stays hidden between bursts of activity. Compositors with `wlr-layer-shell` use the shared-memory overlay. GNOME uses native Shell actors through the bundled extension. Run `setup_window_targeting` once, then log out and back in after installing or updating the extension. Both renderers use the same capture lock and acknowledgement protocol. Set `COMPUTER_USE_LINUX_INDICATOR=0` to turn it off.

Screenshot processing uses one blocking worker per server. Captures enter one pipeline before reading pixels, so concurrent requests do not queue decoded full-screen images. A source can contain up to 134,217,728 pixels; serial processing bounds decoded-image memory while letting MCP answer other requests during resizing and encoding. Session discovery is reused after a successful probe and refreshed when session variables change. Failed discovery is retried.

Captures require the shared capture lock and, when an overlay is running, its acknowledgement that it has hidden. A missing or invalid acknowledgement, a stalled socket, or a failed lock returns a capture error. Concurrent captures keep separate holds; their acknowledgements are exchanged in order.

### MCP safety contract

`computer-use-linux` is not a read-only data source. It can observe the local desktop and, when a mutating tool is called, can change real application state. The `tools/list` response includes MCP `ToolAnnotations` so hosts can surface this distinction before invocation:

| Class | Tools | Contract |
| --- | --- | --- |
| Read-only observation | `doctor`, `list_apps`, `list_windows`, `focused_window`, `get_app_state` | `readOnlyHint=true`; may reveal app, window, accessibility, and screenshot contents. `get_app_state` may trigger the desktop screenshot portal prompt. |
| Local setup mutators | `setup_accessibility`, `setup_window_targeting` | `readOnlyHint=false`, `destructiveHint=false`, `idempotentHint=true`; modifies user desktop configuration by enabling accessibility or installing/enabling the GNOME window-targeting extension. |
| UI state mutators | `activate_window`, `move_window`, `resize_window`, `scroll`, `screenshot` | `readOnlyHint=false`, `destructiveHint=false`; changes focus, geometry, or scroll position in the live desktop, or raises a window to capture it. |
| Desktop action mutators | `click`, `drag`, `press_key`, `type_text`, `perform_action`, `set_value` | `readOnlyHint=false`, `destructiveHint=true`, `openWorldHint=true`; can trigger arbitrary actions in whatever local application is targeted. |
| Conditional host-code execution | `run_shell` | Absent unless `COMPUTER_USE_LINUX_ENABLE_SHELL=1`; when enabled, `readOnlyHint=false`, `destructiveHint=true`, `idempotentHint=false`, `openWorldHint=true`. Runs with the MCP server user's host permissions. |

Annotations are safety hints, not an authorization system. MCP hosts should still ask the user before calls that could submit, delete, send, purchase, overwrite, or otherwise commit state.

`run_shell` is an explicit trust-boundary opt-in, not a restricted command runner. Enabling it grants an approved MCP call the same file and network authority as the user running the server. The tool clears the ambient environment and inherits only a small desktop/runtime allowlist (`PATH`, home/user/locale fields, display/session-bus fields); additional variables must be supplied in the visible call payload. Commands use a fixed non-login `/bin/sh`, an existing canonical working directory, a 30-second default / 120-second hard timeout, process-group cleanup, and stderr audit records keyed by the command SHA-256 rather than command text. Collected streams up to 8 MiB are returned with a 512 KiB per-stream response cap and truncation flag; exceeding 8 MiB on either stream fails the call without partial output. These controls bound accidental leakage and runaway work; they do not make arbitrary shell code safe.

The binary also exposes the same capabilities from the CLI for scripting and debugging:

```
computer-use-linux mcp                                  # stdio MCP server
computer-use-linux doctor                               # JSON readiness report
computer-use-linux setup                                # enable AT-SPI
computer-use-linux guard-accessibility                  # explicit foreground GNOME accessibility guard
computer-use-linux setup-window-targeting               # install GNOME Shell extension
computer-use-linux apps
computer-use-linux state [APP_NAME]
computer-use-linux screenshot                           # JSON screenshot summary
computer-use-linux windows
```

## Support matrix

Validated manually on Ubuntu 25.10 (GNOME Shell 50.1, Wayland). niri window listing and exact focus were also validated on Ubuntu 26.10 (development), niri 26.04, with a single 2x output, through both CLI and direct IPC. Other compositor backends are implemented and covered by parser / contract tests, but real desktop behavior still depends on each session exposing its expected control API.

| Desktop/session | Window backend | Notes |
| --- | --- | --- |
| GNOME Wayland | GNOME Shell extension first, `org.gnome.Shell.Introspect` fallback | Full target. The extension provides exact window activation when GNOME blocks native introspection; Introspect can list windows and focus apps by `app_id` when allowed. |
| GNOME X11 | `org.gnome.Shell.Introspect`, then generic X11/EWMH | AT-SPI works; keyboard input prefers `xdotool`/XTEST so the live XKB layout resolves keys correctly. Scroll uses xdotool wheel buttons so GTK 3 does not drop the event after a pointer warp. |
| KDE Plasma / KWin | temporary KWin DBus scripting | Lists and focuses windows through Plasma 5 or 6 `org.kde.KWin` scripting APIs when the session bus exposes them. |
| Hyprland | `hyprctl clients -j` and `hyprctl dispatch focuswindow` | Requires `hyprctl` in the desktop session. |
| niri | `niri msg` with a direct JSON IPC fallback | Lists toplevels and focuses exact window IDs. Export the session's `NIRI_SOCKET`; otherwise discovery requires an unambiguous socket matching `WAYLAND_DISPLAY`. The fallback works without the `niri` binary. Missing positions remain `null`; bounds are omitted if output scaling is unknown or mixed. |
| i3 | `i3-msg`; optional `xprop` for PID hydration | Lists and focuses i3 windows over the active i3 IPC socket. |
| COSMIC Wayland | `computer-use-linux-cosmic` helper | Installed automatically by `./install.sh`, `cargo install`, and npm. For custom/manual layouts, put the helper next to the main binary, on `PATH`, or point `COMPUTER_USE_LINUX_COSMIC_HELPER` at it. The on-screen indicator was validated here (three outputs, mixed fractional scaling). |
| Sway / generic wlroots | no dedicated backend yet | AT-SPI, screenshots, and global `ydotool` input can still work; exact window list/focus is currently unavailable unless another backend applies. |
| Generic X11 / XFCE / other EWMH WMs | `wmctrl` plus `xprop` | Lists, focuses, moves, and resizes windows; keyboard input prefers `xdotool`/XTEST, and scroll uses xdotool wheel buttons. Window origins are read from the X server, since `wmctrl -lG` counts the frame offset twice. |

If you run on a desktop not covered above, or a covered backend does not come up cleanly, please open an issue with the output of `computer-use-linux doctor` so we can extend the matrix honestly.

## Install

`./install.sh`, `cargo install`, the npm wrapper, and the Claude/Codex plugin launcher all install `computer-use-linux`, `computer-use-linux-cosmic` (COSMIC Wayland helper), and `computer-use-linux-indicator` (on-screen activity overlay) together automatically. Only manual prebuilt-binary installs need you to download all three release assets.

### Option A — `./install.sh` from a clone

Installs system packages on Debian/Ubuntu, Fedora/RHEL-like, Arch-like, or Artix systems; installs Rust if needed; builds all three release binaries (`computer-use-linux`, `computer-use-linux-cosmic`, and `computer-use-linux-indicator`); installs them to `~/.local/bin`; configures `ydotoold` as a systemd user service when available; enables GNOME AT-SPI settings when running under GNOME; and installs the bundled GNOME Shell extension on GNOME Wayland.

```bash
git clone https://github.com/agent-sh/computer-use-linux
cd computer-use-linux
./install.sh
# log out and back in if the GNOME extension was newly installed
computer-use-linux doctor | jq .readiness
```

`ydotool` is an optional fallback. On X11, the installer includes `xdotool` as the required keyboard backend. If ydotool is unavailable from the configured repositories, the installer continues, but `doctor` still requires a keyboard-capable RemoteDesktop portal on Wayland or xdotool on X11; direct uinput provides absolute pointer input only. On non-systemd hosts, automatic `ydotoold` service setup is skipped and the installer prints a command suitable for a per-user supervisor. For an unrecognized distro, pass `--package-manager apt|dnf|pacman`; `--force-unknown-distro` auto-selects only when exactly one of those managers is available.

### Option B — `cargo install` (Rust binaries, no system setup)

Installs all three Rust binaries from crates.io. You still handle the system-level pieces yourself: AT-SPI, desktop portals, the optional `ydotoold` fallback, and the GNOME extension if you need the GNOME Wayland exact-focus backend.

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
sudo apt install ydotool at-spi2-core         # ydotool 1.0.3+ when using this fallback
sudo apt install wtype                        # optional Unicode typing on wlroots/Hyprland Wayland
systemctl --user enable --now ydotoold         # only when doctor selects ydotool
computer-use-linux setup                      # gsettings AT-SPI bridge
computer-use-linux setup-window-targeting     # GNOME Shell extension
```

### Option C — npm wrapper (binary download)

Good for users who already have Node.js and want a no-Rust install. The npm package downloads and verifies the matching `computer-use-linux`, `computer-use-linux-cosmic`, and `computer-use-linux-indicator` binaries during install, then the wrapper wires `COMPUTER_USE_LINUX_COSMIC_HELPER` and `COMPUTER_USE_LINUX_INDICATOR_BIN` to the bundled helpers automatically.

```bash
npm install -g @agent-sh/computer-use-linux
computer-use-linux doctor
```

You will still need AT-SPI enabled (`computer-use-linux setup`) and one input backend reported ready by `doctor`. Start `ydotoold` only when using the ydotool fallback.

### Option D — prebuilt binaries

Linux x86_64 / aarch64 builds are published with each tag. Each binary ships a `.sha256` next to it.

- Latest release: <https://github.com/agent-sh/computer-use-linux/releases/latest>

```bash
target=x86_64-unknown-linux-gnu
base=https://github.com/agent-sh/computer-use-linux/releases/latest/download
for binary in computer-use-linux computer-use-linux-cosmic computer-use-linux-indicator; do
  asset="$binary-$target"
  curl -L -O "$base/$asset"
  curl -L -O "$base/$asset.sha256"
  sha256sum -c "$asset.sha256"
  install -m 0755 "$asset" "$HOME/.local/bin/$binary"
done
```

You will still need AT-SPI enabled (`computer-use-linux setup`) and one input backend reported ready by `doctor` (start `ydotoold` only when using the ydotool fallback).

## Wire it into your MCP host

The binary speaks the `rmcp` 2024-11-05 stdio protocol. Pass `mcp` as the only argument; everything else is configured through MCP tool calls.

### Claude Code and Codex plugin

This repo is a plugin marketplace for both Claude Code and Codex. The plugin
registers the MCP server and ships the `computer-use-linux` skill. On first
start it downloads the release binaries pinned to the plugin version, checks
each against its published `.sha256`, and caches them under
`~/.cache/computer-use-linux/plugin/`. It needs `curl` or `wget`, and nothing
else is installed.

Claude Code:

```bash
claude plugin marketplace add agent-sh/computer-use-linux
claude plugin install computer-use-linux@computer-use-linux
```

Codex CLI:

```bash
codex plugin marketplace add agent-sh/computer-use-linux
codex plugin add computer-use-linux@computer-use-linux
```

Start a new session afterwards. To run your own build instead of the pinned
release, set `COMPUTER_USE_LINUX_BIN` to its path in the host's environment.
System setup (`computer-use-linux setup`, `setup-window-targeting`, ydotoold)
still applies; ask the agent to run the `doctor` tool to see what is missing.
Codex passes only a short allowlist of environment variables to MCP servers,
so the Codex manifest forwards the desktop session variables and the
`COMPUTER_USE_LINUX_*` settings below explicitly. Codex Desktop's Linux build
already bundles Computer Use; do not install both there.

### Codex Desktop (Linux build)

[`codex-desktop-linux`](https://github.com/avifenesh/codex-desktop-linux) includes a native `computer-use-linux` feature that embeds and stages this backend inside the app's unified Computer Use plugin. Enable `computer-use-linux` via `make setup-native` (or in `linux-features/features.json` followed by `make install-native`), then toggle **Settings → Computer use → Any App** in the desktop app. Do not register a second standalone `computer-use-linux` MCP server inside Codex Desktop when the native feature is enabled.

### Claude Code (CLI)

The plugin above is the simplest route. To wire a binary you installed yourself, use the `claude mcp add` command to register the binary as a stdio MCP server. Pick a scope:

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

### Pi Coding Agent

```bash
pi install npm:@agent-sh/computer-use-linux
```

Restart Pi or run `/reload`. The package exposes one small loader initially;
the real tools keep their upstream schemas and are enabled only when Computer
Use is needed:

Native tools require Pi 0.84.4 or newer (Node.js 22.19 or newer). The
standalone npm CLI wrapper continues to support Node.js 18 or newer.

```
computer_use_linux_tools({ tools: ["doctor", "list_windows"] })
computer_use_linux_doctor({})
computer_use_linux_list_windows({})
```

You can also search by capability:

```
computer_use_linux_tools({ query: "observe a window and click a control" })
```

Codemode scripts receive `{ text, images, isError }`. Each image is an MCP image
block that can be passed directly to `image()`:

```js
const result = await tools.computer_use_linux_screenshot({});
if (result.isError) throw new Error(result.text);
text(result.text);
for (const block of result.images) image(block);
```

No separate MCP adapter or manual MCP configuration is required. Pi starts one
computer-use-linux process lazily on the first real tool call, reuses it for the
session so accessibility snapshots remain valid, serializes desktop actions,
and closes it on reload, session switch, or exit. See the
[Pi setup guide](skills/computer-use-linux/references/pi-setup.md) for migration
from older adapter-based installs.

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

   Aim for `can_register_mcp_tools`, `can_build_accessibility_tree`, `can_send_development_input`, and `can_query_windows` all `true`, with an empty `blockers` array. Doctor lists detected screenshot routes in `capabilities.screenshot`, but does not take a screenshot or request consent. Detected routes are always `screenshot_capture_status: "unverified"` and `can_capture_screenshots: false` in a doctor report. `get_app_state` records the result of its requested raw capture as `verified` or `failed`; only `verified` sets `can_capture_screenshots: true` in that response. If no route is detected, the status is `unavailable`. An unverified capture is a warning, not an installation blocker.

2. **If `accessibility.at_spi_bus.ok = false`** — run `computer-use-linux setup` (or call the `setup_accessibility` MCP tool). This sets:
   - `org.gnome.desktop.interface toolkit-accessibility true`

   You may need to restart toolkit-using apps for the change to take effect.

3. **If `windowing.can_list_windows = false`** — inspect `doctor.windowing.backends`. On GNOME Wayland, run `computer-use-linux setup-window-targeting` (or call `setup_window_targeting`) to install the bundled `computer-use-linux@avifenesh.dev` Shell extension, then log out and back in so GNOME Shell loads it. On KDE, Hyprland, niri, i3, COSMIC, or generic X11, install or expose the matching compositor tool/helper shown in the backend details.

4. **Verify screenshot capture.** Call `get_app_state` or run `computer-use-linux screenshot`. The Screenshot portal may request consent for the caller's app ID. On GNOME, that dialog can be rejected when the app ID does not match the focused app. A detected portal interface alone does not prove capture permission. Inspect the full backend error and the desktop portal logs if capture fails; `gnome-screenshot` is an optional fallback when GNOME Shell or portal calls are denied.

5. **Confirm compatible ydotool 1.0.3+ and `ydotoold` are available.**

   ```bash
   systemctl --user status ydotoold
   ```

   Its socket should appear at `/run/user/$UID/.ydotool_socket`.

## Environment variables

Most setups need none of these — `doctor` and the installers pick sensible defaults. They exist for overriding auto-detected paths and input backends.

**Server runtime** (set in the MCP host's environment):

| Variable | Effect |
| --- | --- |
| `COMPUTER_USE_LINUX_NOTIFY_ON_COMPLETE` | Set exactly to `1` to expose the optional `complete_interaction` notification tool. Requires `notify-send` and a desktop notification service; disabled by default. |
| `COMPUTER_USE_LINUX_COSMIC_HELPER` | Path to the `computer-use-linux-cosmic` helper when it isn't next to the binary or on `PATH`. |
| `COMPUTER_USE_LINUX_INDICATOR` | Set to `0` (or `false`/`off`/`no`) to turn off the on-screen indicator. On by default. |
| `COMPUTER_USE_LINUX_INDICATOR_BIN` | Path to the `computer-use-linux-indicator` overlay when it isn't next to the binary or on `PATH`. |
| `COMPUTER_USE_LINUX_INDICATOR_HIDE_TEXT` | Set exactly to `1` to mask all typed text and keycaps in the indicator. `set_value` values are always masked. |
| `COMPUTER_USE_LINUX_AGENT_NAME` | Name shown in the indicator instead of the one derived from the MCP client's `clientInfo`. |
| `CU_DISABLE_ABS_POINTER` | Disable the uinput absolute pointer. Wayland coordinate actions then need the RemoteDesktop portal; they refuse input rather than use ydotool's relative-motion approximation. Native X11 fallback is unchanged. |
| `COMPUTER_USE_LINUX_FORCE_PORTAL_POINTER` / `…_KEYBOARD` | Prefer the RemoteDesktop portal on Wayland, skipping auto-detection. Pointer forcing skips the absolute uinput backend. Forced ydotool or XTEST keyboard overrides take precedence; KDE literal text keeps its clipboard route. Portal startup and text conversion failures return errors without replaying through ydotool. |
| `COMPUTER_USE_LINUX_PORTAL_SCROLL_INVERT` | Set to `1` to reverse vertical scrolling through the RemoteDesktop portal. Defaults to normal direction on both GNOME and KDE Plasma; horizontal direction is unchanged. |
| `COMPUTER_USE_LINUX_PERSIST_REMOTE_DESKTOP` | Set exactly to `1` to ask the RemoteDesktop portal to remember pointer and keyboard grants across processes. The first dialog still appears (on GNOME the remember box starts checked). Later processes reuse the single-use restore token from `Start`, stored mode `0600` under `$XDG_STATE_HOME/computer-use-linux/` (or `~/.local/state/computer-use-linux/`). Pointer and keyboard tokens are separate. Needs interface version 2. Unset, every new process is prompted. No effect when input is not using the portal. |
| `COMPUTER_USE_LINUX_FORCE_YDOTOOL_POINTER` / `…_KEYBOARD` | Always route pointer / keyboard through `ydotool`, skipping the portal and KDE clipboard paths; pointer forcing also skips the absolute uinput backend and native-X11 `xdotool` coordinate clicks and scrolls. Coordinate pointer actions on Wayland then refuse input because ydotool cannot position them accurately; scrolling at the current pointer remains available. |
| `YDOTOOL_SOCKET` | Explicit ydotool socket path. When unset, only an owner-only `.ydotool_socket` in the user's private runtime directory is accepted; the legacy shared `/tmp/.ydotool_socket` is never selected automatically. Setting this variable is an operator trust override for custom layouts. |
| `COMPUTER_USE_LINUX_FORCE_XDOTOOL_KEYBOARD` | Prefer `xdotool`/XTEST keyboard input when `DISPLAY` is available. `COMPUTER_USE_LINUX_FORCE_YDOTOOL_KEYBOARD=1` takes precedence. |
| `COMPUTER_USE_LINUX_XDOTOOL_TYPE_DELAY_MS` | Per-character delay for `xdotool type` in milliseconds (default `12`). `0` is faster but can deliver characters out of order on some X servers. |
| `COMPUTER_USE_LINUX_SCREENSHOT_BACKEND` | Force a single screenshot backend, skipping the fallback chain. Accepts `gnome-shell`, `portal`, `x11`, or `gnome-screenshot`. `x11` works only on a native X11 session. Pin `gnome-screenshot` for background/systemd contexts where the GNOME Shell and portal DBus paths are denied. |
| `COMPUTER_USE_LINUX_ENABLE_SHELL` | Set exactly to `1` before starting the MCP server to register the destructive `run_shell` tool. Unset by default. Do not enable for untrusted or unattended MCP hosts. |

**Build-time identity overrides** (set while compiling a downstream embedded
bundle): `CUL_GNOME_EXTENSION_UUID`, `CUL_DBUS_SERVICE`, and
`CUL_DBUS_OBJECT_PATH` replace the default standalone GNOME Shell extension
UUID and DBus endpoint in both the Rust probes and the generated extension
files.

**npm wrapper** (set during `npm install`, or before running):

| Variable | Effect |
| --- | --- |
| `COMPUTER_USE_LINUX_BIN` | Run this absolute binary path instead of the one bundled by the npm package. Relative paths are ignored. |
| `COMPUTER_USE_LINUX_DOWNLOAD_BASE` | Override the GitHub release base URL the installer downloads from (mirrors, air-gapped hosts). |
| `COMPUTER_USE_LINUX_SKIP_DOWNLOAD=1` | Skip the post-install binary download entirely. |
| `COMPUTER_USE_LINUX_LOCAL_BINARY` / `…_LOCAL_COSMIC_HELPER` / `…_LOCAL_INDICATOR` | Install from a local build instead of downloading (used by CI and local testing). |

## Architecture

- **Accessibility tree** — [`atspi`](https://crates.io/crates/atspi) crate (tokio backend) talks to the AT-SPI registry on the user session bus. The tree is flattened to `(role, name, text, states, bounds)` tuples and indexed; element indices are stable for the duration of a `get_app_state` snapshot. A new snapshot is published only between input operations. `click`, `scroll`, `perform_action`, and `set_value` hold the input lock while they check and resolve an index, so a checked element cannot turn into another app's node mid-action.
- **DBus where desktops expose it** — [`zbus`](https://crates.io/crates/zbus) for portal calls (`org.freedesktop.portal.Screenshot`, `…RemoteDesktop`, `…ScreenCast`), GNOME Shell screenshots (`org.gnome.Shell.Screenshot`), the bundled GNOME extension's `dev.avifenesh.ComputerUseLinux.WindowControl` service, and temporary KWin scripting.
- **MCP transport** — [`rmcp`](https://crates.io/crates/rmcp) with the `transport-io` feature; stdio framing, no network.
- **Input fallback** — on X11, keyboard input prefers `xdotool`/XTEST and falls back only when xdotool cannot launch. On Wayland, literal text uses `wtype` when installed and the remote-desktop portal is unavailable; `wtype` supports Unicode through the virtual-keyboard protocol on compatible compositors such as Hyprland/wlroots. If wtype is unavailable, the binary falls back to a compatible ydotool 1.0.3+ CLI and `ydotoold` socket. A launched wtype failure is returned without replaying the text. `install.sh` can configure `ydotoold`; the `setup` command only enables the GNOME AT-SPI bridge.
- **Hyprland/Electron text** — for classified nonsecret fields on Hyprland, wtype passes one literal argument after `--` to avoid stdin keymap-refresh corruption. Argument text is visible in process listings; confirmed password fields are rejected. Terminals and ambiguous fields (with or without AT-SPI) use private stdin for at most 100 Unicode characters per call; split larger input into smaller calls or expose accessibility. The input probe has its own 1.5-second budget; the 250 ms indicator probe only masks text. Other compatible compositors retain their existing stdin path. NUL and text over 131,071 UTF-8 bytes are rejected before focus or indicators. Success confirms dispatch only; re-observe the field before retrying.
- **Limited accessibility feedback** — when exact window focus was verified before input but AT-SPI reports no focused element, the result describes the missing feedback without claiming the input was lost. It still does not verify the field contents. A complete scoped tree containing only application/window containers adds an Electron/Chromium `--force-renderer-accessibility` hint; a truncated tree instead retains its traversal-limit guidance.
- **Wayland coordinate input** — coordinate clicks, drags, and targeted scrolls require the absolute uinput pointer or an approved RemoteDesktop portal session. Targeted scroll positions through uinput before sending only wheel events through ydotool. If accurate positioning is unavailable, the call returns `ok: false` before sending coordinate input. The absolute pointer still requires a successful full-desktop capture when first created; an existing device does not need another capture. Scrolling without a point or window/element target keeps using the current pointer position.
- **Native X11 coordinate clicks** — eligible native X11 sessions use one supervised `xdotool mousemove -- X Y click --repeat N BUTTON` command for left, middle, and right clicks; ydotool is used only when xdotool cannot launch, while a launched nonzero xdotool command is reported as an error without replay. `COMPUTER_USE_LINUX_FORCE_YDOTOOL_POINTER=1` skips this xdotool path.
- **Native X11 scroll.** The same sessions send one `xdotool` command, `mousemove -- X Y click --delay 12 --repeat N BUTTON` when a point is set, or `click` alone when it is not. Buttons are 4 up, 5 down, 6 left, and 7 right, and `N` is the same notch count ydotool would pass to `mousemove --wheel` (five per page). ydotool's absolute move warps through (0, 0); GTK 3 then drops the single following wheel event because re-entry resets its XI2 scroll valuators. XTEST has no scroll valuators, so GTK takes the button path. A missing xdotool falls back to ydotool; a launched nonzero xdotool command does not. `COMPUTER_USE_LINUX_FORCE_YDOTOOL_POINTER=1` skips this path.
- **Native X11 transport.** Native X11 queries (window origins, root screenshots, `doctor`) connect and read replies under one shared 2-second deadline enforced in the transport itself, including the handshake and every reply. A stalled X server closes the query connection instead of leaving a blocked worker behind. Unix sockets, literal IP addresses, and `localhost` need no resolver. A remote hostname in `DISPLAY` needs `getent ahosts` for bounded name resolution; if it is missing or fails, the X11 route is reported as unavailable.
- **Window registry** — `list_windows`, `focused_window`, `activate_window`, `press_key`, and `type_text` share a backend registry. It tries GNOME extension, GNOME Introspect, COSMIC helper, KWin scripting, Hyprland `hyprctl`, the niri IPC socket, i3 IPC, and generic X11/EWMH in that order, skipping empty or failed backends so another compositor backend can answer.
- **niri backend** — prefers `niri msg`, falling back to direct JSON IPC over the same session socket. `is_minimized` maps to `hidden`. Bounds use `layout.window_size` (or `tile_size`) and, when available, the tile position plus the window's offset and the output's logical origin. Uniform output scaling converts these to device pixels; unknown or mixed scaling omits bounds. Missing window positions stay `null`, so listing and focusing still work without enabling coordinate targeting. An accepted focus action is verified by querying the focused window again.
- **GNOME extension fallback** — recent GNOME builds deny `org.gnome.Shell.Introspect.GetWindows` to non-blessed clients. The bundled Shell extension exposes window data and exact activation under `dev.avifenesh.ComputerUseLinux.WindowControl`.
- **COSMIC helper** — `computer-use-linux-cosmic` talks to COSMIC toplevel protocols and is resolved from `COMPUTER_USE_LINUX_COSMIC_HELPER`, next to the running binary, or from `PATH`.
- **Indicator overlay** — the server sends one JSON datagram per action (`{"agent","tool","x","y","space","keys","text"}`, where `x`/`y` are screenshot pixels and `space` is the capture size; or `{"capture":"begin"}`/`{"capture":"end"}` around captures; `begin` is answered with `shown` or `empty` when the sender's socket is bound) to `$XDG_RUNTIME_DIR/computer-use-linux-indicator.sock`. `computer-use-linux-indicator` (resolved like the COSMIC helper, via `COMPUTER_USE_LINUX_INDICATOR_BIN`) draws nothing while any server holds a shared `flock` on `computer-use-linux-indicator.capture` next to the socket (held for the length of each capture and released by the kernel if a server dies), and otherwise draws on small `wlr-layer-shell` surfaces with [`tiny-skia`](https://crates.io/crates/tiny-skia) into shared-memory buffers, honouring fractional scaling through `wp_fractional_scale_v1` and `wp_viewporter`. The cursor surface moves by changing its margins, so a glide redraws nothing. On GNOME the helper relays events to the verified Shell extension, which draws non-interactive actors. The helper still owns the shared socket and probes the capture lock. If the helper exits, the extension hides its actors when its D-Bus connection disappears. Captures also hide residual GNOME actors when no helper is listening.
- **Terminal enrichment** — `list_windows` cross-references each terminal window with its controlling TTY and the foreground process on that TTY, so `type_text` / `press_key` can target "the terminal where `pytest` is running" without the host ever knowing the window id.

## Security

Computer-use tooling is, by definition, a privilege-escalation surface. The threat model:

- **`ydotoold` runs as a per-user service** with read/write access to `/dev/uinput`. `install.sh` automates this for systemd user sessions and prints manual supervisor guidance elsewhere. Any process that can connect to its socket (`/run/user/$UID/.ydotool_socket`, mode `0600` by default) can synthesize arbitrary input — keypresses, clicks, anything. Keep the socket in the user runtime dir (the default), not in `/tmp` or any world-readable location. Do not run `ydotoold` as root or as a system service.
- **Screenshot consent depends on the portal and caller's app ID.** A saved permission can apply to multiple processes with that identity. Doctor only detects the available routes and does not request consent. Use `get_app_state` with `include_screenshot: false` to skip capture.
- **Persisted remote control is opt-in.** `COMPUTER_USE_LINUX_PERSIST_REMOTE_DESKTOP=1` stores the portal's single-use restore token in the user state directory, mode `0600`. A same-user process that can read that file can restore remote control without a new prompt until the desktop revokes the grant. Leave the variable unset to keep a prompt on every new process.
- **AT-SPI exposes window contents to any client on your session bus.** Enabling the AT-SPI bridge (`setup_accessibility`) is a prerequisite for this binary; it's also what screen readers use, and it shares the same trust boundary.
- **The GNOME Shell extension** is loaded only into your user's GNOME Shell, runs in the Shell's JS sandbox, and exposes a single DBus interface on the user session bus. It does not request any extra permissions.
- **The indicator shows what agents type.** `type_text` text (its last 64 characters) and `press_key` keycaps are drawn on screen unless the target is, or might be, a password field: both are replaced by a fixed mask when the focused element's AT-SPI role is a password role or cannot be read within 250 ms. Generic text boxes and terminal focus are also masked because their roles do not establish whether input is obscured. `set_value` values are always masked, because an app can change a field to a password after its accessibility snapshot. The text also travels over a socket in the user's private runtime directory. Set `COMPUTER_USE_LINUX_INDICATOR_HIDE_TEXT=1` to mask all typed text and keycaps, or `COMPUTER_USE_LINUX_INDICATOR=0` to turn the indicator off.
- **No network.** This binary opens no TCP/UDP listener, makes no outbound Internet connections, and ships no telemetry. It does use local session transports such as DBus and the per-user `ydotoold` Unix socket.
- **Mutating tools are explicit.** The MCP tool list annotates read-only versus mutating tools, and CI fails if the published tool annotations drift from the table above. Treat those annotations as hints; the host is still responsible for user approval and policy.

If you're running this on a shared workstation, set `ydotoold`'s socket permissions to `0600` (the default) and audit which processes on your user can `connect()` to it.

## Troubleshooting

To receive an explicit completion cue, start the MCP server with
`COMPUTER_USE_LINUX_NOTIFY_ON_COMPLETE=1`. This exposes `complete_interaction`,
a parameter-free tool the agent calls once after finishing its desktop work.
It submits a notification through `notify-send` with a two-second execution
limit and bounded process cleanup. Missing services, errors, or timeouts return
`cue: "skipped"`; notification settings may suppress a submitted cue. This does
not reserve the desktop or prove that other clients have stopped sending input.
No sound or additional desktop settings are enabled by this option.
This option currently applies only to directly spawned MCP hosts. The native Pi
extension does not forward the flag or include this optional tool in its catalog.

`setup` and `setup_accessibility` write and read back GNOME's
`org.gnome.desktop.interface toolkit-accessibility` setting, even if AT-SPI
is already enabled at runtime. A runtime-only success is reported as a warning:
newly launched GTK apps may still have no tree. Enabling the saved key may
require restarting target apps. Other accessibility tools can change that key
later; setup does not continuously override user settings.

For an explicit foreground guard while using desktop automation, run:

```bash
computer-use-linux guard-accessibility
```

The guard registers a passive AT-SPI window-activation listener and watches
GNOME's saved `toolkit-accessibility` key. It re-enables the key after a reset
and verifies it by readback, with periodic checks as well as change monitoring.
This affects all applications using the current user's GNOME setting, not just
the target app. It does not start a screen reader or change focus.

`mcp`, `setup`, `setup_accessibility`, and `get_app_state` never start this guard.
Stop it with Ctrl-C or SIGTERM **before** intentionally disabling accessibility.
Stopping ends setting writes and removes its listener without disabling other
accessibility clients or restoring an old saved value. A reset and reassertion
are not atomic: an app launched in that interval may still need restarting.
This is an opt-in mitigation, not a guarantee that every GNOME toggle sequence
preserves application accessibility trees.

`computer-use-linux doctor` is the source of truth. Common failure modes and fixes:

- **`accessibility.at_spi_bus.ok = false`** — AT-SPI registry isn't running or the toolkit bridge is off. Fix: `computer-use-linux setup` (or call the `setup_accessibility` MCP tool). Restart the apps you want to drive.
- **`windowing.gnome_shell_introspect.ok = false` and `gnome_shell_extension_dbus.ok = false`** — GNOME blocks introspection and the extension isn't installed. Fix: `computer-use-linux setup-window-targeting`, then log out and log back in.
- **`input.ydotool_socket.ok = false` while ydotool is the selected fallback** — daemon isn't running. On systemd, run `systemctl --user enable --now ydotoold`. On other init systems, rerun `./install.sh` and configure your per-user supervisor with the command it prints. If `ydotool` is not packaged for your distro, use another input backend or install a compatible ydotool release manually.
- **`input.ydotool.ok = false` with an unsupported CLI message** — install ydotool 1.0.3 or newer. A running daemon or socket alone is not enough; `doctor` verifies the raw key, wheel, stdin typing, and absolute-movement command family before advertising the backend.
- **`input.uinput.ok = false`** — `/dev/uinput` isn't accessible to your user. Fix: add yourself to the `input` group (`sudo usermod -aG input $USER`) and re-login. On distros that ship `uinput` as a kernel module without auto-loading it, add `uinput` to `/etc/modules-load.d/`. Direct uinput supplies absolute pointer input only, so `doctor` also requires a keyboard-capable portal, xdotool, or ydotool backend.
- **Portal calls hang or time out** — `xdg-desktop-portal` or its backend (`-gnome`, `-gtk`, `-kde`, `-wlr`) crashed. Fix: check `journalctl --user -u xdg-desktop-portal -u xdg-desktop-portal-gnome --since '5 min ago'` and restart the relevant unit.
- **KWin / Hyprland / niri / i3 / COSMIC / X11 windowing is unavailable** — check `doctor.windowing.backends`. KWin needs session-bus scripting; Hyprland needs `hyprctl`; niri needs the session's `NIRI_SOCKET`, or an unambiguous matching `niri.*.sock` in `XDG_RUNTIME_DIR`; i3 needs `i3-msg` and its IPC socket; generic X11 needs `wmctrl` and `xprop`. COSMIC needs `computer-use-linux-cosmic`, which the standard installers provide automatically; if you copied binaries by hand, copy the helper too or set `COMPUTER_USE_LINUX_COSMIC_HELPER`.
- **niri windows list correctly but targeted screenshots or relative clicks fail** — niri may omit window positions, leaving `bounds.x`/`bounds.y` as `null`. Unknown or mixed output scaling also leaves bounds unavailable. Focus the window first (`activate_window`) and use full-screen coordinates after checking a fresh screenshot.
- **Screenshots return black frames on multi-monitor setups** — known portal / compositor edge case. Use `get_app_state` with `include_screenshot: false` and rely on AT-SPI until the portal backend is healthy.
- **`type_text` types into the wrong window** — pass an explicit target (`window_id`, `pid`, `wm_class`, `title`, or for terminals `tty` / `terminal_pid` / `terminal_command` / `terminal_cwd`). Without a target, input goes to whatever window currently has compositor focus.
- **Wayland pointer actions miss an unfocused window** — injected pointer input is subject to the compositor's input-focus rules. Call `activate_window` for the target before `click`, `drag`, or coordinate `scroll`; a pointer can land at the requested coordinate without the unfocused surface receiving the action.

If `doctor` is green and a specific tool still misbehaves, file an issue with the JSON output of `doctor` and the failing tool's request payload.

For coordinate calibration, launch [the GTK4 probe](examples/coordinate_probe.py)
in your test desktop and take a targeted screenshot. Choose the center of its
green 10x10 square from that screenshot, convert by `scale`, and click relative
to the same target window. The probe's delivered-event `hit: true` is the
acceptance condition. Do not pass its widget-local `(85, 85)` directly as a
window-relative click: margins and decorations belong to different spaces.

[The semantic-click regression](scripts/semantic_click_test.py) runs against a
built binary in an isolated graphical display and checks actual GTK3 button
activation through MCP at scales 1 and 2. This does not establish correctness
of Mutter 46 EWMH move/resize, all mixed-output layouts, or GNOME 50.4 pointer
clicks.

Native Wayland qualification on GNOME 50.1 at 133.3% display scale used the
same green target for both modes: crop-relative `(122, 145)` and desktop
`(2140, 193)` both delivered widget coordinates approximately `(84.81, 84.78)`
with `hit: true` through uinput. These are one capture's measured coordinates,
not reusable offsets. Raw surface coordinates must still be transformed before
comparing them with a widget-local target.

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

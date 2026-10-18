# @agent-sh/computer-use-linux

Native Linux desktop observation and control for AI agents — published as
[`@agent-sh/computer-use-linux`](https://www.npmjs.com/package/@agent-sh/computer-use-linux).

Provides:
- **`computer-use-linux` CLI & stdio MCP server** (`computer-use-linux mcp`)
- **Native Pi coding agent extension** (`computer_use_linux_*` tools)
- **Bundled helpers**: `computer-use-linux-cosmic` (COSMIC window targeting) and `computer-use-linux-indicator` (on-screen agent activity overlay)

> **Security note**: this server can control the local Linux desktop. Tools such as
> `click`, `type_text`, `press_key`, `perform_action`, and `set_value` are
> mutating and can change real application state. The MCP tool list includes
> `ToolAnnotations` so hosts can distinguish read-only observation from mutating
> desktop actions.

## Quick start

### 1. CLI & MCP server

```bash
npm install -g @agent-sh/computer-use-linux
computer-use-linux doctor
```

Configure in any MCP client:

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

### 2. Pi coding agent (native tools)

```bash
pi install npm:@agent-sh/computer-use-linux
```

When installed through Pi, the package supplies native, dynamically loaded
`computer_use_linux_*` tools. No separate MCP adapter or manual MCP
configuration is required. Native tools require Pi 0.84.4 or newer; the
standalone CLI wrapper retains Node.js 18 support.

In Pi codemode, native tools return `{ text, images, isError }`. Pass image
blocks directly to `image(result.images[0])` and check `isError` before using
a result.

### 3. Claude Code & Codex CLI plugins

```bash
# Claude Code
claude plugin marketplace add agent-sh/computer-use-linux
claude plugin install computer-use-linux@computer-use-linux

# Codex CLI
codex plugin marketplace add agent-sh/computer-use-linux
codex plugin add computer-use-linux@computer-use-linux
```

The plugin registers the MCP server and caches private release binaries on first
start (it does not put `computer-use-linux` on `PATH`; also run step 1 if you
want the shell CLI).

### 4. Hermes Agent

```bash
npm install -g @agent-sh/computer-use-linux
hermes skills tap add agent-sh/computer-use-linux
hermes skills install agent-sh/computer-use-linux/computer-use-linux
hermes mcp add computer-use-linux --command computer-use-linux --args mcp
hermes mcp test computer-use-linux
hermes mcp configure computer-use-linux
```

The generated Hermes config should look like this:

```yaml
mcp_servers:
  computer-use-linux:
    command: computer-use-linux
    args: ["mcp"]
    timeout: 120
    connect_timeout: 30
```

## What the package installs

The package downloads the matching Linux `x86_64` or `aarch64` binary from the
GitHub release for this package version and verifies the `.sha256` asset before
installing it. It also installs the matching `computer-use-linux-cosmic` helper
used for COSMIC desktop window targeting and the `computer-use-linux-indicator`
overlay that shows agent activity on screen (`COMPUTER_USE_LINUX_INDICATOR=0`
turns it off).

If you already built or installed the binary yourself, set
`COMPUTER_USE_LINUX_BIN=/path/to/computer-use-linux` to make the wrapper use
that executable instead.

## Post-action observation & completion cue

Mutating tools (`click`, `drag`, `scroll`, `press_key`, `type_text`,
`perform_action`, `set_value`, `activate_window`, `move_window`, and
`resize_window`) accept `screenshot_after: true | "screen" | "window"`,
`format`/`quality`/`scale`/`max_width`/`max_height`/`max_bytes`, and
`include_tree: true` to return a post-action screenshot and/or compacted AT-SPI
tree in the same tool result.

For an optional MCP completion notification, set
`COMPUTER_USE_LINUX_NOTIFY_ON_COMPLETE=1` in the server environment and have the
agent call `complete_interaction` after its desktop work. `notify-send` must be
installed with an available desktop notification service. The cue is best effort
and does not provide exclusive desktop ownership.
This cue is available only to directly spawned MCP hosts; the native Pi extension
does not yet forward the flag or include the tool in its catalog.

## Desktop setup & compositor notes

- **Accessibility setup**: If accessibility is disabled, run `computer-use-linux setup`. Setup writes and
  reads back GNOME's `toolkit-accessibility` setting and warns if only runtime
  accessibility is available. Restart target apps if their trees remain empty.
- **Foreground accessibility guard**: To explicitly keep that saved setting enabled during desktop automation, run
  `computer-use-linux guard-accessibility` in a foreground terminal. It registers
  a passive AT-SPI window-activation listener and reasserts the current user's
  GNOME `toolkit-accessibility` key after resets, verifying each write by readback.
  This affects other apps using the same user setting. MCP, setup, and
  `get_app_state` never start it automatically. Stop with Ctrl-C or SIGTERM before
  disabling accessibility; stopping removes its listener and ends writes without
  disabling other clients or restoring an old value. An app launched during a
  reset may still need restarting, so this is not a complete GNOME toggle fix.
- **GNOME Wayland portal persistence**: On GNOME Wayland the remote-control portal asks again for every new server
  process. Set `COMPUTER_USE_LINUX_PERSIST_REMOTE_DESKTOP=1` in that process's
  environment to reuse a restore token instead. The first dialog still appears.
  Leave it unset to keep the prompt. The repository README records where the
  token is stored and who can read it.
- **niri**: On niri, export the session's `NIRI_SOCKET` to the server. Window listing and
  exact focus use `niri msg` with a direct IPC fallback. Missing positions or
  unknown/mixed output scaling leave window-relative coordinates unavailable;
  focus the target and inspect a fresh full-screen screenshot before using
  desktop coordinates.

## Input & coordinate contracts

- **Click vs. semantic actions**: Plain left `click` by element index or selector prefers a native AT-SPI
  `click`, `press`, or `toggle` action over toolkit bounds,
  avoiding pointer-coordinate conversion when that action is available. Explicit
  `activate`/`jump` requests belong in `perform_action`, not `click`, even when
  bounds are missing. Explicit
  `x`/`y`, right clicks, and multi-clicks retain pointer semantics. For coordinate
  `click` or `scroll` with `relative: true`, use the clipped target-window
  screenshot crop origin and divide preview coordinates by screenshot `scale`.
  Do not use widget-local or raw GDK surface coordinates. A window target is
  required.
- **X11 layout risk**: xdotool/XTEST can silently substitute symbols under non-US XKB layouts (reported with Brazilian `br`/ABNT2) or changing keymap state, even with `ok: true`. Successful xdotool text dispatches include a layout warning. Inspect the actual text before pressing Enter, submitting it, or treating it as delivered. Use `include_tree: true` when AT-SPI exposes the text, or `screenshot_after: true`; neither automatically compares the field with the request. Do not retry blindly.
- **X11 text input**: X11 text input sends LF and CRLF line endings as Return. Focused terminals
  accept keyboard input even when AT-SPI does not expose EditableText. Focus
  feedback describes the target; re-observe its contents to confirm input.
- **Hyprland `type_text`**: On Hyprland, `type_text` passes one literal wtype argument after `--` for a
  classified nonsecret field, avoiding stdin keymap-refresh corruption in Electron.
  Argument text is visible in process listings, so confirmed password roles are
  rejected. Terminals and ambiguous roles (`Entry`/`Text`, `text box`, unreadable
  roles), or a missing/incomplete/timed-out probe, use private stdin for at most
  100 Unicode characters per call. This includes terminals with AT-SPI support;
  they can still contain password prompts. Larger ambiguous-field text must be
  split into smaller calls or, for a nonterminal field, accessibility enabled.
  The input probe has its own 1.5-second budget; the 250 ms indicator probe only
  masks text. Both protections apply with the indicator disabled. Other compatible
  compositors retain their existing stdin path. Wtype rejects NUL and text over
  131,071 UTF-8 bytes before focusing a window or displaying text. `ok: true`
  confirms dispatch only; every text backend leaves field contents unverified.
  Re-observe before retrying. Exact window focus alone does not suppress missing-focus
  warnings: only a complete scoped tree containing window/menu chrome without
  content makes the feedback informational. Such trees get a conditional
  Electron/Chromium hint computed before compaction; this is not positive Electron
  detection.

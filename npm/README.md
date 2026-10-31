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

On labwc, window targeting requires the Selkies IPC patch and its `labwc.sock`; stock labwc does not expose this socket. Activation requires a unique app-id/title match. Window IDs include the title and must be refreshed after it changes. Ambiguous windows are rejected. Window bounds and relative coordinate targeting are unavailable until output scaling can be verified.

Native X11 clicks, scrolls and drags use xdotool/XTEST on `DISPLAY`, ahead of uinput and portal input. Install xdotool for this route. Missing or failed commands return an error without a system-wide fallback. Drags send paced intermediate motion and always attempt release after a failed press or motion; verify the drop target afterward.

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
- **COSMIC Wayland**: Pointer coordinates use per-monitor RemoteDesktop portal streams. Approve the consent dialog before continuing. Available EIS connections wait for device readiness before the first input event. Window bounds support targeted screenshots and relative input, and window IDs fit JavaScript's exact integer range. The toplevel protocol does not expose PID.
- **Window IDs**: Every backend's `window_id` fits JavaScript's exact integer range (at most 2^53-1), including KWin, so JSON clients can pass it back unchanged. A larger value is rejected with a prompt to re-run `list_windows`.
- **Portal keyboard focus**: The receiving window or app target is preserved before consent and its focus is restored and verified afterward. If verification fails, no text or key chord is sent. Untargeted portal keyboard calls require working window introspection. GNOME Introspect can only verify an already focused app; refocus it manually after consent if needed.
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

- **Element targets**: `click`, `scroll`, `perform_action`, and `set_value`
  accept an optional app target (`pid`, `window_id`, `app_id`, `wm_class`,
  `window_title`). With one, an `element_index` or selector from another app's
  `get_app_state` snapshot is refused. When the window backend reports no pid
  for the target window (i3, sway, COSMIC), the snapshot must have been taken
  for that window. `perform_action` and `set_value` do not focus the window;
  an `element_identifier` is refused only when its bus owner is known and is
  not the target pid.
- **Click vs. semantic actions**: Plain left `click` by element index or selector prefers a native AT-SPI
  `click`, `press`, or `toggle` action over toolkit bounds,
  avoiding pointer-coordinate conversion when that action is available. Explicit
  `activate`/`jump` requests belong in `perform_action`, not `click`, even when
  bounds are missing. Explicit
  `x`/`y`, right clicks, and multi-clicks retain pointer semantics. Explicit
  coordinates default to raw screenshot pixels (`coordinate_space: "capture"`).
  On KDE Wayland, `coordinate_space: "atspi"` with `element_index` or a semantic
  selector converts a global AT-SPI screen point through the element's KWin window,
  decoration, output origin, and display scale, even with a client-local AT-SPI
  window frame. It requires both `x`/`y` and element identity
  and cannot be combined with `relative: true`. For coordinate
  `click` or `scroll` with `relative: true`, use the clipped target-window
  screenshot crop origin and divide preview coordinates by screenshot `scale`.
  Do not use widget-local or raw GDK surface coordinates. A window target is
  required.
- **Selkies containers**: set `PIXELFLUX_CU` and `PIXELFLUX_CU_TOKEN` so screenshots and input use the container's pixelflux computer-use server. No portal prompt. A closed port falls through; a server error does not.
- **X11 layout risk**: xdotool/XTEST can silently substitute symbols under non-US XKB layouts (reported with Brazilian `br`/ABNT2) or changing keymap state, even with `ok: true`. Successful xdotool text dispatches include a layout warning. XKB layout replacement events, persistent mapping/group changes, and keyboard focus/grab transitions detected across a call return an error after dispatch, without replaying the text. Keyboard grabs can divert characters without changing the polled focused-window ID. If monitoring cannot start, xdotool still dispatches and the response explicitly warns that layout-change and keyboard interception detection were unavailable. This also detects a `setxkbmap` layout changed and restored during one call. Temporary raw remaps or group switches restored within a call and dropped text with a stable map remain unverified. Inspect the actual text before pressing Enter, submitting it, or treating it as delivered. Use `include_tree: true` when AT-SPI exposes the text, or `screenshot_after: true`; neither automatically compares the field with the request. Do not retry blindly.
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

  Chromium can mark a `document web` focused alongside its contenteditable
  editor. The focus probe keeps searching that document and reports the editor;
  a document that holds focus itself still gets a warning, which no longer
  claims the text was lost. An editor can carry the `editable` state without the
  EditableText interface (`supports_editable_text: false`): `type_text` can
  reach it, `set_value` cannot. Compact nodes leave a false
  `supports_editable_text` out. Verify its text or a screenshot before
  submitting or retrying. Embedded browser documents can use up an app-scoped
  snapshot's node budget before the app's own controls; when
  `accessibility_tree_truncated` is true, a missing editor proves nothing. Raise
  `max_nodes` (default 1000, hard cap 10000) and, if needed, `max_depth` (hard
  cap 64); a busy renderer may give only a few thousand nodes before the
  deadline. A snapshot stops reading at 10 seconds
  and returns what it read, marked truncated; if the editor is still missing,
  hide the embedded browser pane for the snapshot.

  When a result would pass 32,000 bytes, the tree comes back as its leading
  nodes plus `accessibility_tree_spill` (`path`, `node_count`, `bytes`,
  `inline_node_count`). `path` is a private JSON file holding the whole tree
  with the same `element_index` values (under `$XDG_RUNTIME_DIR` when it is
  private, otherwise `~/.cache`). Search it instead of requesting the tree
  again; hosts that cannot read files page through it with `get_tree_page`. The
  file lasts until the next snapshot or until the server exits.

  Tree nodes use a compact encoding: null, empty, zero and false fields are
  left out, containers do not carry the U+FFFC placeholder text Chromium gives
  them, and `actions` lists action names. `verbose: true` returns the full
  encoding with action descriptions and keybindings. `interactive_only: true`
  returns only nodes with an action, editable text or a value, plus headings,
  status and live regions, windows and dialogs, with containers collapsed and
  the same `element_index` values as the full tree. When `title` or
  `window_id` names one of several windows under one AT-SPI application root,
  as in Chromium, the tree covers only that window.

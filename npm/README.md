# computer-use-linux

NPM wrapper for the `computer-use-linux` MCP server, published as
[`@agent-sh/computer-use-linux`](https://www.npmjs.com/package/@agent-sh/computer-use-linux).

Security note: this server can control the local Linux desktop. Tools such as
`click`, `type_text`, `press_key`, `perform_action`, and `set_value` are
mutating and can change real application state. The MCP tool list includes
`ToolAnnotations` so hosts can distinguish read-only observation from mutating
desktop actions.

Plain left `click` by element index or selector prefers a native AT-SPI
`click`, `press`, or `toggle` action over toolkit bounds,
avoiding pointer-coordinate conversion when that action is available. Explicit
`activate`/`jump` requests belong in `perform_action`, not `click`, even when
bounds are missing. Explicit
`x`/`y`, right clicks, and multi-clicks retain pointer semantics. For coordinate
`click` or `scroll` with `relative: true`, use the clipped target-window
screenshot crop origin and divide preview coordinates by screenshot `scale`.
Do not use widget-local or raw GDK surface coordinates. A window target is
required.

```bash
npm install -g @agent-sh/computer-use-linux
computer-use-linux doctor
pi install npm:@agent-sh/computer-use-linux
hermes skills tap add agent-sh/computer-use-linux
hermes skills install agent-sh/computer-use-linux/computer-use-linux
hermes mcp add computer-use-linux --command computer-use-linux --args mcp
hermes mcp test computer-use-linux
hermes mcp configure computer-use-linux
```

For an optional MCP completion notification, set
`COMPUTER_USE_LINUX_NOTIFY_ON_COMPLETE=1` in the server environment and have the
agent call `complete_interaction` after its desktop work. `notify-send` must be
installed with an available desktop notification service. The cue is best effort
and does not provide exclusive desktop ownership.
This cue is available only to directly spawned MCP hosts; the native Pi extension
does not yet forward the flag or include the tool in its catalog.

If accessibility is disabled, run `computer-use-linux setup`. Setup writes and
reads back GNOME's `toolkit-accessibility` setting and warns if only runtime
accessibility is available. Restart target apps if their trees remain empty.

To explicitly keep that saved setting enabled during desktop automation, run
`computer-use-linux guard-accessibility` in a foreground terminal. It registers
a passive AT-SPI window-activation listener and reasserts the current user's
GNOME `toolkit-accessibility` key after resets, verifying each write by readback.
This affects other apps using the same user setting. MCP, setup, and
`get_app_state` never start it automatically. Stop with Ctrl-C or SIGTERM before
disabling accessibility; stopping removes its listener and ends writes without
disabling other clients or restoring an old value. An app launched during a
reset may still need restarting, so this is not a complete GNOME toggle fix.

The generated Hermes config should look like this:

```yaml
mcp_servers:
  computer-use-linux:
    command: computer-use-linux
    args: ["mcp"]
    timeout: 120
    connect_timeout: 30
```

The package downloads the matching Linux x86_64 or aarch64 binary from the
GitHub release for this package version and verifies the `.sha256` asset before
installing it. It also installs the matching `computer-use-linux-cosmic` helper
used for COSMIC desktop window targeting.

When installed through Pi, the package supplies native, dynamically loaded
`computer_use_linux_*` tools. No separate MCP adapter or manual MCP
configuration is required. Native tools require Pi 0.84.4 or newer; the
standalone CLI wrapper retains Node.js 18 support.

If you already built or installed the binary yourself, set
`COMPUTER_USE_LINUX_BIN=/path/to/computer-use-linux` to make the wrapper use
that executable instead.

---
name: pi-setup
description: "Pi coding agent setup for native computer-use-linux tools."
---

# Pi Setup

## Install

Install one package:

```bash
pi install npm:@agent-sh/computer-use-linux
```

Restart Pi or run `/reload`. No separate MCP adapter or MCP configuration is
required.

The integration is designed for current Pi releases with additive dynamic tool
loading. Update Pi and installed packages when needed:

```bash
pi update --all
```

## How tool loading works

Pi initially sees one small loader:

```text
computer_use_linux_tools
```

Enable exact tools:

```text
computer_use_linux_tools({ tools: ["doctor", "list_windows"] })
```

Or search by capability:

```text
computer_use_linux_tools({ query: "inspect a window and type text" })
```

The selected native tools appear starting on the next model turn with their
full upstream schemas and remain active for the session:

```text
computer_use_linux_doctor({})
computer_use_linux_list_windows({})
computer_use_linux_type_text({ text: "hello", title: "Notes" })
```

Pi does not start the desktop process when the loader runs. The first real tool
call starts one computer-use-linux process, and the package reuses it for the
session so `get_app_state` element indices and portal sessions remain valid.

## Safe operating loop

1. Enable `get_app_state`, `list_windows`, and any likely action tools.
2. Call `computer_use_linux_get_app_state`, using
   `include_screenshot: false` when accessibility data is enough.
3. Inspect the returned readiness block; enable/call `doctor` only for full
   diagnostics.
4. Identify the target with `computer_use_linux_list_windows` or
   `computer_use_linux_focused_window`.
5. Enable and call the required action tool.
6. Re-observe after the UI changes.

Native Computer Use tools execute sequentially. Cancellation is forwarded to
the MCP request. If the server process exits, the failed call is never replayed
automatically; call `get_app_state` again before another element-based action.

## Migration from the adapter-based package

Older releases required `pi-mcp-adapter` and wrote a
`computer-use-linux` entry into `mcp.json` under the configured Pi agent
directory.

The native integration does not write MCP configuration. It reads only the
legacy entry location to show a migration notice. After updating:

1. Remove only the `computer-use-linux` entry from the Pi agent `mcp.json` if
   it is still present.
2. Keep `pi-mcp-adapter` if you use it for other MCP servers; otherwise remove
   it with `pi remove npm:pi-mcp-adapter`.
3. Run `/reload`.

The package reports a non-destructive migration notice when it detects the
legacy entry.

## If the binary is not found

The extension looks for `computer-use-linux` in this order:

1. `COMPUTER_USE_LINUX_BIN`
2. The binary downloaded inside the installed npm package

Reinstall the package if the bundled binary is missing:

```bash
pi remove npm:@agent-sh/computer-use-linux
pi install npm:@agent-sh/computer-use-linux
```

Users with a separate build can set:

```bash
export COMPUTER_USE_LINUX_BIN=/absolute/path/to/computer-use-linux
```

## Verification

Enable and call the readiness tool:

```text
computer_use_linux_tools({ tools: ["doctor"] })
computer_use_linux_doctor({})
```

Ready output has:

- `can_register_mcp_tools: true`
- `can_build_accessibility_tree: true`
- `can_query_windows: true`
- `can_send_development_input: true`
- `blockers: []`

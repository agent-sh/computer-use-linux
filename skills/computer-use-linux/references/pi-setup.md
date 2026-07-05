---
name: pi-setup
description: "Pi coding agent setup for the computer-use-linux MCP server."
---

# Pi Setup

## Prerequisites

You need both of these installed:

```bash
pi install npm:pi-mcp-adapter
pi install npm:@agent-sh/computer-use-linux
```

> **Note:** `pi-mcp-adapter` is a separate extension that provides MCP protocol support in pi. The `@agent-sh/computer-use-linux` package auto-registers the desktop MCP server into pi-mcp-adapter's config.

After installing both, restart pi or run `/reload`.

## Verify

The `mcp()` proxy tool should now be available. Call it from any prompt:

```bash
mcp({ search: "doctor" })
```

Or start with the readiness check (note the server-prefixed tool name):

```bash
mcp({ tool: "computer_use_linux_doctor" })
```

Search for available tools:

```bash
mcp({ server: "computer-use-linux" })
```

## If the binary is not found

The extension looks for `computer-use-linux` in this order:

1. `COMPUTER_USE_LINUX_BIN` environment variable
2. The npm-bundled binary (from `npm install -g @agent-sh/computer-use-linux`)
3. `$PATH`

If none are found, install it:

```bash
npm install -g @agent-sh/computer-use-linux
# or
cargo install computer-use-linux
```

Then restart pi or run `/reload`.

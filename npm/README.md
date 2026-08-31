# computer-use-linux

NPM wrapper for the `computer-use-linux` MCP server, published as
[`@agent-sh/computer-use-linux`](https://www.npmjs.com/package/@agent-sh/computer-use-linux).

Security note: this server can control the local Linux desktop. Tools such as
`click`, `type_text`, `press_key`, `perform_action`, and `set_value` are
mutating and can change real application state. The MCP tool list includes
`ToolAnnotations` so hosts can distinguish read-only observation from mutating
desktop actions.

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

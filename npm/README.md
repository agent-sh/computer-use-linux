# @agent-sh/computer-use-linux

Native Linux desktop observation and control for AI agents, published as
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

This page covers the npm install. Tool contracts, compositor notes, environment
variables and troubleshooting are in the
[repository README](https://github.com/agent-sh/computer-use-linux#readme),
which this package also ships as `README.md`. Agents should read
`skills/computer-use-linux/SKILL.md`.

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

If accessibility is disabled, run `computer-use-linux setup`.

### 2. Pi coding agent (native tools)

```bash
pi install npm:@agent-sh/computer-use-linux
```

Pi gets native `computer_use_linux_*` tools with no MCP configuration. They need
Pi 0.84.4 or newer; the standalone CLI wrapper keeps Node.js 18 support. Each
call times out after 60 seconds; set `COMPUTER_USE_LINUX_TIMEOUT_MS` in Pi's
environment to change it. See
[Pi Coding Agent](https://github.com/agent-sh/computer-use-linux#pi-coding-agent)
for codemode results.

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
start. It does not put `computer-use-linux` on `PATH`; run step 1 as well if you
want the shell CLI.

### 4. Hermes Agent

```bash
npm install -g @agent-sh/computer-use-linux
hermes skills tap add agent-sh/computer-use-linux
hermes skills install agent-sh/computer-use-linux/computer-use-linux
hermes mcp add computer-use-linux --command computer-use-linux --args mcp
hermes mcp test computer-use-linux
```

See [Hermes Agent](https://github.com/agent-sh/computer-use-linux#hermes-agent)
for tool subsets and timeouts.

### 5. GitHub Copilot CLI

After step 1, register the server from your logged-in desktop session:

```bash
copilot mcp add computer-use-linux \
  --env COMPUTER_USE_LINUX_ENABLE_SHELL=0 \
  -- computer-use-linux mcp
copilot mcp get computer-use-linux
```

See [GitHub Copilot CLI](https://github.com/agent-sh/computer-use-linux#github-copilot-cli)
before first use.

## What the package installs

The package downloads the matching Linux `x86_64` or `aarch64` binary from the
GitHub release for this package version and verifies the `.sha256` asset before
installing it. It also installs the matching `computer-use-linux-cosmic` helper
used for COSMIC desktop window targeting and the `computer-use-linux-indicator`
overlay that shows agent activity on screen (`COMPUTER_USE_LINUX_INDICATOR=0`
turns it off). When npm skips the install script (npm 12 does for scripts it was
not told to allow), the wrapper runs the same download on first launch, with its
output on stderr. A package directory that cannot be written is refused.

If you already built or installed the binary yourself, set
`COMPUTER_USE_LINUX_BIN=/path/to/computer-use-linux` to make the wrapper use
that executable instead. The Pi extension uses it only when it is an absolute
path to an executable.

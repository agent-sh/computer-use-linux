---
name: hermes-setup
description: "Hermes agent setup for the computer-use-linux MCP server."
---

# Hermes Setup

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
```

If the binary is not on `PATH`, pass the absolute path to `--command`.

Hermes registers tools using the `mcp_<server>_<tool>` pattern. With this config, tool names are prefixed as `mcp_computer_use_linux_`, for example:

| MCP tool | Hermes tool name |
| --- | --- |
| `doctor` | `mcp_computer_use_linux_doctor` |
| `get_app_state` | `mcp_computer_use_linux_get_app_state` |
| `list_windows` | `mcp_computer_use_linux_list_windows` |
| `click` | `mcp_computer_use_linux_click` |
| `type_text` | `mcp_computer_use_linux_type_text` |

Restart Hermes after changing MCP config.

/**
 * Pi coding agent extension for computer-use-linux.
 *
 * Discovers the computer-use-linux binary and registers it as an MCP server
 * for pi-mcp-adapter by writing to ~/.pi/agent/mcp.json.
 *
 * Prerequisite: pi-mcp-adapter (npm:pi-mcp-adapter) must be installed.
 */

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { env } from "node:process";

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const PACKAGE_NAME = "@agent-sh/computer-use-linux";
const MCP_SERVER_NAME = "computer-use-linux";

/**
 * Path to pi's agent-level MCP config. pi-mcp-adapter reads from this file
 * when it starts up, alongside ~/.config/mcp/mcp.json and .mcp.json.
 */
function getPiAgentMcpConfigPath(): string {
	const configured = env.PI_CODING_AGENT_DIR?.trim();
	if (configured) {
		return join(configured, "mcp.json");
	}
	return join(homedir(), ".pi", "agent", "mcp.json");
}

// ---------------------------------------------------------------------------
// Binary discovery
// ---------------------------------------------------------------------------

/**
 * Find the computer-use-linux binary. Cached after first successful lookup.
 */
let _binaryPath: string | null | undefined;

function findBinary(): string | null {
	if (_binaryPath !== undefined) return _binaryPath;

	// 1) Environment variable override
	const fromEnv = env.COMPUTER_USE_LINUX_BIN;
	if (fromEnv && existsSync(fromEnv)) {
		_binaryPath = fromEnv;
		return fromEnv;
	}

	// 2) Bundled by the npm wrapper's postinstall
	//    The npm wrapper downloads binaries to npm/bin/ next to the JS wrapper.
	//    We derive the path relative to our own installed location.
	const candidates = [
		// Bundled binary for this platform
		join(
			__dirname,
			"..",
			"..",
			"npm",
			"bin",
			`computer-use-linux-linux-${process.arch}`,
		),
		// Direct npm global install
		join(__dirname, "..", "..", "npm", "bin", "computer-use-linux"),
	];
	for (const candidate of candidates) {
		const resolved = join(candidate);
		if (existsSync(resolved)) {
			_binaryPath = resolved;
			return resolved;
		}
	}

	// 3) PATH
	const pathDirs = (env.PATH || "").split(":");
	for (const dir of pathDirs) {
		const candidate = join(dir, "computer-use-linux");
		try {
			const stat = existsSync(candidate);
			if (stat) {
				_binaryPath = candidate;
				return candidate;
			}
		} catch {
			// Skip inaccessible directories
		}
	}

	_binaryPath = null;
	return null;
}

// ---------------------------------------------------------------------------
// MCP config management
// ---------------------------------------------------------------------------

interface McpServerEntry {
	command: string;
	args: string[];
	lifecycle?: string;
}

interface McpConfig {
	mcpServers?: Record<string, McpServerEntry>;
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function readMcpConfig(path: string): McpConfig {
	try {
		const raw = readFileSync(path, "utf-8");
		const parsed = JSON.parse(raw);
		if (!isRecord(parsed)) return {};
		return parsed as McpConfig;
	} catch {
		return {};
	}
}

function writeMcpConfig(path: string, config: McpConfig): void {
	try {
		mkdirSync(dirname(path), { recursive: true });
		writeFileSync(path, JSON.stringify(config, null, 2) + "\n", "utf-8");
	} catch (error) {
		console.error(
			`${PACKAGE_NAME}: failed to write MCP config to ${path}:`,
			error instanceof Error ? error.message : String(error),
		);
	}
}

function ensureServerEntry(configPath: string, binaryPath: string): boolean {
	const config = readMcpConfig(configPath);
	const servers = config.mcpServers ?? {};
	const existing = servers[MCP_SERVER_NAME];

	// Check if entry already exists and matches
	if (
		existing &&
		existing.command === binaryPath &&
		existing.args?.length === 1 &&
		existing.args[0] === "mcp"
	) {
		return false; // No change needed
	}

	// Add or update the entry
	servers[MCP_SERVER_NAME] = {
		command: binaryPath,
		args: ["mcp"],
	};

	writeMcpConfig(configPath, { ...config, mcpServers: servers });
	return true; // Config was updated
}

// ---------------------------------------------------------------------------
// Extension entry point
// ---------------------------------------------------------------------------

export default function (pi: ExtensionAPI) {
	// Resolve binary path eagerly (before session_start) so it's cached.
	const binaryPath = findBinary();

	pi.on("session_start", async (_event, ctx) => {
		if (!binaryPath) {
			ctx.ui.notify?.(
				`${PACKAGE_NAME}: computer-use-linux binary not found. ` +
					"Install it with 'npm install -g @agent-sh/computer-use-linux' " +
					"or set COMPUTER_USE_LINUX_BIN.",
				"warning",
			);
			return;
		}

		// Write/update the MCP config that pi-mcp-adapter reads
		const configPath = getPiAgentMcpConfigPath();
		const changed = ensureServerEntry(configPath, binaryPath);

		if (changed && ctx.hasUI) {
			ctx.ui.notify?.(
				`${PACKAGE_NAME}: MCP server configured at ${configPath}. ` +
					"Run /reload if pi-mcp-adapter is already installed.",
				"info",
			);
		}

		// Check that pi-mcp-adapter is available (its tools are registered)
		if (!pi.getAllTools().some((t) => t.name === "mcp")) {
			ctx.ui.notify?.(
				`${PACKAGE_NAME}: pi-mcp-adapter not detected. ` +
					"Install it with 'pi install npm:pi-mcp-adapter' then /reload.",
				"warning",
			);
		}
	});
}

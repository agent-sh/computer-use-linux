/**
 * Native Pi integration for computer-use-linux.
 *
 * Pi starts with one small loader tool. The real Computer Use tools remain
 * inactive until the loader enables them, then call one session-scoped MCP
 * process with their exact upstream schemas.
 */

import type {
	AgentToolResult,
	ExtensionAPI,
	ExtensionContext,
	ToolResultEvent,
} from "@earendil-works/pi-coding-agent";
import { StringEnum } from "@earendil-works/pi-ai";
import {
	accessSync,
	constants,
	existsSync,
	readFileSync,
} from "node:fs";
import { createRequire } from "node:module";
import { homedir } from "node:os";
import { join } from "node:path";
import { Type, type TSchema } from "typebox";
import {
	GENERATED_MCP_TOOLS,
	GENERATED_OPTIONAL_MCP_TOOLS,
	GENERATED_SERVER_VERSION,
	GENERATED_SHELL_TOOL_CATALOG_HASH,
	GENERATED_TOOL_CATALOG_HASH,
	type GeneratedMcpToolDefinition,
} from "./generated-tools.ts";

const require = createRequire(import.meta.url);
const PACKAGE_NAME = "@agent-sh/computer-use-linux";
const ACTIVE_TOOLS_ENTRY = "computer-use-linux-active-tools";
const LOADER_TOOL_NAME = "computer_use_linux_tools";
const TOOL_PREFIX = "computer_use_linux_";
const DEFAULT_TOOLS = [
	"doctor",
	"list_windows",
	"focused_window",
	"get_app_state",
] as const;
const MAX_RESULT_TEXT_CHARS = 200_000;
const MAX_RESULT_IMAGE_BYTES = 2 * 1024 * 1024;
const MAX_RESULT_IMAGES = 4;
const TRUNCATION_NOTICE =
	"[Result truncated by the Pi extension. Request a smaller/bounded result.]";
const SHELL_ENABLED = process.env.COMPUTER_USE_LINUX_ENABLE_SHELL === "1";
const AVAILABLE_MCP_TOOLS: readonly GeneratedMcpToolDefinition[] = SHELL_ENABLED
	? [...GENERATED_MCP_TOOLS, ...GENERATED_OPTIONAL_MCP_TOOLS]
	: GENERATED_MCP_TOOLS;

type PiContent = AgentToolResult<Record<string, unknown>>["content"][number];

interface McpCallToolResult {
	content?: unknown[];
	isError?: boolean;
	structuredContent?: unknown;
}

interface NativeMcpClient {
	callTool(
		name: string,
		args: Record<string, unknown>,
		signal?: AbortSignal,
	): Promise<McpCallToolResult>;
	close(): Promise<void>;
}

interface NativeMcpClientConstructor {
	new (options: {
		binaryPath: string;
		binaryArgs?: string[];
		clientVersion: string;
		env: Record<string, string>;
		expectedCatalogHash: string;
		expectedServerVersion: string;
		requestTimeoutMs?: number;
	}): NativeMcpClient;
}

interface NativeMcpClientModule {
	ComputerUseMcpClient: NativeMcpClientConstructor;
}

interface BinaryLaunch {
	binaryPath: string;
	env: Record<string, string>;
}

interface ExtensionDependencies {
	findBinary?: () => BinaryLaunch | null;
	loadClientModule?: () => NativeMcpClientModule;
}

const ORIGINAL_TOOL_NAMES = AVAILABLE_MCP_TOOLS.map((tool) => tool.name);
const NATIVE_TOOL_NAMES = AVAILABLE_MCP_TOOLS.map(
	(tool) => `${TOOL_PREFIX}${tool.name}`,
);
const NATIVE_TOOL_NAME_SET = new Set(NATIVE_TOOL_NAMES);
const TOOL_BY_ORIGINAL_NAME: Map<string, GeneratedMcpToolDefinition> = new Map(
	AVAILABLE_MCP_TOOLS.map((tool) => [tool.name, tool]),
);
const TOOL_ALIASES: Record<string, string[]> = {
	activate_window: ["focus", "raise", "switch window"],
	click: ["press button", "mouse"],
	doctor: ["diagnose", "readiness", "health", "setup"],
	drag: ["mouse drag"],
	focused_window: ["active window", "focus"],
	get_app_state: ["accessibility", "observe", "snapshot", "screen state"],
	list_apps: ["applications", "processes"],
	list_windows: ["windows", "titles"],
	move_window: ["position window"],
	perform_action: ["activate element", "press element", "toggle"],
	press_key: ["keyboard", "shortcut", "hotkey"],
	resize_window: ["window size"],
	run_shell: ["shell", "command", "terminal"],
	screenshot: ["image", "screen capture"],
	scroll: ["wheel", "page"],
	set_value: ["input value", "text field", "slider"],
	setup_accessibility: ["at-spi", "accessibility setup"],
	setup_window_targeting: ["gnome extension", "window setup"],
	type_text: ["write text", "keyboard text"],
};

const LoaderToolNameSchema = StringEnum(ORIGINAL_TOOL_NAMES);
const LoaderParameters = Type.Object(
	{
		tools: Type.Optional(
			Type.Array(LoaderToolNameSchema, {
				description:
					"Exact Computer Use tool names to enable starting next turn for this session.",
				minItems: 1,
				uniqueItems: true,
			}),
		),
		query: Type.Optional(
			Type.String({
				description:
					"Capability to search for when exact tool names are not known.",
				minLength: 1,
			}),
		),
	},
	{ additionalProperties: false },
);

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function executable(path: string): boolean {
	try {
		accessSync(path, constants.X_OK);
		return true;
	} catch {
		return false;
	}
}

function runtimeEnvironment(): Record<string, string> {
	const allowed = new Set([
		"PATH",
		"HOME",
		"USER",
		"LOGNAME",
		"SHELL",
		"LANG",
		"LANGUAGE",
		"TERM",
		"TMPDIR",
		"DISPLAY",
		"WAYLAND_DISPLAY",
		"XAUTHORITY",
		"DBUS_SESSION_BUS_ADDRESS",
		"XDG_RUNTIME_DIR",
		"XDG_CURRENT_DESKTOP",
		"XDG_SESSION_TYPE",
		"XDG_SESSION_DESKTOP",
		"XDG_DATA_DIRS",
		"XDG_CONFIG_HOME",
		"XDG_CACHE_HOME",
		"XDG_STATE_HOME",
		"DESKTOP_SESSION",
		"GDMSESSION",
		"HYPRLAND_INSTANCE_SIGNATURE",
		"I3SOCK",
		"SWAYSOCK",
		"YDOTOOL_SOCKET",
		"GSETTINGS_SCHEMA_DIR",
		"GIO_EXTRA_MODULES",
		"GI_TYPELIB_PATH",
		"LD_LIBRARY_PATH",
		"NIX_LD",
		"NIX_LD_LIBRARY_PATH",
		"RUST_LOG",
		"COMPUTER_USE_LINUX_COSMIC_HELPER",
		"COMPUTER_USE_LINUX_ENABLE_SHELL",
		"COMPUTER_USE_LINUX_FORCE_PORTAL_KEYBOARD",
		"COMPUTER_USE_LINUX_FORCE_PORTAL_POINTER",
		"COMPUTER_USE_LINUX_FORCE_XDOTOOL_KEYBOARD",
		"COMPUTER_USE_LINUX_FORCE_YDOTOOL_KEYBOARD",
		"COMPUTER_USE_LINUX_FORCE_YDOTOOL_POINTER",
		"COMPUTER_USE_LINUX_PORTAL_SCROLL_INVERT",
		"COMPUTER_USE_LINUX_SCREENSHOT_BACKEND",
		"CU_DISABLE_ABS_POINTER",
	]);
	return Object.fromEntries(
		Object.entries(process.env).filter(
			(entry): entry is [string, string] =>
				typeof entry[1] === "string" &&
				(allowed.has(entry[0]) || entry[0].startsWith("LC_")),
		),
	);
}

function defaultFindBinary(): BinaryLaunch | null {
	const env = runtimeEnvironment();
	const override = process.env.COMPUTER_USE_LINUX_BIN?.trim();
	if (override && executable(override)) {
		return { binaryPath: override, env };
	}

	const bundledBinary = join(
		__dirname,
		"..",
		"..",
		"npm",
		"bin",
		`computer-use-linux-${process.platform}-${process.arch}`,
	);
	if (executable(bundledBinary)) {
		const cosmicHelper = join(
			__dirname,
			"..",
			"..",
			"npm",
			"bin",
			"computer-use-linux-cosmic",
		);
		if (!env.COMPUTER_USE_LINUX_COSMIC_HELPER && executable(cosmicHelper)) {
			env.COMPUTER_USE_LINUX_COSMIC_HELPER = cosmicHelper;
		}
		return { binaryPath: bundledBinary, env };
	}

	return null;
}

function defaultLoadClientModule(): NativeMcpClientModule {
	return require("./mcp-client.bundle.cjs") as NativeMcpClientModule;
}

function requestTimeoutMs(): number {
	const value = Number(process.env.COMPUTER_USE_LINUX_TIMEOUT_MS ?? "60000");
	return Number.isFinite(value) && value > 0 ? value : 60_000;
}

function toolParameters(schema: Record<string, unknown>): TSchema {
	const unsafe = (
		Type as unknown as {
			Unsafe?: (value: Record<string, unknown>) => TSchema;
		}
	).Unsafe;
	return unsafe ? unsafe(schema) : (schema as TSchema);
}

function nativeToolDescription(tool: GeneratedMcpToolDefinition): string {
	if (tool.annotations.destructiveHint === true) {
		return (
			`${tool.description} This tool can trigger actions in the live desktop; ` +
			"obtain user approval before submitting, deleting, sending, purchasing, or overwriting."
		);
	}
	if (tool.annotations.readOnlyHint === false) {
		return `${tool.description} This tool changes local desktop state.`;
	}
	return tool.description;
}

function tokens(value: string): string[] {
	return value.toLowerCase().split(/[^a-z0-9]+/).filter(Boolean);
}

function searchTools(query: string): GeneratedMcpToolDefinition[] {
	const normalized = query.trim().toLowerCase();
	const queryTokens = tokens(normalized);
	return AVAILABLE_MCP_TOOLS.map((tool) => {
		const name = tool.name.toLowerCase();
		const description = tool.description.toLowerCase();
		const aliases = (TOOL_ALIASES[tool.name] ?? []).join(" ").toLowerCase();
		let score = 0;
		if (name === normalized) score += 100;
		if (name.includes(normalized)) score += 40;
		for (const token of queryTokens) {
			if (name.includes(token)) score += 12;
			if (aliases.includes(token)) score += 8;
			if (description.includes(token)) score += 3;
		}
		return { tool, score };
	})
		.filter(({ score }) => score > 0)
		.sort(
			(left, right) =>
				right.score - left.score ||
				left.tool.name.localeCompare(right.tool.name),
		)
		.slice(0, 8)
		.map(({ tool }) => tool);
}

function selectTools(
	tools: string[] | undefined,
	query: string | undefined,
): GeneratedMcpToolDefinition[] {
	const selected = new Map<string, GeneratedMcpToolDefinition>();
	for (const name of tools ?? []) {
		const tool = TOOL_BY_ORIGINAL_NAME.get(name);
		if (tool) selected.set(name, tool);
	}
	if (query?.trim()) {
		for (const tool of searchTools(query)) {
			selected.set(tool.name, tool);
		}
	}
	if (selected.size === 0 && !query?.trim() && !(tools?.length)) {
		for (const name of DEFAULT_TOOLS) {
			const tool = TOOL_BY_ORIGINAL_NAME.get(name);
			if (tool) selected.set(name, tool);
		}
	}
	return [...selected.values()];
}

function renderJson(value: unknown): string {
	try {
		return JSON.stringify(value, null, 2);
	} catch {
		return String(value);
	}
}

function estimatedBase64Bytes(data: string): number {
	const padding = data.endsWith("==") ? 2 : data.endsWith("=") ? 1 : 0;
	return Math.max(0, Math.floor((data.length * 3) / 4) - padding);
}

function convertMcpResult(result: McpCallToolResult): PiContent[] {
	const converted: PiContent[] = [];
	let remainingText = MAX_RESULT_TEXT_CHARS - TRUNCATION_NOTICE.length;
	let imageBytes = 0;
	let imageCount = 0;
	let truncated = false;
	const appendText = (text: string) => {
		if (remainingText <= 0) {
			if (text.length > 0) truncated = true;
			return;
		}
		const keep = Math.min(text.length, remainingText);
		if (keep > 0) {
			converted.push({ type: "text", text: text.slice(0, keep) });
			remainingText -= keep;
		}
		if (keep < text.length) truncated = true;
	};
	const appendImage = (data: string, mimeType: string) => {
		const bytes = estimatedBase64Bytes(data);
		if (
			imageCount >= MAX_RESULT_IMAGES ||
			imageBytes + bytes > MAX_RESULT_IMAGE_BYTES
		) {
			truncated = true;
			return;
		}
		converted.push({ type: "image", data, mimeType });
		imageBytes += bytes;
		imageCount += 1;
	};

	for (const block of result.content ?? []) {
		if (!isRecord(block) || typeof block.type !== "string") {
			appendText(renderJson(block));
			continue;
		}
		if (block.type === "text" && typeof block.text === "string") {
			appendText(block.text);
			continue;
		}
		if (
			block.type === "image" &&
			typeof block.data === "string" &&
			typeof block.mimeType === "string"
		) {
			appendImage(block.data, block.mimeType);
			continue;
		}
		if (block.type === "resource" && isRecord(block.resource)) {
			const resource = block.resource;
			if (typeof resource.text === "string") {
				appendText(resource.text);
				continue;
			}
			if (
				typeof resource.blob === "string" &&
				typeof resource.mimeType === "string" &&
				resource.mimeType.startsWith("image/")
			) {
				appendImage(resource.blob, resource.mimeType);
				continue;
			}
		}
		appendText(renderJson(block));
	}
	if (converted.length === 0 && result.structuredContent !== undefined) {
		appendText(renderJson(result.structuredContent));
	}
	if (truncated) {
		converted.push({ type: "text", text: TRUNCATION_NOTICE });
	}
	if (converted.length === 0) {
		converted.push({ type: "text", text: "(empty result)" });
	}
	return converted;
}

function legacyConfigPath(): string {
	const agentDir =
		process.env.PI_CODING_AGENT_DIR?.trim() || join(homedir(), ".pi", "agent");
	return join(agentDir, "mcp.json");
}

function hasLegacyRegistration(): boolean {
	const path = legacyConfigPath();
	if (!existsSync(path)) return false;
	try {
		const config = JSON.parse(readFileSync(path, "utf8"));
		return (
			isRecord(config) &&
			isRecord(config.mcpServers) &&
			isRecord(config.mcpServers["computer-use-linux"])
		);
	} catch {
		return false;
	}
}

function isNativeToolResult(
	event: ToolResultEvent,
): event is ToolResultEvent & {
	details: Record<string, unknown> & { computerUseLinux: true };
} {
	return (
		event.toolName.startsWith(TOOL_PREFIX) &&
		isRecord(event.details) &&
		event.details.computerUseLinux === true
	);
}

function restoredNativeTools(ctx: ExtensionContext): string[] {
	const entries = ctx.sessionManager.getBranch();
	for (let index = entries.length - 1; index >= 0; index -= 1) {
		const entry = entries[index];
		if (
			entry?.type !== "custom" ||
			entry.customType !== ACTIVE_TOOLS_ENTRY ||
			!isRecord(entry.data) ||
			!Array.isArray(entry.data.tools)
		) {
			continue;
		}
		return entry.data.tools.filter(
			(name): name is string =>
				typeof name === "string" && NATIVE_TOOL_NAME_SET.has(name),
		);
	}
	return [];
}

export function createComputerUseLinuxExtension(
	dependencies: ExtensionDependencies = {},
) {
	const findBinary = dependencies.findBinary ?? defaultFindBinary;
	const loadClientModule =
		dependencies.loadClientModule ?? defaultLoadClientModule;

	return function computerUseLinuxExtension(pi: ExtensionAPI) {
		let launch: BinaryLaunch | undefined;
		let client: NativeMcpClient | undefined;
		let enabledNativeTools = new Set<string>();

		const resolveLaunch = () => {
			if (launch) return launch;
			const resolved = findBinary();
			if (resolved) launch = resolved;
			return resolved;
		};

		const getClient = () => {
			if (client) return client;
			const resolved = resolveLaunch();
			if (!resolved) {
				throw new Error(
					"computer-use-linux binary was not found. Reinstall " +
						`${PACKAGE_NAME} or set COMPUTER_USE_LINUX_BIN.`,
				);
			}
			const { ComputerUseMcpClient } = loadClientModule();
			client = new ComputerUseMcpClient({
				binaryPath: resolved.binaryPath,
				clientVersion: GENERATED_SERVER_VERSION,
				env: resolved.env,
				expectedCatalogHash: SHELL_ENABLED
					? GENERATED_SHELL_TOOL_CATALOG_HASH
					: GENERATED_TOOL_CATALOG_HASH,
				expectedServerVersion: GENERATED_SERVER_VERSION,
				requestTimeoutMs: requestTimeoutMs(),
			});
			return client;
		};

		for (const tool of AVAILABLE_MCP_TOOLS) {
			const nativeName = `${TOOL_PREFIX}${tool.name}`;
			pi.registerTool({
				name: nativeName,
				label: `Computer Use: ${tool.name}`,
				description: nativeToolDescription(tool),
				parameters: toolParameters(tool.inputSchema),
				executionMode: "sequential",
				async execute(_toolCallId, params, signal) {
					const result = await getClient().callTool(
						tool.name,
						params as Record<string, unknown>,
						signal,
					);
					return {
						content: convertMcpResult(result),
						details: {
							computerUseLinux: true,
							mcpIsError: result.isError === true,
							tool: tool.name,
						},
					};
				},
			});
		}

		pi.registerTool({
			name: LOADER_TOOL_NAME,
			label: "Computer Use Tools",
			description:
				"Enable native Linux desktop observation/control tools starting next turn for this session. " +
				"Choose exact tool names or search by capability; no desktop process starts until an enabled tool is called.",
			promptSnippet:
				"Enable Linux desktop tools only when the task needs local GUI observation or control",
			promptGuidelines: [
				"Use computer_use_linux_tools before attempting local Linux GUI observation or control.",
				"After enabling Computer Use tools, begin with computer_use_linux_get_app_state; use computer_use_linux_list_windows or computer_use_linux_focused_window before targeted keyboard input, and re-observe after the UI changes.",
			],
			parameters: LoaderParameters,
			async execute(_toolCallId, params) {
				const selected = selectTools(params.tools, params.query);
				if (selected.length === 0) {
					return {
						content: [
							{
								type: "text",
								text:
									"No matching Computer Use tools. Available names: " +
									ORIGINAL_TOOL_NAMES.join(", "),
							},
						],
						details: { added: [], matches: [] },
					};
				}

				const active = pi.getActiveTools();
				const additions = selected
					.map((tool) => `${TOOL_PREFIX}${tool.name}`)
					.filter((name) => !active.includes(name));
				for (const tool of selected) {
					enabledNativeTools.add(`${TOOL_PREFIX}${tool.name}`);
				}
				if (additions.length > 0) {
					pi.setActiveTools([...active, ...additions]);
				}
				pi.appendEntry(ACTIVE_TOOLS_ENTRY, {
					tools: [...enabledNativeTools].sort(),
				});
				const lines = selected.map(
					(tool) => `- ${tool.name}: ${tool.description}`,
				);
				return {
					content: [
						{
							type: "text",
							text:
								`Enabled ${selected.length} Computer Use tool(s) starting next turn for this session:\n` +
								lines.join("\n"),
						},
					],
					details: {
						added: additions,
						matches: selected.map((tool) => tool.name),
					},
				};
			},
		});

		pi.on("session_start", (event, ctx) => {
			enabledNativeTools = new Set(restoredNativeTools(ctx));
			const active = pi
				.getActiveTools()
				.filter((name) => !NATIVE_TOOL_NAME_SET.has(name));
			active.push(...enabledNativeTools);
			if (!active.includes(LOADER_TOOL_NAME)) {
				active.push(LOADER_TOOL_NAME);
			}
			pi.setActiveTools(active);

			if (!resolveLaunch() && ctx.hasUI) {
				ctx.ui.notify(
					`${PACKAGE_NAME}: binary not found; reinstall the package or set COMPUTER_USE_LINUX_BIN.`,
					"warning",
				);
			}
			if (hasLegacyRegistration() && ctx.hasUI) {
				ctx.ui.notify(
					`${PACKAGE_NAME}: native Pi tools are active. A legacy computer-use-linux entry still exists in ${legacyConfigPath()}; remove that entry if it is no longer needed by pi-mcp-adapter.`,
					"info",
				);
			}
		});

		pi.on("tool_result", (event) => {
			if (
				isNativeToolResult(event) &&
				event.details.mcpIsError === true
			) {
				return { isError: true };
			}
		});

		pi.on("session_shutdown", async () => {
			const current = client;
			client = undefined;
			await current?.close();
		});

	};
}

export default createComputerUseLinuxExtension();

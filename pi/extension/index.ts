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
	statSync,
} from "node:fs";
import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import { homedir } from "node:os";
import { delimiter, isAbsolute, join } from "node:path";
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
	"get_tree_page",
] as const;
const MAX_RESULT_TEXT_CHARS = 200_000;
const MAX_RESULT_IMAGE_BYTES = 2 * 1024 * 1024;
const MAX_RESULT_IMAGES = 4;
const SAFE_CHILD_PATH = ["/usr/local/bin", "/usr/bin", "/bin"].join(delimiter);
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
	/** Replaces the whole lookup, the first-use download included. */
	findBinary?: () => BinaryLaunch | null;
	loadClientModule?: () => NativeMcpClientModule;
	/** Directory holding npm/bin and npm/install.js. Defaults to this package. */
	packageRoot?: string;
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
	get_tree_page: ["tree page", "more nodes", "spilled tree"],
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
		// A directory on PATH also passes X_OK; only a regular file can be spawned.
		if (!statSync(path).isFile()) return false;
		accessSync(path, constants.X_OK);
		return true;
	} catch {
		return false;
	}
}

export function sanitizeExecutablePath(pathValue: string | undefined): string {
	return (pathValue ?? "")
		.split(delimiter)
		.filter((dir) => dir.length > 0 && isAbsolute(dir))
		.join(delimiter);
}

/**
 * First executable `name` in a PATH-style list, or `null`. Empty and relative
 * segments are skipped rather than read relative to the current directory.
 */
export function findExecutableOnPath(
	pathValue: string | undefined,
	name: string,
	isExecutable: (path: string) => boolean = executable,
): string | null {
	for (const dir of sanitizeExecutablePath(pathValue).split(delimiter)) {
		if (!dir) continue;
		const candidate = join(dir, name);
		if (isExecutable(candidate)) return candidate;
	}
	return null;
}

/**
 * Variables the server child receives, plus every LC_ variable. Everything
 * else in Pi's environment stays out of the desktop process.
 * scripts/check_env_forwarding.py fails CI when the server reads a variable
 * missing here. COMPUTER_USE_LINUX_NOTIFY_ON_COMPLETE is left out: it adds a
 * tool, and the client refuses a catalog that differs from the generated one.
 */
const FORWARDED_ENVIRONMENT = [
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
	"NIRI_SOCKET",
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
	"COMPUTER_USE_LINUX_AGENT_NAME",
	"COMPUTER_USE_LINUX_COSMIC_HELPER",
	"COMPUTER_USE_LINUX_ENABLE_SHELL",
	"COMPUTER_USE_LINUX_FORCE_PORTAL_KEYBOARD",
	"COMPUTER_USE_LINUX_FORCE_PORTAL_POINTER",
	"COMPUTER_USE_LINUX_FORCE_XDOTOOL_KEYBOARD",
	"COMPUTER_USE_LINUX_FORCE_YDOTOOL_KEYBOARD",
	"COMPUTER_USE_LINUX_FORCE_YDOTOOL_POINTER",
	"COMPUTER_USE_LINUX_HYPRLAND_BACKGROUND",
	"COMPUTER_USE_LINUX_INDICATOR",
	"COMPUTER_USE_LINUX_INDICATOR_BIN",
	"COMPUTER_USE_LINUX_INDICATOR_HIDE_TEXT",
	"COMPUTER_USE_LINUX_PERSIST_REMOTE_DESKTOP",
	"COMPUTER_USE_LINUX_PORTAL_SCROLL_INVERT",
	"COMPUTER_USE_LINUX_SCREENSHOT_BACKEND",
	"COMPUTER_USE_LINUX_XDOTOOL_TYPE_DELAY_MS",
	"CU_DISABLE_ABS_POINTER",
	"PIXELFLUX_CU",
	"PIXELFLUX_CU_TOKEN",
] as const;

function runtimeEnvironment(): Record<string, string> {
	const allowed = new Set<string>(FORWARDED_ENVIRONMENT);
	const env = Object.fromEntries(
		Object.entries(process.env).filter(
			(entry): entry is [string, string] =>
				typeof entry[1] === "string" &&
				(allowed.has(entry[0]) || entry[0].startsWith("LC_")),
		),
	);
	const sanitizedPath = sanitizeExecutablePath(env.PATH);
	env.PATH = sanitizedPath || SAFE_CHILD_PATH;
	return env;
}

/** Directory holding npm/bin and npm/install.js. */
const PACKAGE_ROOT = join(__dirname, "..", "..");
const INSTALL_OUTPUT_CHARS = 4000;

function installerPath(packageRoot: string): string {
	return join(packageRoot, "npm", "install.js");
}

/** COMPUTER_USE_LINUX_BIN, then the package's own binary. */
function localLaunch(packageRoot: string): BinaryLaunch | null {
	const env = runtimeEnvironment();
	const override = process.env.COMPUTER_USE_LINUX_BIN?.trim();
	// Relative overrides would be resolved from Pi's potentially untrusted
	// project directory. Ignore them and continue with bundled/global discovery.
	if (override && isAbsolute(override) && executable(override)) {
		return { binaryPath: override, env };
	}

	const binDir = join(packageRoot, "npm", "bin");
	const bundledBinary = join(
		binDir,
		`computer-use-linux-${process.platform}-${process.arch}`,
	);
	if (!executable(bundledBinary)) return null;
	const cosmicHelper = join(binDir, "computer-use-linux-cosmic");
	if (!env.COMPUTER_USE_LINUX_COSMIC_HELPER && executable(cosmicHelper)) {
		env.COMPUTER_USE_LINUX_COSMIC_HELPER = cosmicHelper;
	}
	const indicator = join(binDir, "computer-use-linux-indicator");
	if (!env.COMPUTER_USE_LINUX_INDICATOR_BIN && executable(indicator)) {
		env.COMPUTER_USE_LINUX_INDICATOR_BIN = indicator;
	}
	return { binaryPath: bundledBinary, env };
}

/**
 * A global npm or cargo install. The MCP client still checks the server
 * version and tool catalog, so a mismatched install fails with that reason.
 */
function pathLaunch(): BinaryLaunch | null {
	const onPath = findExecutableOnPath(process.env.PATH, "computer-use-linux");
	return onPath ? { binaryPath: onPath, env: runtimeEnvironment() } : null;
}

/**
 * Runs npm/install.js, the first-use download the CLI wrapper also runs.
 * npm 12 skips install scripts it was not told to allow, so `pi install` can
 * stage the package without its binary. The installer checks each download's
 * sha256 and moves the main binary into place last. Its output is kept for
 * the error and never reaches Pi's terminal.
 */
function runInstaller(packageRoot: string): Promise<void> {
	const installer = installerPath(packageRoot);
	// Under Bun, execPath is the Bun or Pi executable, not node.
	const node = process.versions.bun
		? findExecutableOnPath(process.env.PATH, "node")
		: process.execPath;
	if (!node) {
		return Promise.reject(
			new Error(`node was not found on PATH to run ${installer}`),
		);
	}
	return new Promise((resolve, reject) => {
		let output = "";
		const keep = (chunk: Buffer) => {
			output = (output + chunk.toString()).slice(-INSTALL_OUTPUT_CHARS);
		};
		const child = spawn(node, [installer], {
			cwd: packageRoot,
			stdio: ["ignore", "pipe", "pipe"],
		});
		child.stdout.on("data", keep);
		child.stderr.on("data", keep);
		child.on("error", reject);
		child.on("close", (code, signal) => {
			if (code === 0) {
				resolve();
				return;
			}
			reject(
				new Error(
					`${installer} exited with ${signal ?? `status ${code}`}: ${output.trim()}`,
				),
			);
		});
	});
}

/** Settles with `promise`, or rejects with the signal's reason once it aborts. */
function untilAborted<T>(promise: Promise<T>, signal?: AbortSignal): Promise<T> {
	if (!signal) return promise;
	return new Promise((resolve, reject) => {
		const onAbort = () => reject(signal.reason);
		signal.addEventListener("abort", onAbort, { once: true });
		promise.then(
			(value) => {
				signal.removeEventListener("abort", onAbort);
				resolve(value);
			},
			(error) => {
				signal.removeEventListener("abort", onAbort);
				reject(error);
			},
		);
	});
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

/**
 * Machine-readable result for codemode scripts.
 *
 * Pi's codemode sandbox only passes `structuredContent` to nested calls from
 * tools that declare `outputSchema`; otherwise scripts receive text only and
 * image blocks are dropped. Mirror the `read` tool pattern so screenshots and
 * other image results stay reachable via `image()`.
 */
const NativeToolOutputSchema = Type.Object({
	text: Type.String({
		description: "Joined text blocks of the converted MCP result.",
	}),
	images: Type.Array(
		Type.Object({
			type: Type.Literal("image"),
			data: Type.String({ description: "Base64 image data." }),
			mimeType: Type.String({ description: "Image MIME type." }),
		}),
		{ description: "Image blocks of the converted MCP result." },
	),
	isError: Type.Boolean({ description: "Whether the MCP tool reported a failure." }),
});

function toNativeToolStructuredContent(content: PiContent[], isError: boolean): {
	text: string;
	images: Array<{ type: "image"; data: string; mimeType: string }>;
	isError: boolean;
} {
	const texts: string[] = [];
	const images: Array<{ type: "image"; data: string; mimeType: string }> = [];
	for (const block of content) {
		if (block.type === "text") {
			texts.push(block.text);
		} else if (block.type === "image") {
			images.push({ type: "image", data: block.data, mimeType: block.mimeType });
		}
	}
	return { text: texts.join("\n"), images, isError };
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
	const packageRoot = dependencies.packageRoot ?? PACKAGE_ROOT;
	const findLocal =
		dependencies.findBinary ?? (() => localLaunch(packageRoot));
	const findOnPath = dependencies.findBinary ? () => null : pathLaunch;
	const canInstall = () =>
		!dependencies.findBinary && existsSync(installerPath(packageRoot));
	const loadClientModule =
		dependencies.loadClientModule ?? defaultLoadClientModule;

	return function computerUseLinuxExtension(pi: ExtensionAPI) {
		let launch: BinaryLaunch | undefined;
		let client: NativeMcpClient | undefined;
		let enabledNativeTools = new Set<string>();
		let install: Promise<string | undefined> | undefined;
		let installFailure: string | undefined;
		// True while the launch is a PATH binary taken because the package
		// binary could not be downloaded. It may not match this extension.
		let launchIsFallback = false;
		// Bumped by session_shutdown, so a download that outlives the session
		// never starts a server nothing would close.
		let session = 0;

		// The package's binary comes before PATH, as after an install whose
		// script ran: it matches this extension's catalog.
		const resolveLaunch = async () => {
			if (launch) return launch;
			let resolved = findLocal();
			if (!resolved && canInstall()) {
				install ??= runInstaller(packageRoot).then(
					() => undefined,
					(error: Error) => error.message,
				);
				const pending = install;
				installFailure = await pending;
				if (install === pending) install = undefined;
				resolved = findLocal();
			}
			launchIsFallback = false;
			if (!resolved) {
				resolved = findOnPath();
				launchIsFallback = resolved !== null && installFailure !== undefined;
			}
			if (resolved) launch = resolved;
			return resolved;
		};

		const dropClient = async () => {
			const current = client;
			client = undefined;
			launch = undefined;
			launchIsFallback = false;
			await current?.close();
		};

		const getClient = async () => {
			const started = session;
			// The package binary appeared since the fallback was chosen.
			if (launchIsFallback && findLocal()) await dropClient();
			if (client) return client;
			const resolved = await resolveLaunch();
			if (session !== started) {
				throw new Error("Pi session ended while computer-use-linux was being installed");
			}
			if (client) return client;
			if (!resolved) {
				const failure = installFailure
					? ` Downloading it failed: ${installFailure}`
					: "";
				throw new Error(
					`computer-use-linux binary was not found.${failure} Reinstall ` +
						`${PACKAGE_NAME} with its install script allowed, install computer-use-linux on PATH, or set COMPUTER_USE_LINUX_BIN.`,
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

		const callTool = async (
			name: string,
			args: Record<string, unknown>,
			signal: AbortSignal | undefined,
		) => {
			// A cancelled call never starts a download. A call cancelled while
			// one runs stops waiting; the download goes on for the next call.
			signal?.throwIfAborted();
			const active = await untilAborted(getClient(), signal);
			try {
				return await active.callTool(name, args, signal);
			} catch (error) {
				// A PATH binary taken after a failed download can be the wrong
				// version. Drop it so the next call tries the download again.
				if (signal?.aborted || !launchIsFallback || client !== active) throw error;
				const failure = installFailure;
				await dropClient();
				throw new Error(
					`${(error instanceof Error ? error.message : String(error)).replace(/\.$/, "")}. ` +
						`This computer-use-linux came from PATH because downloading the package binary failed: ${failure}. ` +
						"The next call tries the download again.",
					{ cause: error },
				);
			}
		};

		for (const tool of AVAILABLE_MCP_TOOLS) {
			const nativeName = `${TOOL_PREFIX}${tool.name}`;
			pi.registerTool({
				name: nativeName,
				label: `Computer Use: ${tool.name}`,
				description: nativeToolDescription(tool),
				parameters: toolParameters(tool.inputSchema),
				outputSchema: NativeToolOutputSchema,
				executionMode: "sequential",
				async execute(_toolCallId, params, signal) {
					const result = await callTool(
						tool.name,
						params as Record<string, unknown>,
						signal,
					);
					const content = convertMcpResult(result);
					return {
						content,
						details: {
							computerUseLinux: true,
							mcpIsError: result.isError === true,
							tool: tool.name,
						},
						structuredContent: toNativeToolStructuredContent(content, result.isError === true),
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

			// Nothing is downloaded here; a missing package binary is fetched on
			// the first tool call.
			if (!findLocal() && !canInstall() && !findOnPath() && ctx.hasUI) {
				ctx.ui.notify(
					`${PACKAGE_NAME}: binary not found; reinstall the package, install computer-use-linux on PATH, or set COMPUTER_USE_LINUX_BIN.`,
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
			session += 1;
			const current = client;
			client = undefined;
			await current?.close();
		});

	};
}

export default createComputerUseLinuxExtension();

import type {
	ExtensionAPI,
	ToolDefinition,
	ToolResultEvent,
} from "@earendil-works/pi-coding-agent";
import { createCodemodeExtension } from "@earendil-works/pi-coding-agent";
import {
	chmodSync,
	copyFileSync,
	existsSync,
	mkdirSync,
	mkdtempSync,
	rmSync,
	writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
	createComputerUseLinuxExtension,
	findExecutableOnPath,
	sanitizeExecutablePath,
} from "../extension/index.ts";
import { GENERATED_MCP_TOOLS } from "../extension/generated-tools.ts";

type Handler = (event: any, ctx: any) => unknown;

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");

class FakeMcpClient {
	static instances: FakeMcpClient[] = [];
	/** A binary path whose calls fail, as a mismatched server would. */
	static failingBinary: string | undefined;
	static result: {
		content?: unknown[];
		isError?: boolean;
		structuredContent?: unknown;
	} = { content: [{ type: "text", text: "ok" }] };

	readonly calls: Array<{
		name: string;
		args: Record<string, unknown>;
		signal: AbortSignal | undefined;
	}> = [];
	closed = 0;

	constructor(readonly options: Record<string, unknown>) {
		FakeMcpClient.instances.push(this);
	}

	async callTool(
		name: string,
		args: Record<string, unknown>,
		signal?: AbortSignal,
	) {
		this.calls.push({ name, args, signal });
		if (this.options.binaryPath === FakeMcpClient.failingBinary) {
			throw new Error("computer-use-linux version 0.0.1 does not match");
		}
		return FakeMcpClient.result;
	}

	async close() {
		this.closed += 1;
	}
}

function createPi() {
	const tools = new Map<string, ToolDefinition>();
	const handlers = new Map<string, Handler[]>();
	const entries: Array<{
		type: "custom";
		customType: string;
		data: unknown;
	}> = [];
	let activeTools = ["read", "bash"];
	const notify = vi.fn();
	const pi = {
		registerTool(tool: ToolDefinition) {
			tools.set(tool.name, tool);
			if (!activeTools.includes(tool.name)) activeTools.push(tool.name);
		},
		on(event: string, handler: Handler) {
			const entries = handlers.get(event) ?? [];
			entries.push(handler);
			handlers.set(event, entries);
		},
		getActiveTools() {
			return [...activeTools];
		},
		setActiveTools(names: string[]) {
			activeTools = [...names];
		},
		appendEntry(customType: string, data: unknown) {
			entries.push({ type: "custom", customType, data });
		},
		getAllTools() {
			return [...tools.values()].map((tool) => ({
				name: tool.name,
				description: tool.description,
				parameters: tool.parameters,
				sourceInfo: undefined,
			}));
		},
	} as unknown as ExtensionAPI;

	const emit = async (event: string, value: unknown = {}) => {
		let result: unknown;
		for (const handler of handlers.get(event) ?? []) {
			const next = await handler(value, {
				hasUI: true,
					sessionManager: {
						getEntries: () => [...entries],
						getBranch: () => [...entries],
					},
				ui: { notify },
			});
			if (next !== undefined) result = next;
		}
		return result;
	};

	return {
		activeTools: () => [...activeTools],
		emit,
		handlers,
		notify,
		pi,
		tools,
	};
}

describe("native Pi extension", () => {
	let agentDir: string;

	beforeEach(() => {
		agentDir = mkdtempSync(join(tmpdir(), "computer-use-linux-pi-test-"));
		vi.stubEnv("PI_CODING_AGENT_DIR", agentDir);
		FakeMcpClient.instances = [];
		FakeMcpClient.failingBinary = undefined;
		FakeMcpClient.result = { content: [{ type: "text", text: "ok" }] };
	});

	afterEach(() => {
		vi.unstubAllEnvs();
		if (agentDir && existsSync(agentDir)) {
			rmSync(agentDir, { recursive: true, force: true });
		}
	});

	function load() {
		const harness = createPi();
		createComputerUseLinuxExtension({
			findBinary: () => ({
				binaryPath: "/tmp/computer-use-linux",
				env: { PATH: "/usr/bin" },
			}),
			loadClientModule: () => ({
				ComputerUseMcpClient: FakeMcpClient as never,
			}),
		})(harness.pi);
		return harness;
	}

	it("registers exact native schemas but starts with only the loader active", async () => {
		const harness = load();

		expect(harness.tools.size).toBe(GENERATED_MCP_TOOLS.length + 1);
		expect(FakeMcpClient.instances).toHaveLength(0);

		await harness.emit("session_start");

		expect(harness.activeTools()).toEqual([
			"read",
			"bash",
			"computer_use_linux_tools",
		]);
		expect(FakeMcpClient.instances).toHaveLength(0);

		for (const tool of GENERATED_MCP_TOOLS) {
			const registered = harness.tools.get(`computer_use_linux_${tool.name}`);
			expect(registered?.description).toContain(tool.description);
			expect(registered?.executionMode).toBe("sequential");
			expect(registered?.parameters).toMatchObject(tool.inputSchema);
			expect(registered?.outputSchema).toBeDefined();
		}
		expect(
			harness.tools.get("computer_use_linux_click")?.description,
		).toContain("obtain user approval");
	});

	it("activates exact tools additively without starting the desktop process", async () => {
		const harness = load();
		await harness.emit("session_start");
		const loader = harness.tools.get("computer_use_linux_tools")!;

		const result = await loader.execute(
			"loader",
			{ tools: ["doctor", "get_app_state"] },
			undefined,
			undefined,
			{} as never,
		);

		expect(harness.activeTools()).toEqual([
			"read",
			"bash",
			"computer_use_linux_tools",
			"computer_use_linux_doctor",
			"computer_use_linux_get_app_state",
		]);
		expect(result.details).toMatchObject({
			matches: ["doctor", "get_app_state"],
		});
		expect(FakeMcpClient.instances).toHaveLength(0);
	});

	it("restores enabled tools from persisted branch state", async () => {
		const harness = load();
		await harness.emit("session_start", { reason: "startup" });
		const loader = harness.tools.get("computer_use_linux_tools")!;
		await loader.execute(
			"loader",
			{ tools: ["doctor"] },
			undefined,
			undefined,
			{} as never,
		);

		await harness.emit("session_start", { reason: "reload" });

		expect(harness.activeTools()).toContain("computer_use_linux_doctor");
	});

	it("finds tools by capability query", async () => {
		const harness = load();
		await harness.emit("session_start");
		const loader = harness.tools.get("computer_use_linux_tools")!;

		const result = await loader.execute(
			"loader",
			{ query: "screen capture" },
			undefined,
			undefined,
			{} as never,
		);

		expect(result.details).toMatchObject({
			matches: expect.arrayContaining(["screenshot"]),
		});
		expect(harness.activeTools()).toContain(
			"computer_use_linux_screenshot",
		);
	});

	it("reuses one client and preserves text and image results", async () => {
		const harness = load();
		await harness.emit("session_start");
		FakeMcpClient.result = {
			content: [
				{ type: "text", text: "done" },
				{ type: "image", data: "aGVsbG8=", mimeType: "image/png" },
			],
		};
		const doctor = harness.tools.get("computer_use_linux_doctor")!;

		const first = await doctor.execute(
			"one",
			{},
			undefined,
			undefined,
			{} as never,
		);
		const second = await doctor.execute(
			"two",
			{},
			undefined,
			undefined,
			{} as never,
		);

		expect(FakeMcpClient.instances).toHaveLength(1);
		expect(FakeMcpClient.instances[0]?.calls).toHaveLength(2);
		expect(first.content).toEqual([
			{ type: "text", text: "done" },
			{ type: "image", data: "aGVsbG8=", mimeType: "image/png" },
		]);
		expect(second.content).toEqual(first.content);
		expect(first.structuredContent).toEqual({
			text: "done",
			images: [{ type: "image", data: "aGVsbG8=", mimeType: "image/png" }],
			isError: false,
		});
		expect(second.structuredContent).toEqual(first.structuredContent);
	});

	it.each([
		{ type: "image", data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aENsAAAAASUVORK5CYII=", mimeType: "image/png" },
		{ type: "resource", resource: { blob: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aENsAAAAASUVORK5CYII=", mimeType: "image/png" } },
	])("preserves screenshot images through Pi's real codemode runtime: $type", async (imageBlock) => {
		const harness = load();
		await harness.emit("session_start");
		FakeMcpClient.result = {
			content: [{ type: "text", text: "screenshot" }, imageBlock],
		};
		createCodemodeExtension({ models: false, mode: "on", inlineBudget: 0 })(harness.pi);
		const screenshot = harness.tools.get("computer_use_linux_screenshot")!;
		const codemode = harness.tools.get("codemode")!;
		const result = await codemode.execute("script", {
			code: "const r = await tools.computer_use_linux_screenshot({}); text(r.text); image(r.images[0]);",
		}, undefined, undefined, {
			tools: [screenshot],
			sessionManager: { getBranch: () => [] },
			async executeTool(name: string, args: unknown) {
				expect(name).toBe(screenshot.name);
				return {
					toolCall: { id: "script/1" },
					isError: false,
					result: await screenshot.execute("script/1", args, undefined, undefined, {} as never),
				};
			},
		} as never);
		expect(result.content).toContainEqual({ type: "text", text: "screenshot" });
		expect(result.content).toContainEqual({ type: "image", data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aENsAAAAASUVORK5CYII=", mimeType: "image/png" });
	});

	it("marks MCP tool-level failures as Pi tool errors", async () => {
		const harness = load();
		await harness.emit("session_start");
		FakeMcpClient.result = {
			content: [{ type: "text", text: "failed" }],
			isError: true,
		};
		const doctor = harness.tools.get("computer_use_linux_doctor")!;
		const result = await doctor.execute(
			"one",
			{},
			undefined,
			undefined,
			{} as never,
		);

		const override = await harness.emit("tool_result", {
			type: "tool_result",
			toolName: "computer_use_linux_doctor",
			toolCallId: "one",
			input: {},
			content: result.content,
			details: result.details,
			isError: false,
		} satisfies Partial<ToolResultEvent>);

		expect(override).toEqual({ isError: true });
		expect(result.structuredContent).toEqual({ text: "failed", images: [], isError: true });
	});

	it("bounds aggregate text results and always reports omitted blocks", async () => {
		const harness = load();
		await harness.emit("session_start");
		FakeMcpClient.result = {
			content: [
				{ type: "text", text: "x".repeat(200_000) },
				{ type: "text", text: "omitted tail" },
			],
		};
		const doctor = harness.tools.get("computer_use_linux_doctor")!;
		const result = await doctor.execute(
			"one",
			{},
			undefined,
			undefined,
			{} as never,
		);
		const texts = result.content
			.filter((block) => block.type === "text")
			.map((block) => block.text);
		expect(texts.reduce((total, text) => total + text.length, 0)).toBeLessThanOrEqual(
			200_000,
		);
		expect(texts.at(-1)).toContain("Result truncated by the Pi extension");
		expect(texts.join("")).not.toContain("omitted tail");
	});

	it("bounds aggregate image count and reports omitted images", async () => {
		const harness = load();
		await harness.emit("session_start");
		FakeMcpClient.result = {
			content: Array.from({ length: 5 }, () => ({
				type: "image",
				data: "aGVsbG8=",
				mimeType: "image/png",
			})),
		};
		const screenshot = harness.tools.get("computer_use_linux_screenshot")!;
		const result = await screenshot.execute(
			"one",
			{},
			undefined,
			undefined,
			{} as never,
		);

		expect(result.content.filter((block) => block.type === "image")).toHaveLength(4);
		const structured = result.structuredContent as {
			text: string;
			images: Array<{ data: string; mimeType: string }>;
		};
		expect(structured.images).toHaveLength(4);
		expect(structured.text).toContain("Result truncated by the Pi extension");
		expect(
			result.content
				.filter((block) => block.type === "text")
				.map((block) => block.text)
				.join("\n"),
		).toContain("Result truncated by the Pi extension");
	});

	it("closes the session client exactly once", async () => {
		const harness = load();
		await harness.emit("session_start");
		const doctor = harness.tools.get("computer_use_linux_doctor")!;
		await doctor.execute(
			"one",
			{},
			undefined,
			undefined,
			{} as never,
		);

		await harness.emit("session_shutdown");
		await harness.emit("session_shutdown");

		expect(FakeMcpClient.instances[0]?.closed).toBe(1);
	});

	it("fails clearly when the binary is unavailable", async () => {
		const harness = createPi();
		createComputerUseLinuxExtension({
			findBinary: () => null,
			loadClientModule: () => ({
				ComputerUseMcpClient: FakeMcpClient as never,
			}),
		})(harness.pi);
		await harness.emit("session_start");

		const doctor = harness.tools.get("computer_use_linux_doctor")!;
		await expect(
			doctor.execute(
				"one",
				{},
				undefined,
				undefined,
				{} as never,
			),
		).rejects.toThrow("binary was not found");
		expect(harness.notify).toHaveBeenCalledWith(
			expect.stringContaining("binary not found"),
			"warning",
		);
	});

	it("passes only absolute PATH entries to the child process", async () => {
		const absoluteBin = join(agentDir, "bin");
		vi.stubEnv("COMPUTER_USE_LINUX_BIN", process.execPath);
		vi.stubEnv(
			"PATH",
			["", ".", "relative-bin", absoluteBin].join(delimiter),
		);
		const harness = createPi();
		createComputerUseLinuxExtension({
			// No npm/install.js here, so a missing binary never starts a download.
			packageRoot: agentDir,
			loadClientModule: () => ({
				ComputerUseMcpClient: FakeMcpClient as never,
			}),
		})(harness.pi);
		await harness.emit("session_start");

		await harness.tools.get("computer_use_linux_doctor")!.execute(
			"one",
			{},
			undefined,
			undefined,
			{} as never,
		);

		expect(FakeMcpClient.instances[0]?.options).toMatchObject({
			binaryPath: process.execPath,
			env: { PATH: absoluteBin },
		});
	});

	it("uses a safe child PATH fallback when every entry is unsafe", async () => {
		vi.stubEnv("COMPUTER_USE_LINUX_BIN", process.execPath);
		vi.stubEnv("PATH", ["", ".", "relative-bin"].join(delimiter));
		const harness = createPi();
		createComputerUseLinuxExtension({
			// No npm/install.js here, so a missing binary never starts a download.
			packageRoot: agentDir,
			loadClientModule: () => ({
				ComputerUseMcpClient: FakeMcpClient as never,
			}),
		})(harness.pi);
		await harness.emit("session_start");

		await harness.tools.get("computer_use_linux_doctor")!.execute(
			"one",
			{},
			undefined,
			undefined,
			{} as never,
		);

		expect(FakeMcpClient.instances[0]?.options).toMatchObject({
			env: {
				PATH: ["/usr/local/bin", "/usr/bin", "/bin"].join(delimiter),
			},
		});
	});

	it("passes the server's settings to the child and nothing else", async () => {
		const settings = {
			COMPUTER_USE_LINUX_HYPRLAND_BACKGROUND: "1",
			COMPUTER_USE_LINUX_PERSIST_REMOTE_DESKTOP: "1",
			COMPUTER_USE_LINUX_PORTAL_SCROLL_INVERT: "1",
			COMPUTER_USE_LINUX_XDOTOOL_TYPE_DELAY_MS: "12",
			NIRI_SOCKET: "/run/user/1000/niri.sock",
			PIXELFLUX_CU: "8765",
			PIXELFLUX_CU_TOKEN: "token",
		};
		for (const [name, value] of Object.entries(settings)) {
			vi.stubEnv(name, value);
		}
		vi.stubEnv("CUL_PI_TEST_UNRELATED", "1");
		vi.stubEnv("COMPUTER_USE_LINUX_NOTIFY_ON_COMPLETE", "1");
		vi.stubEnv("COMPUTER_USE_LINUX_BIN", process.execPath);
		const harness = createPi();
		createComputerUseLinuxExtension({
			// No npm/install.js here, so a missing binary never starts a download.
			packageRoot: agentDir,
			loadClientModule: () => ({
				ComputerUseMcpClient: FakeMcpClient as never,
			}),
		})(harness.pi);

		await harness.tools
			.get("computer_use_linux_doctor")!
			.execute("one", {}, undefined, undefined, {} as never);

		const env = FakeMcpClient.instances[0]?.options.env;
		expect(env).toMatchObject(settings);
		expect(env).not.toHaveProperty("CUL_PI_TEST_UNRELATED");
		// It would add a tool the generated catalog does not have.
		expect(env).not.toHaveProperty("COMPUTER_USE_LINUX_NOTIFY_ON_COMPLETE");
	});

	function stagePackageWithoutBinary() {
		// What npm 12 leaves when it skips the package's install script.
		const packageRoot = join(agentDir, "package");
		mkdirSync(join(packageRoot, "npm"), { recursive: true });
		copyFileSync(
			join(REPO_ROOT, "npm", "install.js"),
			join(packageRoot, "npm", "install.js"),
		);
		copyFileSync(
			join(REPO_ROOT, "package.json"),
			join(packageRoot, "package.json"),
		);
		return packageRoot;
	}

	it("downloads the package binary on first use, ahead of PATH", async () => {
		const packageRoot = stagePackageWithoutBinary();
		const release = join(agentDir, "release-binary");
		writeFileSync(release, "#!/bin/sh\n");
		chmodSync(release, 0o755);
		const pathDir = join(agentDir, "path");
		mkdirSync(pathDir);
		writeFileSync(join(pathDir, "computer-use-linux"), "#!/bin/sh\n");
		chmodSync(join(pathDir, "computer-use-linux"), 0o755);
		vi.stubEnv("COMPUTER_USE_LINUX_BIN", "");
		// The installer copies this file instead of downloading a release.
		vi.stubEnv("COMPUTER_USE_LINUX_LOCAL_BINARY", release);
		vi.stubEnv("PATH", pathDir);
		const harness = createPi();
		createComputerUseLinuxExtension({
			packageRoot,
			loadClientModule: () => ({
				ComputerUseMcpClient: FakeMcpClient as never,
			}),
		})(harness.pi);
		await harness.emit("session_start");
		const installed = join(
			packageRoot,
			"npm",
			"bin",
			`computer-use-linux-${process.platform}-${process.arch}`,
		);
		expect(existsSync(installed)).toBe(false);
		expect(harness.notify).not.toHaveBeenCalled();

		await harness.tools
			.get("computer_use_linux_doctor")!
			.execute("one", {}, undefined, undefined, {} as never);

		expect(existsSync(installed)).toBe(true);
		expect(FakeMcpClient.instances[0]?.options.binaryPath).toBe(installed);
	});

	it("reports a failed first-use download", async () => {
		const packageRoot = stagePackageWithoutBinary();
		vi.stubEnv("COMPUTER_USE_LINUX_BIN", "");
		vi.stubEnv(
			"COMPUTER_USE_LINUX_LOCAL_BINARY",
			join(agentDir, "missing-release-binary"),
		);
		vi.stubEnv("PATH", join(agentDir, "empty-path"));
		const harness = createPi();
		createComputerUseLinuxExtension({
			packageRoot,
			loadClientModule: () => ({
				ComputerUseMcpClient: FakeMcpClient as never,
			}),
		})(harness.pi);

		await expect(
			harness.tools
				.get("computer_use_linux_doctor")!
				.execute("one", {}, undefined, undefined, {} as never),
		).rejects.toThrow(/Downloading it failed: .*install\.js exited with status 1: .*missing-release-binary/);
		expect(FakeMcpClient.instances).toHaveLength(0);
	});

	it("retries the download after a PATH fallback fails", async () => {
		const packageRoot = stagePackageWithoutBinary();
		const pathDir = join(agentDir, "path");
		mkdirSync(pathDir);
		const pathBinary = join(pathDir, "computer-use-linux");
		writeFileSync(pathBinary, "#!/bin/sh\n");
		chmodSync(pathBinary, 0o755);
		FakeMcpClient.failingBinary = pathBinary;
		vi.stubEnv("COMPUTER_USE_LINUX_BIN", "");
		vi.stubEnv(
			"COMPUTER_USE_LINUX_LOCAL_BINARY",
			join(agentDir, "release-binary"),
		);
		vi.stubEnv("PATH", pathDir);
		const harness = createPi();
		createComputerUseLinuxExtension({
			packageRoot,
			loadClientModule: () => ({
				ComputerUseMcpClient: FakeMcpClient as never,
			}),
		})(harness.pi);
		const doctor = harness.tools.get("computer_use_linux_doctor")!;

		// The download fails, so the mismatched PATH binary runs.
		await expect(
			doctor.execute("one", {}, undefined, undefined, {} as never),
		).rejects.toThrow("does not match");
		expect(FakeMcpClient.instances[0]?.options.binaryPath).toBe(pathBinary);
		expect(FakeMcpClient.instances[0]?.closed).toBe(1);

		// The download works now; the next call installs and uses it.
		writeFileSync(join(agentDir, "release-binary"), "#!/bin/sh\n");
		chmodSync(join(agentDir, "release-binary"), 0o755);
		await doctor.execute("two", {}, undefined, undefined, {} as never);
		expect(FakeMcpClient.instances[1]?.options.binaryPath).toBe(
			join(
				packageRoot,
				"npm",
				"bin",
				`computer-use-linux-${process.platform}-${process.arch}`,
			),
		);
	});

	it("starts no download for a cancelled call", async () => {
		const packageRoot = stagePackageWithoutBinary();
		const release = join(agentDir, "release-binary");
		writeFileSync(release, "#!/bin/sh\n");
		chmodSync(release, 0o755);
		vi.stubEnv("COMPUTER_USE_LINUX_BIN", "");
		vi.stubEnv("COMPUTER_USE_LINUX_LOCAL_BINARY", release);
		vi.stubEnv("PATH", join(agentDir, "empty-path"));
		const harness = createPi();
		createComputerUseLinuxExtension({
			packageRoot,
			loadClientModule: () => ({
				ComputerUseMcpClient: FakeMcpClient as never,
			}),
		})(harness.pi);
		const controller = new AbortController();
		controller.abort(new Error("cancelled by the user"));

		await expect(
			harness.tools
				.get("computer_use_linux_doctor")!
				.execute("one", {}, controller.signal, undefined, {} as never),
		).rejects.toThrow("cancelled by the user");
		expect(existsSync(join(packageRoot, "npm", "bin"))).toBe(false);
		expect(FakeMcpClient.instances).toHaveLength(0);
	});

	it("stops waiting for a download when the call is cancelled", async () => {
		const packageRoot = join(agentDir, "package");
		mkdirSync(join(packageRoot, "npm"), { recursive: true });
		// An installer that is still downloading.
		writeFileSync(
			join(packageRoot, "npm", "install.js"),
			"setTimeout(() => {}, 2000);\n",
		);
		vi.stubEnv("COMPUTER_USE_LINUX_BIN", "");
		vi.stubEnv("PATH", join(agentDir, "empty-path"));
		const harness = createPi();
		createComputerUseLinuxExtension({
			packageRoot,
			loadClientModule: () => ({
				ComputerUseMcpClient: FakeMcpClient as never,
			}),
		})(harness.pi);
		const controller = new AbortController();
		const call = harness.tools
			.get("computer_use_linux_doctor")!
			.execute("one", {}, controller.signal, undefined, {} as never);
		const started = Date.now();
		setTimeout(() => controller.abort(new Error("cancelled by the user")), 100);

		await expect(call).rejects.toThrow("cancelled by the user");
		expect(Date.now() - started).toBeLessThan(1500);
		expect(FakeMcpClient.instances).toHaveLength(0);
	});

	it("ignores a relative executable override", async () => {
		const relativeOverride = relative(process.cwd(), process.execPath);
		vi.stubEnv("COMPUTER_USE_LINUX_BIN", relativeOverride);
		vi.stubEnv("PATH", "relative-bin");
		const harness = createPi();
		createComputerUseLinuxExtension({
			// No npm/install.js here, so a missing binary never starts a download.
			packageRoot: agentDir,
			loadClientModule: () => ({
				ComputerUseMcpClient: FakeMcpClient as never,
			}),
		})(harness.pi);
		await harness.emit("session_start");

		await harness.tools
			.get("computer_use_linux_doctor")!
			.execute("one", {}, undefined, undefined, {} as never)
			.catch(() => undefined);

		expect(FakeMcpClient.instances[0]?.options.binaryPath).not.toBe(
			relativeOverride,
		);
	});
});

describe("findExecutableOnPath", () => {
	let root: string;
	beforeEach(() => {
		root = mkdtempSync(join(tmpdir(), "cul-path-"));
	});
	afterEach(() => {
		rmSync(root, { recursive: true, force: true });
	});

	const script = (dir: string, mode: number) => {
		mkdirSync(dir, { recursive: true });
		const file = join(dir, "computer-use-linux");
		writeFileSync(file, "#!/bin/sh\nexit 0\n");
		chmodSync(file, mode);
		return file;
	};

	it("returns the first executable in PATH order", () => {
		const first = script(join(root, "a"), 0o755);
		script(join(root, "b"), 0o755);
		const path = [join(root, "a"), join(root, "b")].join(delimiter);
		expect(findExecutableOnPath(path, "computer-use-linux")).toBe(first);
	});

	it("skips non-executable files, directories, and empty segments", () => {
		script(join(root, "noexec"), 0o644);
		mkdirSync(join(root, "dir", "computer-use-linux"), { recursive: true });
		const wanted = script(join(root, "ok"), 0o755);
		const path = [
			"",
			join(root, "noexec"),
			join(root, "dir"),
			"",
			join(root, "ok"),
		].join(delimiter);
		expect(findExecutableOnPath(path, "computer-use-linux")).toBe(wanted);
	});

	it("skips relative PATH entries", () => {
		const wanted = script(join(root, "ok"), 0o755);
		const checked: string[] = [];
		const path = [".", "relative-bin", join(root, "ok")].join(delimiter);

		expect(
			findExecutableOnPath(path, "computer-use-linux", (candidate) => {
				checked.push(candidate);
				return candidate === wanted;
			}),
		).toBe(wanted);
		expect(checked).toEqual([wanted]);
	});

	it("sanitizes PATH lists to absolute entries", () => {
		const path = ["", ".", "relative-bin", "/usr/bin", "/bin"].join(
			delimiter,
		);
		expect(sanitizeExecutablePath(path)).toBe(
			["/usr/bin", "/bin"].join(delimiter),
		);
	});

	it("returns null when nothing on PATH matches", () => {
		expect(findExecutableOnPath(join(root, "missing"), "computer-use-linux")).toBeNull();
		expect(findExecutableOnPath(undefined, "computer-use-linux")).toBeNull();
	});
});

import type { ExtensionAPI, ToolDefinition } from "@earendil-works/pi-coding-agent";
import { afterEach, describe, expect, it, vi } from "vitest";

class FakeShellClient {
	static options: Record<string, unknown> | undefined;
	static calls: Array<{ name: string; args: Record<string, unknown> }> = [];

	constructor(options: Record<string, unknown>) {
		FakeShellClient.options = options;
	}

	async callTool(name: string, args: Record<string, unknown>) {
		FakeShellClient.calls.push({ name, args });
		return { content: [{ type: "text", text: "ok" }] };
	}

	async close() {}
}

describe("shell-enabled native Pi catalog", () => {
	afterEach(() => {
		vi.unstubAllEnvs();
		vi.resetModules();
		FakeShellClient.options = undefined;
		FakeShellClient.calls = [];
	});

	it("wires run_shell through the shell catalog without inheriting unrelated secrets", async () => {
		vi.stubEnv("COMPUTER_USE_LINUX_ENABLE_SHELL", "1");
		vi.stubEnv("COMPUTER_USE_LINUX_BIN", process.execPath);
		vi.stubEnv("COMPUTER_USE_LINUX_TEST_SECRET", "must-not-leak");
		vi.resetModules();
		const [
			{ createComputerUseLinuxExtension },
			{ GENERATED_SHELL_TOOL_CATALOG_HASH },
		] = await Promise.all([
			import("../extension/index.ts"),
			import("../extension/generated-tools.ts"),
		]);
		const tools = new Map<string, ToolDefinition>();
		const pi = {
			registerTool(tool: ToolDefinition) {
				tools.set(tool.name, tool);
			},
			on() {},
		} as unknown as ExtensionAPI;

		createComputerUseLinuxExtension({
			loadClientModule: () => ({
				ComputerUseMcpClient: FakeShellClient as never,
			}),
		})(pi);

		expect(tools.has("computer_use_linux_run_shell")).toBe(true);
		const loader = tools.get("computer_use_linux_tools");
		expect(loader?.parameters).toMatchObject({
			properties: {
				tools: {
					items: {
						enum: expect.arrayContaining(["run_shell"]),
					},
				},
			},
		});
		await tools.get("computer_use_linux_run_shell")!.execute(
			"shell",
			{ command: "printf ok" },
			undefined,
			undefined,
			{} as never,
		);
		expect(FakeShellClient.options).toMatchObject({
			expectedCatalogHash: GENERATED_SHELL_TOOL_CATALOG_HASH,
			env: {
				COMPUTER_USE_LINUX_ENABLE_SHELL: "1",
			},
		});
		expect(
			(FakeShellClient.options?.env as Record<string, string>)[
				"COMPUTER_USE_LINUX_TEST_SECRET"
			],
		).toBeUndefined();
		expect(FakeShellClient.calls).toEqual([
			{ name: "run_shell", args: { command: "printf ok" } },
		]);
	});
});

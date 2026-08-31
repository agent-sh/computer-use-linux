import type { ExtensionAPI, ToolDefinition } from "@earendil-works/pi-coding-agent";
import { afterEach, describe, expect, it, vi } from "vitest";

const binary = process.env.COMPUTER_USE_LINUX_TEST_BINARY;

describe.runIf(binary)("shell-enabled extension environment", () => {
	afterEach(() => {
		vi.unstubAllEnvs();
		vi.resetModules();
	});

	it("does not expose unrelated Pi secrets through the MCP parent process", async () => {
		vi.stubEnv("COMPUTER_USE_LINUX_ENABLE_SHELL", "1");
		vi.stubEnv("COMPUTER_USE_LINUX_BIN", binary!);
		vi.stubEnv("COMPUTER_USE_LINUX_TEST_SECRET", "must-not-leak");
		vi.resetModules();
		const { createComputerUseLinuxExtension } = await import(
			"../extension/index.ts"
		);
		const tools = new Map<string, ToolDefinition>();
		const handlers = new Map<string, Array<(...args: any[]) => unknown>>();
		const pi = {
			registerTool(tool: ToolDefinition) {
				tools.set(tool.name, tool);
			},
			on(event: string, handler: (...args: any[]) => unknown) {
				const entries = handlers.get(event) ?? [];
				entries.push(handler);
				handlers.set(event, entries);
			},
		} as unknown as ExtensionAPI;
		createComputerUseLinuxExtension()(pi);

		const result = await tools.get("computer_use_linux_run_shell")!.execute(
			"shell",
			{
				command:
					"if tr '\\0' '\\n' < /proc/$PPID/environ | grep -q '^COMPUTER_USE_LINUX_TEST_SECRET='; then printf leaked; exit 9; else printf isolated; fi",
				timeout_seconds: 5,
			},
			undefined,
			undefined,
			{} as never,
		);
		const first = result.content[0];
		const payload = JSON.parse(first?.type === "text" ? first.text : "{}");
		expect(payload).toMatchObject({ ok: true, stdout: "isolated" });

		for (const handler of handlers.get("session_shutdown") ?? []) {
			await handler({}, {});
		}
	});
});

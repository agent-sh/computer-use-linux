import { getModel } from "@earendil-works/pi-ai/compat";
import {
	DefaultResourceLoader,
	SessionManager,
	SettingsManager,
	createAgentSession,
} from "@earendil-works/pi-coding-agent";
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it, vi } from "vitest";
import { GENERATED_MCP_TOOLS } from "../extension/generated-tools.ts";

const extensionPath =
	process.env.COMPUTER_USE_LINUX_EXTENSION_PATH ??
	resolve(
		dirname(fileURLToPath(import.meta.url)),
		"..",
		"extension",
		"index.ts",
	);

describe("Pi 0.84 native integration", () => {
	let tempDir: string | undefined;

	afterEach(() => {
		vi.unstubAllEnvs();
		if (tempDir && existsSync(tempDir)) {
			rmSync(tempDir, { recursive: true, force: true });
		}
		tempDir = undefined;
	});

	it("loads through Pi's real lifecycle and preserves exact tools across reload", async () => {
		tempDir = mkdtempSync(join(tmpdir(), "computer-use-linux-pi-real-"));
		const agentDir = join(tempDir, "agent");
		const usePackagedBinary =
			process.env.COMPUTER_USE_LINUX_USE_PACKAGED_BINARY === "1";
		vi.stubEnv(
			"COMPUTER_USE_LINUX_BIN",
			usePackagedBinary
				? ""
				: process.env.COMPUTER_USE_LINUX_TEST_BINARY ?? process.execPath,
		);
		vi.stubEnv("PI_CODING_AGENT_DIR", agentDir);

		const settingsManager = SettingsManager.create(tempDir, agentDir);
		const resourceLoader = new DefaultResourceLoader({
			cwd: tempDir,
			agentDir,
			settingsManager,
			additionalExtensionPaths: [extensionPath],
		});
		await resourceLoader.reload();

		const { session } = await createAgentSession({
			cwd: tempDir,
			agentDir,
			model: getModel("anthropic", "claude-sonnet-4-5")!,
			settingsManager,
			sessionManager: SessionManager.inMemory(),
			resourceLoader,
		});
		try {
				await session.bindExtensions({ onError: () => {} });
			expect(session.getAllTools().map((tool) => tool.name)).toEqual(
				expect.arrayContaining([
					"computer_use_linux_tools",
					...GENERATED_MCP_TOOLS.map(
						(tool) => `computer_use_linux_${tool.name}`,
					),
				]),
			);
			expect(session.getActiveToolNames()).toContain(
				"computer_use_linux_tools",
			);
			expect(session.systemPrompt).toContain(
				"Enable Linux desktop tools only when the task needs local GUI observation or control",
			);
			expect(session.systemPrompt).not.toContain(
				"computer_use_linux_click",
			);
			for (const tool of GENERATED_MCP_TOOLS) {
				expect(session.getActiveToolNames()).not.toContain(
					`computer_use_linux_${tool.name}`,
				);
			}

			const loader = session.getToolDefinition("computer_use_linux_tools");
			expect(loader).toBeDefined();
			await loader!.execute(
				"loader",
				{ tools: ["doctor", "get_app_state"] },
				undefined,
				undefined,
				{} as never,
			);
				expect(session.getActiveToolNames()).toEqual(
					expect.arrayContaining([
						"computer_use_linux_tools",
						"computer_use_linux_doctor",
						"computer_use_linux_get_app_state",
					]),
				);
				await session.reload();
				expect(session.getActiveToolNames()).toEqual(
					expect.arrayContaining([
						"computer_use_linux_tools",
						"computer_use_linux_doctor",
						"computer_use_linux_get_app_state",
					]),
				);
				for (const tool of GENERATED_MCP_TOOLS) {
					if (tool.name === "doctor" || tool.name === "get_app_state") continue;
					expect(session.getActiveToolNames()).not.toContain(
						`computer_use_linux_${tool.name}`,
					);
				}
				if (usePackagedBinary) {
				const doctor = session.getToolDefinition(
					"computer_use_linux_doctor",
				);
				const result = await doctor!.execute(
					"doctor",
					{},
					undefined,
					undefined,
					{} as never,
				);
				const first = result.content[0];
				expect(first?.type).toBe("text");
				const report = JSON.parse(
					first?.type === "text" ? first.text : "{}",
				);
				expect(report).toHaveProperty(
					"readiness.can_register_mcp_tools",
					true,
				);
			}
		} finally {
			session.dispose();
		}
	});
});

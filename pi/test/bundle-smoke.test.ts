import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
	GENERATED_SERVER_VERSION,
	GENERATED_SHELL_TOOL_CATALOG_HASH,
	GENERATED_TOOL_CATALOG_HASH,
} from "../extension/generated-tools.ts";

const require = createRequire(import.meta.url);
const binary = process.env.COMPUTER_USE_LINUX_TEST_BINARY;

describe.runIf(binary)("generated MCP client bundle", () => {
	it("connects to the real Rust server and calls doctor", async () => {
		const module = require(
			resolve(
				dirname(fileURLToPath(import.meta.url)),
				"..",
				"extension",
				"mcp-client.bundle.cjs",
			),
		) as {
			ComputerUseMcpClient: new (options: Record<string, unknown>) => {
				callTool(
					name: string,
					args: Record<string, unknown>,
				): Promise<{
					content?: Array<{ type: string; text?: string }>;
				}>;
				close(): Promise<void>;
			};
		};
		const env = Object.fromEntries(
			Object.entries(process.env).filter(
				(entry): entry is [string, string] => typeof entry[1] === "string",
			),
		);
		const client = new module.ComputerUseMcpClient({
			binaryPath: binary,
			clientVersion: GENERATED_SERVER_VERSION,
			env,
			expectedCatalogHash: GENERATED_TOOL_CATALOG_HASH,
			expectedServerVersion: GENERATED_SERVER_VERSION,
			requestTimeoutMs: 30_000,
		});
		try {
			const result = await client.callTool("doctor", {});
			expect(result.content?.[0]).toMatchObject({ type: "text" });
			const report = JSON.parse(result.content?.[0]?.text ?? "{}");
			expect(report).toHaveProperty("readiness.can_register_mcp_tools", true);
		} finally {
			await client.close();
		}
	});

	it("accepts and calls the opt-in run_shell catalog", async () => {
		const module = require(
			resolve(
				dirname(fileURLToPath(import.meta.url)),
				"..",
				"extension",
				"mcp-client.bundle.cjs",
			),
		) as {
			ComputerUseMcpClient: new (options: Record<string, unknown>) => {
				callTool(
					name: string,
					args: Record<string, unknown>,
				): Promise<{
					content?: Array<{ type: string; text?: string }>;
				}>;
				close(): Promise<void>;
			};
		};
		const env = Object.fromEntries(
			Object.entries(process.env).filter(
				(entry): entry is [string, string] => typeof entry[1] === "string",
			),
		);
		env.COMPUTER_USE_LINUX_ENABLE_SHELL = "1";
		const client = new module.ComputerUseMcpClient({
			binaryPath: binary,
			clientVersion: GENERATED_SERVER_VERSION,
			env,
			expectedCatalogHash: GENERATED_SHELL_TOOL_CATALOG_HASH,
			expectedServerVersion: GENERATED_SERVER_VERSION,
			requestTimeoutMs: 30_000,
		});
		try {
			const result = await client.callTool("run_shell", {
				command: "printf pi-shell-ok",
				timeout_seconds: 5,
			});
			const payload = JSON.parse(result.content?.[0]?.text ?? "{}");
			expect(payload).toMatchObject({ ok: true, stdout: "pi-shell-ok" });
		} finally {
			await client.close();
		}
	});
});

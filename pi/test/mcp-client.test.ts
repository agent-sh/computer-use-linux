import { createHash } from "node:crypto";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { ComputerUseMcpClient } from "../extension/mcp-client.ts";

const fixture = join(
	dirname(fileURLToPath(import.meta.url)),
	"fixtures",
	"mcp-server.mjs",
);

const fixtureTools = [
	{
		name: "delay",
		description: "Delay until cancelled.",
		inputSchema: { type: "object", properties: {} },
		outputSchema: null,
		annotations: {},
	},
	{
		name: "exit",
		description: "Exit before responding.",
		inputSchema: { type: "object", properties: {} },
		outputSchema: null,
		annotations: {},
	},
	{
		name: "ping",
		description: "Return process identity and call count.",
		inputSchema: { type: "object", properties: {} },
		outputSchema: null,
		annotations: {},
	},
];

function canonicalize(value: unknown): unknown {
	if (Array.isArray(value)) return value.map(canonicalize);
	if (value && typeof value === "object") {
		return Object.fromEntries(
			Object.entries(value)
				.sort(([left], [right]) =>
					left < right ? -1 : left > right ? 1 : 0,
				)
				.map(([key, item]) => [key, canonicalize(item)]),
		);
	}
	return value;
}

function catalogHash() {
	return createHash("sha256")
		.update(JSON.stringify(canonicalize(fixtureTools)))
		.digest("hex");
}

function createClient(overrides: Record<string, unknown> = {}) {
	return new ComputerUseMcpClient({
		binaryPath: process.execPath,
		binaryArgs: [fixture],
		clientVersion: "0.4.10",
		env: {
			PATH: process.env.PATH ?? "",
			FIXTURE_VERSION: "0.4.10",
		},
		expectedCatalogHash: catalogHash(),
		expectedServerVersion: "0.4.10",
		requestTimeoutMs: 2_000,
		...overrides,
	} as never);
}

function parsePing(result: Awaited<ReturnType<ComputerUseMcpClient["callTool"]>>) {
	const block = result.content?.[0];
	if (!block || block.type !== "text") {
		throw new Error("ping did not return text");
	}
	return JSON.parse(block.text) as {
		pid: number;
		count: number;
		cancelled: number;
		pending: number;
	};
}

describe("ComputerUseMcpClient", () => {
	it("reuses one stdio process for stateful calls", async () => {
		const client = createClient();

		try {
			const first = parsePing(await client.callTool("ping", {}));
			const second = parsePing(await client.callTool("ping", {}));
			expect(second.pid).toBe(first.pid);
			expect(second.count).toBe(2);
		} finally {
			await client.close();
		}
	});

	it("rejects catalog drift before dispatching tools", async () => {
		const client = createClient({
			expectedCatalogHash: "0".repeat(64),
		});
		try {
			await expect(client.callTool("ping", {})).rejects.toThrow(
				"tools do not match",
			);
		} finally {
			await client.close();
		}
	});

	it("does not replay a call when the server exits and reconnects on the next call", async () => {
		const client = createClient();
		try {
			const before = parsePing(await client.callTool("ping", {}));
			await expect(client.callTool("exit", {})).rejects.toThrow(
				"was not replayed",
			);
			const after = parsePing(await client.callTool("ping", {}));
			expect(after.pid).not.toBe(before.pid);
			expect(after.count).toBe(1);
		} finally {
			await client.close();
		}
	});

	it("forwards cancellation without killing the session process", async () => {
		const client = createClient();
		try {
			const before = parsePing(await client.callTool("ping", {}));
			const controller = new AbortController();
			const delayed = client.callTool("delay", {}, controller.signal);
			setTimeout(() => controller.abort(new Error("cancelled by test")), 25);
			await expect(delayed).rejects.toThrow("was not replayed");
			const after = parsePing(await client.callTool("ping", {}));
			expect(after.pid).toBe(before.pid);
			expect(after.count).toBe(2);
			expect(after.cancelled).toBe(1);
			expect(after.pending).toBe(0);
		} finally {
			await client.close();
		}
	});

	it("does not replay timed-out calls and keeps the process usable", async () => {
		const client = createClient({ requestTimeoutMs: 1_000 });
		try {
			const before = parsePing(await client.callTool("ping", {}));
			await expect(client.callTool("delay", {})).rejects.toThrow(
				"was not replayed",
			);
			const after = parsePing(await client.callTool("ping", {}));
			expect(after.pid).toBe(before.pid);
			expect(after.count).toBe(2);
			expect(after.cancelled).toBe(1);
			expect(after.pending).toBe(0);
		} finally {
			await client.close();
		}
	});

	it("lets one caller cancel its wait without aborting a shared connection", async () => {
		const client = createClient({
			env: {
				PATH: process.env.PATH ?? "",
				FIXTURE_VERSION: "0.4.10",
				FIXTURE_INIT_DELAY_MS: "100",
			},
		});
		try {
			const controller = new AbortController();
			const cancelledCall = client.callTool("ping", {}, controller.signal);
			const waitingCall = client.callTool("ping", {});
			setTimeout(() => controller.abort(new Error("cancel first waiter")), 10);
			await expect(cancelledCall).rejects.toThrow("cancel first waiter");
			const result = parsePing(await waitingCall);
			expect(result.count).toBe(1);
		} finally {
			await client.close();
		}
	});

	it("does not start a connection for a caller that is already aborted", async () => {
		const client = createClient();
		const controller = new AbortController();
		controller.abort(new Error("already cancelled"));
		try {
			await expect(
				client.callTool("ping", {}, controller.signal),
			).rejects.toThrow("already cancelled");
			expect(client.pid).toBeNull();
		} finally {
			await client.close();
		}
	});

	it("rejects a binary version that does not match its generated schemas", async () => {
		const client = createClient({
			expectedServerVersion: "9.9.9",
		});
		try {
			await expect(client.callTool("ping", {})).rejects.toThrow(
				"does not match",
			);
		} finally {
			await client.close();
		}
	});
});

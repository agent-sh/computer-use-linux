import {
	Client,
	SdkError,
	SdkErrorCode,
	type CallToolResult,
	type Implementation,
	type Tool,
} from "@modelcontextprotocol/client";
import { StdioClientTransport } from "@modelcontextprotocol/client/stdio";
import { createHash } from "node:crypto";

export interface ComputerUseMcpClientOptions {
	binaryPath: string;
	binaryArgs?: string[];
	clientVersion: string;
	env: Record<string, string>;
	expectedCatalogHash: string;
	expectedServerVersion: string;
	requestTimeoutMs?: number;
}

function combineSignals(
	first: AbortSignal | undefined,
	second: AbortSignal,
): AbortSignal {
	return first ? AbortSignal.any([first, second]) : second;
}

function isRequestTimeout(error: unknown): boolean {
	return (
		error instanceof SdkError &&
		error.code === SdkErrorCode.RequestTimeout
	);
}

function abortReason(signal: AbortSignal): Error {
	return signal.reason instanceof Error
		? signal.reason
		: new Error(String(signal.reason ?? "cancelled"));
}

function compareStrings(left: string, right: string): number {
	return left < right ? -1 : left > right ? 1 : 0;
}

function canonicalize(value: unknown): unknown {
	if (Array.isArray(value)) {
		return value.map(canonicalize);
	}
	if (value && typeof value === "object") {
			return Object.fromEntries(
				Object.entries(value)
					.sort(([left], [right]) => compareStrings(left, right))
					.map(([key, item]) => [key, canonicalize(item)]),
			);
	}
	return value;
}

function catalogHash(tools: Tool[]): string {
	const catalog = tools
			.map((tool) => ({
			name: tool.name,
			description: tool.description ?? "",
			inputSchema: tool.inputSchema ?? { type: "object" },
			outputSchema: tool.outputSchema ?? null,
			annotations: tool.annotations ?? {},
		}))
			.sort((left, right) => compareStrings(left.name, right.name));
	return createHash("sha256")
		.update(JSON.stringify(canonicalize(catalog)))
		.digest("hex");
}

export class ComputerUseMcpClient {
	private client: Client | undefined;
	private transport: StdioClientTransport | undefined;
	private connecting: Promise<void> | undefined;
	private readonly sessionAbort = new AbortController();
	private stderrTail = "";
	private closed = false;

	constructor(private readonly options: ComputerUseMcpClientOptions) {}

	get pid(): number | null {
		return this.transport?.pid ?? null;
	}

	get serverVersion(): Implementation | undefined {
		return this.client?.getServerVersion();
	}

	async listTools(signal?: AbortSignal): Promise<Tool[]> {
		const client = await this.ensureConnected(signal);
		try {
			const result = await client.listTools(
				undefined,
				this.requestOptions(signal),
			);
			return result.tools;
		} catch (error) {
			throw await this.handleRequestFailure(error, signal);
		}
	}

	async callTool(
		name: string,
		args: Record<string, unknown>,
		signal?: AbortSignal,
	): Promise<CallToolResult> {
		const client = await this.ensureConnected(signal);
		try {
			return await client.callTool(
				{ name, arguments: args },
				this.requestOptions(signal),
			);
		} catch (error) {
			throw await this.handleRequestFailure(error, signal);
		}
	}

	async close(): Promise<void> {
		if (this.closed) return;
		this.closed = true;
		this.sessionAbort.abort(new Error("Pi session ended"));
		const connecting = this.connecting;
		this.connecting = undefined;
		if (connecting) {
			await connecting.catch(() => {});
		}
		await this.resetConnection();
	}

	private async ensureConnected(signal?: AbortSignal): Promise<Client> {
		if (this.closed) {
			throw new Error("Computer Use connection is closed");
		}
		if (signal?.aborted) {
			throw abortReason(signal);
		}
		if (this.client && this.transport?.pid !== null) {
			return this.client;
		}
		if (this.client || this.transport) {
			await this.resetConnection();
		}
		if (!this.connecting) {
			const connecting = this.connect().finally(() => {
				if (this.connecting === connecting) {
					this.connecting = undefined;
				}
			});
			this.connecting = connecting;
		}
		await this.waitForConnection(this.connecting, signal);
		if (!this.client) {
			throw new Error("Computer Use MCP client did not connect");
		}
		return this.client;
	}

	private async connect(): Promise<void> {
		this.stderrTail = "";
		const client = new Client({
			name: "computer-use-linux-pi",
			version: this.options.clientVersion,
		});
		const transport = new StdioClientTransport({
			command: this.options.binaryPath,
			args: this.options.binaryArgs ?? ["mcp"],
			env: this.options.env,
			stderr: "pipe",
		});
		transport.stderr?.on("data", (chunk: Buffer | string) => {
			this.stderrTail = `${this.stderrTail}${chunk.toString()}`.slice(-8192);
		});

		try {
			await client.connect(transport, this.requestOptions());
			const server = client.getServerVersion();
			if (server?.name !== "computer-use-linux") {
				throw new Error(
					`unexpected MCP server identity: ${server?.name ?? "unknown"}`,
				);
			}
			if (server.version !== this.options.expectedServerVersion) {
				throw new Error(
					`computer-use-linux version ${server.version} does not match ` +
						`the Pi extension catalog version ${this.options.expectedServerVersion}`,
				);
			}
			const tools = await client.listTools(
				undefined,
				this.requestOptions(),
			);
			const actualCatalogHash = catalogHash(tools.tools);
			if (actualCatalogHash !== this.options.expectedCatalogHash) {
				throw new Error(
					"computer-use-linux tools do not match the Pi extension catalog; " +
						"reinstall or update @agent-sh/computer-use-linux",
				);
			}
			if (this.closed) {
				throw new Error("Pi session ended while Computer Use was connecting");
			}
			this.client = client;
			this.transport = transport;
		} catch (error) {
			await client.close().catch(() => {});
			throw this.enrichError(error);
		}
	}

	private requestOptions(signal?: AbortSignal) {
		return {
			signal: combineSignals(signal, this.sessionAbort.signal),
			timeout: this.options.requestTimeoutMs ?? 60_000,
		};
	}

	private async waitForConnection(
		connection: Promise<void>,
		signal?: AbortSignal,
	): Promise<void> {
		if (!signal) {
			await connection;
			return;
		}
		if (signal.aborted) {
			throw abortReason(signal);
		}
		await new Promise<void>((resolve, reject) => {
			const onAbort = () => {
				reject(abortReason(signal));
			};
			signal.addEventListener("abort", onAbort, { once: true });
			connection.then(resolve, reject).finally(() => {
				signal.removeEventListener("abort", onAbort);
			});
		});
	}

	private async handleRequestFailure(
		error: unknown,
		signal?: AbortSignal,
	): Promise<Error> {
		const connectionEnded = this.transport?.pid === null;
		if (connectionEnded) {
			await this.resetConnection();
		}
		const enriched = this.enrichError(error);
		if (signal?.aborted || connectionEnded || isRequestTimeout(error)) {
			return new Error(
				`${enriched.message} The call ended without a confirmed result and was not replayed. ` +
					"Verify the current UI state and call get_app_state again before retrying an element-based action.",
				{ cause: enriched },
			);
		}
		return enriched;
	}

	private async resetConnection(): Promise<void> {
		const client = this.client;
		const transport = this.transport;
		this.client = undefined;
		this.transport = undefined;
		if (client) {
			await client.close().catch(() => {});
		} else if (transport) {
			await transport.close().catch(() => {});
		}
	}

	private enrichError(error: unknown): Error {
		const base = error instanceof Error ? error : new Error(String(error));
		const stderr = this.stderrTail.trim();
		if (!stderr || base.message.includes(stderr)) {
			return base;
		}
		return new Error(`${base.message} (server stderr: ${stderr})`, {
			cause: base,
		});
	}
}

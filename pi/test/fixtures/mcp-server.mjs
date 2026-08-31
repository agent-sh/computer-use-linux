import { createInterface } from "node:readline";

const version = process.env.FIXTURE_VERSION || "0.4.10";
const initializeDelayMs = Number(process.env.FIXTURE_INIT_DELAY_MS || "0");
let count = 0;
let cancelled = 0;
const timers = new Map();

const tools = [
	{
		name: "delay",
		description: "Delay until cancelled.",
		inputSchema: { type: "object", properties: {} },
		annotations: {},
	},
	{
		name: "exit",
		description: "Exit before responding.",
		inputSchema: { type: "object", properties: {} },
		annotations: {},
	},
	{
		name: "ping",
		description: "Return process identity and call count.",
		inputSchema: { type: "object", properties: {} },
		annotations: {},
	},
];

function send(message) {
	process.stdout.write(`${JSON.stringify(message)}\n`);
}

function result(id, value) {
	send({ jsonrpc: "2.0", id, result: value });
}

const lines = createInterface({ input: process.stdin });
lines.on("line", (line) => {
	const message = JSON.parse(line);
	if (message.method === "initialize") {
		setTimeout(() => {
			result(message.id, {
				protocolVersion: "2025-11-25",
				capabilities: { tools: {} },
				serverInfo: { name: "computer-use-linux", version },
			});
		}, initializeDelayMs);
		return;
	}
	if (message.method === "tools/list") {
		result(message.id, { tools });
		return;
	}
	if (message.method === "notifications/cancelled") {
		const requestId = message.params?.requestId;
		const timer = timers.get(requestId);
		if (timer) {
			clearTimeout(timer);
			timers.delete(requestId);
			cancelled += 1;
		}
		return;
	}
	if (message.method !== "tools/call") return;

	const name = message.params?.name;
	if (name === "exit") {
		process.exit(23);
	}
	if (name === "delay") {
		const timer = setTimeout(() => {
			timers.delete(message.id);
			result(message.id, {
				content: [{ type: "text", text: "late" }],
			});
		}, 5_000);
		timers.set(message.id, timer);
		return;
	}
	if (name === "ping") {
		count += 1;
		result(message.id, {
			content: [
				{
					type: "text",
					text: JSON.stringify({
						pid: process.pid,
						count,
						cancelled,
						pending: timers.size,
					}),
				},
			],
		});
	}
});

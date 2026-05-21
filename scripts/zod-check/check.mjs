#!/usr/bin/env node
// Regression guard for https://github.com/agent-sh/computer-use-linux/issues/1.
//
// Launches the MCP server, performs the initialize handshake, and parses the
// tools/list response with @modelcontextprotocol/sdk's ListToolsResultSchema —
// the same zod validation strict clients (mcphub, Claude Desktop) run. A
// boolean schema node such as `outputSchema.properties.received: true` makes
// AssertObjectSchema throw a ZodError here, exactly as it did for clients.
//
// Usage: node check.mjs [--command <binary-or-wrapper>]
//   --command  path to the server entrypoint; `mcp` is appended (default:
//              target/debug/computer-use-linux)

import { spawn } from 'node:child_process';
import process from 'node:process';
import { ListToolsResultSchema } from '@modelcontextprotocol/sdk/types.js';

function parseArgs(argv) {
  const args = { command: 'target/debug/computer-use-linux' };
  for (let i = 0; i < argv.length; i += 1) {
    if (argv[i] === '--command' && argv[i + 1]) {
      args.command = argv[i + 1];
      i += 1;
    }
  }
  return args;
}

const { command } = parseArgs(process.argv.slice(2));

const child = spawn(command, ['mcp'], { stdio: ['pipe', 'pipe', 'inherit'] });
child.stdout.setEncoding('utf8');

function done(code, message) {
  if (message) {
    (code === 0 ? console.log : console.error)(message);
  }
  if (!child.killed) {
    child.kill();
  }
  process.exit(code);
}

child.on('error', (error) => done(1, `[zod-schema] failed to start ${command}: ${error.message}`));

const pending = new Map();
let buffer = '';

child.stdout.on('data', (chunk) => {
  buffer += chunk;
  let newline = buffer.indexOf('\n');
  while (newline >= 0) {
    const line = buffer.slice(0, newline).trim();
    buffer = buffer.slice(newline + 1);
    newline = buffer.indexOf('\n');
    if (!line) continue;
    let message;
    try {
      message = JSON.parse(line);
    } catch {
      // Non-JSON-RPC chatter on stdout — ignore.
      continue;
    }
    if (message.id != null && pending.has(message.id)) {
      const { resolve, reject } = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) reject(new Error(JSON.stringify(message.error)));
      else resolve(message.result);
    }
  }
});

let nextId = 1;
function request(method, params) {
  const id = nextId;
  nextId += 1;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`);
  });
}

function notify(method, params) {
  child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', method, params })}\n`);
}

const timeout = setTimeout(() => done(1, '[zod-schema] timed out waiting for MCP server'), 20000);

try {
  await request('initialize', {
    protocolVersion: '2024-11-05',
    capabilities: {},
    clientInfo: { name: 'zod-schema-check', version: '0' },
  });
  notify('notifications/initialized', {});

  const result = await request('tools/list', {});
  ListToolsResultSchema.parse(result);

  clearTimeout(timeout);
  done(
    0,
    `[zod-schema] OK: ${result.tools.length} tools validated against @modelcontextprotocol/sdk ListToolsResultSchema`,
  );
} catch (error) {
  clearTimeout(timeout);
  if (error && error.name === 'ZodError') {
    console.error('[zod-schema] ZodError validating tools/list (strict MCP clients would reject this):');
    console.error(JSON.stringify(error.issues, null, 2));
    done(1);
  } else {
    done(1, `[zod-schema] ${error && error.message ? error.message : error}`);
  }
}

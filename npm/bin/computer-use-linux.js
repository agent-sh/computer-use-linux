#!/usr/bin/env node
'use strict';

const fs = require('node:fs');
const path = require('node:path');
const { spawn, spawnSync } = require('node:child_process');

const binaryName = `computer-use-linux-${process.platform}-${process.arch}`;
const bundledBinary = path.join(__dirname, binaryName);
const binary = process.env.COMPUTER_USE_LINUX_BIN || bundledBinary;
const bundledCosmicHelper = path.join(__dirname, 'computer-use-linux-cosmic');
const bundledIndicator = path.join(__dirname, 'computer-use-linux-indicator');

// npm 12 skips install scripts it has not been told to allow, so a global
// install can arrive without the native binary. Run the installer on first
// launch. Its output goes to stderr: stdout is the MCP channel.
function installBinary() {
  const installer = path.join(__dirname, '..', 'install.js');
  const result = spawnSync(process.execPath, [installer], { stdio: ['ignore', 2, 2] });
  return result.status === 0 && fs.existsSync(binary);
}

if (!fs.existsSync(binary) && (process.env.COMPUTER_USE_LINUX_BIN || !installBinary())) {
  console.error(
    [
      `computer-use-linux binary not found: ${binary}`,
      'Run `node npm/install.js` in the package directory, allow its install script, or set COMPUTER_USE_LINUX_BIN.',
    ].join('\n')
  );
  process.exit(127);
}

const env = { ...process.env };
if (!env.COMPUTER_USE_LINUX_COSMIC_HELPER && fs.existsSync(bundledCosmicHelper)) {
  env.COMPUTER_USE_LINUX_COSMIC_HELPER = bundledCosmicHelper;
}
if (!env.COMPUTER_USE_LINUX_INDICATOR_BIN && fs.existsSync(bundledIndicator)) {
  env.COMPUTER_USE_LINUX_INDICATOR_BIN = bundledIndicator;
}

const child = spawn(binary, process.argv.slice(2), {
  stdio: 'inherit',
  env,
});

for (const signal of ['SIGINT', 'SIGTERM', 'SIGHUP']) {
  process.on(signal, () => {
    if (!child.killed) {
      child.kill(signal);
    }
  });
}

child.on('error', (error) => {
  console.error(`failed to start computer-use-linux: ${error.message}`);
  process.exit(127);
});

child.on('exit', (code, signal) => {
  if (signal) {
    const signalExitCodes = {
      SIGHUP: 129,
      SIGINT: 130,
      SIGTERM: 143,
    };
    process.exit(signalExitCodes[signal] || 1);
  }
  process.exit(code ?? 1);
});

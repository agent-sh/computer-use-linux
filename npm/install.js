#!/usr/bin/env node
'use strict';

const crypto = require('node:crypto');
const fs = require('node:fs');
const https = require('node:https');
const os = require('node:os');
const path = require('node:path');

const pkg = require('../package.json');

const archToTarget = {
  x64: 'x86_64',
  arm64: 'aarch64',
};

const binDir = path.join(__dirname, 'bin');
const binaryPath = path.join(binDir, `computer-use-linux-${process.platform}-${process.arch}`);
const cosmicHelperPath = path.join(binDir, 'computer-use-linux-cosmic');
const indicatorPath = path.join(binDir, 'computer-use-linux-indicator');
// A request that receives no bytes for this long is stalled. The wrapper
// installs on first launch, so a stalled download would otherwise hold the
// MCP client's startup open and outlive it when the client gives up.
const IDLE_TIMEOUT_MS = 30_000;

// Messages can carry paths and URLs from the environment. JSON string
// escaping turns CR, LF and the other C0 control characters (U+0000-U+001F)
// into visible escapes, so a value with a newline cannot forge a log line.
function say(write, message) {
  const safe = JSON.stringify(String(message)).slice(1, -1);
  write(`[computer-use-linux] ${safe}`);
}

function fail(message) {
  say(console.error, message);
  process.exit(1);
}

// Write next to the destination, then rename over it, so a server starting
// at the same time (the wrapper installs on first launch) never runs a
// half-written binary.
function place(source, destination) {
  const staging = `${destination}.tmp-${process.pid}-${crypto.randomBytes(4).toString('hex')}`;
  try {
    fs.copyFileSync(source, staging);
    fs.chmodSync(staging, 0o755);
    fs.renameSync(staging, destination);
  } finally {
    fs.rmSync(staging, { force: true });
  }
}

function copyLocalBinary(source) {
  fs.mkdirSync(binDir, { recursive: true });
  // Main binary last, as in main(): its presence tells the wrapper the
  // helpers are in place.
  if (process.env.COMPUTER_USE_LINUX_LOCAL_COSMIC_HELPER) {
    place(process.env.COMPUTER_USE_LINUX_LOCAL_COSMIC_HELPER, cosmicHelperPath);
  }
  if (process.env.COMPUTER_USE_LINUX_LOCAL_INDICATOR) {
    place(process.env.COMPUTER_USE_LINUX_LOCAL_INDICATOR, indicatorPath);
  }
  place(source, binaryPath);
  say(console.log, `installed local binary from ${source}`);
}

function download(url, destination, redirects = 5) {
  return new Promise((resolve, reject) => {
    // An explicit timer, re-armed on every chunk: Node's socket idle timeout
    // did not fire at the requested time during a stalled TLS handshake.
    let idle = null;
    const disarm = () => clearTimeout(idle);
    const arm = () => {
      disarm();
      idle = setTimeout(() => {
        request.destroy(new Error(`no data for ${IDLE_TIMEOUT_MS / 1000} s: ${url}`));
      }, IDLE_TIMEOUT_MS);
    };
    const request = https.get(url, (response) => {
      arm();
      if (
        response.statusCode >= 300 &&
        response.statusCode < 400 &&
        response.headers.location &&
        redirects > 0
      ) {
        disarm();
        response.resume();
        const nextUrl = new URL(response.headers.location, url).toString();
        download(nextUrl, destination, redirects - 1).then(resolve, reject);
        return;
      }

      if (response.statusCode !== 200) {
        disarm();
        response.resume();
        const error = new Error(`download failed with HTTP ${response.statusCode}: ${url}`);
        error.statusCode = response.statusCode;
        reject(error);
        return;
      }

      const file = fs.createWriteStream(destination, { mode: 0o600 });
      response.on('data', arm);
      response.pipe(file);
      file.on('finish', () => {
        disarm();
        file.close(resolve);
      });
      file.on('error', (error) => {
        disarm();
        reject(error);
      });
    });
    arm();
    request.on('error', (error) => {
      disarm();
      reject(error);
    });
  });
}

// Releases before the overlay omit its asset; only HTTP 404 is optional.
async function downloadOptional(url, destination) {
  try {
    await download(url, destination);
    return true;
  } catch (error) {
    if (error.statusCode === 404) {
      return false;
    }
    throw error;
  }
}

function parseSha256(text) {
  const match = text.match(/\b[a-fA-F0-9]{64}\b/);
  if (!match) {
    throw new Error('sha256 file did not contain a 64-character hex digest');
  }
  return match[0].toLowerCase();
}

function sha256File(filePath) {
  const hash = crypto.createHash('sha256');
  hash.update(fs.readFileSync(filePath));
  return hash.digest('hex');
}

async function main() {
  if (process.env.COMPUTER_USE_LINUX_SKIP_DOWNLOAD === '1') {
    say(console.log, 'skipping binary download');
    return;
  }

  if (process.env.COMPUTER_USE_LINUX_LOCAL_BINARY) {
    copyLocalBinary(process.env.COMPUTER_USE_LINUX_LOCAL_BINARY);
    return;
  }

  if (process.platform !== 'linux') {
    fail(`unsupported platform: ${process.platform}. This package only supports Linux.`);
  }

  const targetArch = archToTarget[process.arch];
  if (!targetArch) {
    fail(`unsupported CPU architecture: ${process.arch}. Supported: x64, arm64.`);
  }

  const asset = `computer-use-linux-${targetArch}-unknown-linux-gnu`;
  const cosmicAsset = `computer-use-linux-cosmic-${targetArch}-unknown-linux-gnu`;
  const indicatorAsset = `computer-use-linux-indicator-${targetArch}-unknown-linux-gnu`;
  const baseUrl =
    process.env.COMPUTER_USE_LINUX_DOWNLOAD_BASE ||
    `https://github.com/agent-sh/computer-use-linux/releases/download/v${pkg.version}`;
  // Refuse before downloading ~15 MB that could not be installed (a global
  // prefix owned by root).
  try {
    fs.accessSync(fs.existsSync(binDir) ? binDir : path.dirname(binDir), fs.constants.W_OK);
  } catch (error) {
    fail(`cannot write ${binDir}: ${error.message}`);
  }

  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'computer-use-linux-'));
  const tmpBinary = path.join(tmpDir, asset);
  const tmpSha = path.join(tmpDir, `${asset}.sha256`);
  const tmpCosmic = path.join(tmpDir, cosmicAsset);
  const tmpCosmicSha = path.join(tmpDir, `${cosmicAsset}.sha256`);
  const tmpIndicator = path.join(tmpDir, indicatorAsset);
  const tmpIndicatorSha = path.join(tmpDir, `${indicatorAsset}.sha256`);

  try {
    say(console.log, `downloading ${asset} from ${baseUrl}`);
    await download(`${baseUrl}/${asset}`, tmpBinary);
    await download(`${baseUrl}/${asset}.sha256`, tmpSha);
    await download(`${baseUrl}/${cosmicAsset}`, tmpCosmic);
    await download(`${baseUrl}/${cosmicAsset}.sha256`, tmpCosmicSha);
    const hasIndicator = await downloadOptional(`${baseUrl}/${indicatorAsset}`, tmpIndicator);
    if (hasIndicator) {
      await download(`${baseUrl}/${indicatorAsset}.sha256`, tmpIndicatorSha);
    }

    const expected = parseSha256(fs.readFileSync(tmpSha, 'utf8'));
    const actual = sha256File(tmpBinary);
    if (actual !== expected) {
      throw new Error(`sha256 mismatch for ${asset}: expected ${expected}, got ${actual}`);
    }

    const expectedCosmic = parseSha256(fs.readFileSync(tmpCosmicSha, 'utf8'));
    const actualCosmic = sha256File(tmpCosmic);
    if (actualCosmic !== expectedCosmic) {
      throw new Error(`sha256 mismatch for ${cosmicAsset}: expected ${expectedCosmic}, got ${actualCosmic}`);
    }

    if (hasIndicator) {
      const expectedIndicator = parseSha256(fs.readFileSync(tmpIndicatorSha, 'utf8'));
      const actualIndicator = sha256File(tmpIndicator);
      if (actualIndicator !== expectedIndicator) {
        throw new Error(`sha256 mismatch for ${indicatorAsset}: expected ${expectedIndicator}, got ${actualIndicator}`);
      }
    }

    // The wrapper only checks for the main binary, so it goes last: once it
    // exists, the helpers it points the server at exist too.
    fs.mkdirSync(binDir, { recursive: true });
    place(tmpCosmic, cosmicHelperPath);
    if (hasIndicator) {
      place(tmpIndicator, indicatorPath);
    } else {
      fs.rmSync(indicatorPath, { force: true });
    }
    place(tmpBinary, binaryPath);
    say(
      console.log,
      `installed ${asset} and ${cosmicAsset}${hasIndicator ? ` and ${indicatorAsset}` : ' (release has no indicator)'}`
    );
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
}

main().catch((error) => fail(error.message));

#!/usr/bin/env node
'use strict';

const { spawnSync } = require('node:child_process');
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

function fail(message) {
  console.error(`[computer-use-linux] ${message}`);
  process.exit(1);
}

function copyLocalBinary(source) {
  fs.mkdirSync(binDir, { recursive: true });
  fs.copyFileSync(source, binaryPath);
  fs.chmodSync(binaryPath, 0o755);
  if (process.env.COMPUTER_USE_LINUX_LOCAL_COSMIC_HELPER) {
    fs.copyFileSync(process.env.COMPUTER_USE_LINUX_LOCAL_COSMIC_HELPER, cosmicHelperPath);
    fs.chmodSync(cosmicHelperPath, 0o755);
  }
  if (process.env.COMPUTER_USE_LINUX_LOCAL_INDICATOR) {
    fs.copyFileSync(process.env.COMPUTER_USE_LINUX_LOCAL_INDICATOR, indicatorPath);
    fs.chmodSync(indicatorPath, 0o755);
  }
  console.log(`[computer-use-linux] installed local binary from ${source}`);
}

function download(url, destination, redirects = 5) {
  return new Promise((resolve, reject) => {
    const request = https.get(url, (response) => {
      if (
        response.statusCode >= 300 &&
        response.statusCode < 400 &&
        response.headers.location &&
        redirects > 0
      ) {
        response.resume();
        const nextUrl = new URL(response.headers.location, url).toString();
        download(nextUrl, destination, redirects - 1).then(resolve, reject);
        return;
      }

      if (response.statusCode !== 200) {
        response.resume();
        const error = new Error(`download failed with HTTP ${response.statusCode}: ${url}`);
        error.statusCode = response.statusCode;
        reject(error);
        return;
      }

      const file = fs.createWriteStream(destination, { mode: 0o600 });
      response.pipe(file);
      file.on('finish', () => file.close(resolve));
      file.on('error', reject);
    });
    request.on('error', reject);
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

function hostGlibcVersion() {
  try {
    return process.report.getReport().header.glibcVersionRuntime || null;
  } catch {
    return null;
  }
}

// Prebuilt binaries link against the glibc of the release build host. On an
// older distribution the dynamic loader rejects them before main() runs, and
// the only message is a terse "GLIBC_x.y not found". Probe the binary once so
// the install log explains the failure and points at a source build instead.
function warnIfBinaryCannotRun(binary) {
  const result = spawnSync(binary, ['--help'], { encoding: 'utf8', timeout: 10000 });
  if (result.error || result.status === 0) {
    return;
  }
  const required = [...new Set(
    [...(result.stderr || '').matchAll(/version `GLIBC_([0-9.]+)' not found/g)].map((m) => m[1])
  )];
  if (required.length === 0) {
    return;
  }
  const host = hostGlibcVersion() || 'unknown';
  console.warn(
    [
      '[computer-use-linux] WARNING: the prebuilt binary cannot run on this system.',
      `[computer-use-linux] It requires glibc ${required.join(', ')}; this system has glibc ${host}.`,
      '[computer-use-linux] Build from source instead (needs a Rust toolchain):',
      '[computer-use-linux]   git clone https://github.com/agent-sh/computer-use-linux && cd computer-use-linux && ./install.sh',
      '[computer-use-linux] or: cargo install computer-use-linux',
      '[computer-use-linux] Then point this wrapper at it with COMPUTER_USE_LINUX_BIN=/path/to/computer-use-linux.',
    ].join('\n')
  );
}

async function main() {
  if (process.env.COMPUTER_USE_LINUX_SKIP_DOWNLOAD === '1') {
    console.log('[computer-use-linux] skipping binary download');
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
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'computer-use-linux-'));
  const tmpBinary = path.join(tmpDir, asset);
  const tmpSha = path.join(tmpDir, `${asset}.sha256`);
  const tmpCosmic = path.join(tmpDir, cosmicAsset);
  const tmpCosmicSha = path.join(tmpDir, `${cosmicAsset}.sha256`);
  const tmpIndicator = path.join(tmpDir, indicatorAsset);
  const tmpIndicatorSha = path.join(tmpDir, `${indicatorAsset}.sha256`);

  try {
    console.log(`[computer-use-linux] downloading ${asset} from ${baseUrl}`);
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

    fs.mkdirSync(binDir, { recursive: true });
    fs.copyFileSync(tmpBinary, binaryPath);
    fs.chmodSync(binaryPath, 0o755);
    fs.copyFileSync(tmpCosmic, cosmicHelperPath);
    fs.chmodSync(cosmicHelperPath, 0o755);
    if (hasIndicator) {
      fs.copyFileSync(tmpIndicator, indicatorPath);
      fs.chmodSync(indicatorPath, 0o755);
    } else {
      fs.rmSync(indicatorPath, { force: true });
    }
    console.log(`[computer-use-linux] installed ${asset} and ${cosmicAsset}${hasIndicator ? ` and ${indicatorAsset}` : ' (release has no indicator)'}`);
    warnIfBinaryCannotRun(binaryPath);
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
}

main().catch((error) => fail(error.message));

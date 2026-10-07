#!/usr/bin/env python3
"""Exercise release installation without network access or a running desktop."""

import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[1]
VERSION = json.loads((ROOT / "package.json").read_text())["version"]
ARCH = {"x86_64": "x86_64", "aarch64": "aarch64", "arm64": "aarch64"}[platform.machine()]
TARGET = f"{ARCH}-unknown-linux-gnu"
NAMES = ["computer-use-linux", "computer-use-linux-cosmic", "computer-use-linux-indicator"]
BINARY = b"#!/bin/sh\nexit 0\n"

# Downloaders read the same synthetic release, distinguishing an absent
# optional asset from a failed request. Neither can contact the network.
DOWNLOADER = r'''import os, pathlib, sys, urllib.parse
args = sys.argv[1:]
url = urllib.parse.urlparse(args[-1])
source = pathlib.Path(urllib.parse.unquote(url.path)) if url.scheme == "file" else pathlib.Path(os.environ["RELEASE_FIXTURE"]) / pathlib.Path(url.path).name
with open(os.environ["REQUEST_LOG"], "a") as log:
    log.write(source.name + "\n")
status = 500 if source.with_name(source.name + ".http500").exists() else 200 if source.exists() else 404
is_wget = pathlib.Path(sys.argv[0]).name == "wget"
destination = pathlib.Path(args[args.index("-O" if is_wget else "-o") + 1])
# wget opens its output even when the server then returns an error.
if is_wget:
    destination.write_bytes(b"")
if "-w" in args:
    print(status, end="")
if status != 200:
    print(f"  HTTP/1.1 {status} fixture: {source.name}", file=sys.stderr)
    sys.exit(8 if is_wget else 22)
destination.write_bytes(source.read_bytes())
'''
HTTPS = r'''const fs = require('node:fs');
const path = require('node:path');
const { EventEmitter } = require('node:events');
const { Readable } = require('node:stream');
require('node:https').get = (url, callback) => {
  const request = new EventEmitter();
  process.nextTick(() => {
    const name = path.basename(new URL(url).pathname);
    const file = path.join(process.env.RELEASE_FIXTURE, name);
    fs.appendFileSync(process.env.REQUEST_LOG, name + '\n');
    if (fs.existsSync(file + '.network-error')) {
      request.emit('error', new Error('fixture network failure'));
      return;
    }
    const status = fs.existsSync(file + '.http500') ? 500 : fs.existsSync(file) ? 200 : 404;
    const response = Readable.from(status === 200 ? [fs.readFileSync(file)] : []);
    response.statusCode = status;
    response.headers = {};
    callback(response);
  });
  return request;
};
'''


def snapshot(directory):
    return {str(p.relative_to(directory)): p.read_bytes() for p in directory.rglob("*") if p.is_file()}


def put(directory, name, data=BINARY):
    directory.mkdir(parents=True, exist_ok=True)
    file = directory / name
    file.write_bytes(data)
    file.chmod(0o755)
    return file


def check(kind, scenario):
    with tempfile.TemporaryDirectory(prefix="cul-packaging-") as temporary:
        base = Path(temporary)
        release = base / "release"
        for name in NAMES:
            asset = put(release, f"{name}-{TARGET}")
            asset.with_name(asset.name + ".sha256").write_text(hashlib.sha256(BINARY).hexdigest())
        indicator = release / f"{NAMES[2]}-{TARGET}"
        if scenario in ("old-release", "cached-old-release"):
            indicator.unlink()
            indicator.with_name(indicator.name + ".sha256").unlink()
        elif scenario == "bad-sha":
            indicator.with_name(indicator.name + ".sha256").write_text("0" * 64)
        elif scenario == "missing-sha":
            indicator.with_name(indicator.name + ".sha256").unlink()
        elif scenario in ("http500", "network-error"):
            indicator.with_name(indicator.name + "." + scenario).touch()
        elif scenario.startswith("missing-"):
            (release / f"{NAMES[int(scenario[-1])]}-{TARGET}").unlink()

        env = {key: value for key, value in os.environ.items() if not key.startswith("COMPUTER_USE_LINUX_")}
        for key in ("ARGV0", "APPIMAGE", "APPDIR", "LD_LIBRARY_PATH", "LD_PRELOAD", "NODE_OPTIONS", "DISPLAY", "WAYLAND_DISPLAY"):
            env.pop(key, None)
        env.update(HOME=str(base), XDG_CACHE_HOME=str(base / "cache"), TMPDIR=str(base),
                   RELEASE_FIXTURE=str(release), REQUEST_LOG=str(base / "requests"),
                   DBUS_SESSION_BUS_ADDRESS="unix:path=/nonexistent")
        if kind.startswith("plugin"):
            tools = base / "tools"
            downloader = "wget" if kind == "plugin-wget" else "curl"
            put(tools, downloader, (f"#!{sys.executable}\n" + DOWNLOADER).encode())
            if downloader == "wget":
                # A closed PATH exercises wget fallback even on curl hosts.
                for name in "sh uname mkdir mktemp grep head tr sha256sum cut chmod mv rm touch find".split():
                    (tools / name).symlink_to(shutil.which(name))
                env["PATH"] = str(tools)
            else:
                env["PATH"] = str(tools) + os.pathsep + env["PATH"]
            env["COMPUTER_USE_LINUX_DOWNLOAD_BASE"] = release.as_uri() if kind == "plugin-file" else "https://fixture.invalid/release"
            destination = base / "cache/computer-use-linux/plugin" / f"v{VERSION}"
            command = ["sh", str(ROOT / "plugins/computer-use-linux/bin/computer-use-linux"), "--help"]
            if scenario == "cached-old-release":
                for name in NAMES[:2]:
                    put(destination, name)
                    (release / f"{name}-{TARGET}").unlink()
        else:
            package = base / "package"
            (package / "npm").mkdir(parents=True)
            shutil.copy2(ROOT / "package.json", package / "package.json")
            shutil.copy2(ROOT / "npm/install.js", package / "npm/install.js")
            mock = base / "https.cjs"
            mock.write_text(HTTPS)
            env["COMPUTER_USE_LINUX_DOWNLOAD_BASE"] = "https://fixture.invalid/release"
            destination = package / "npm/bin"
            command = ["node", "--require", str(mock), str(package / "npm/install.js")]
            if scenario == "old-release":
                put(destination, NAMES[2], b"stale indicator")

        success = scenario in ("present", "old-release", "cached-old-release")
        node_arch = "x64" if ARCH == "x86_64" else "arm64"
        installed = NAMES if kind.startswith("plugin") else [f"computer-use-linux-linux-{node_arch}", *NAMES[1:]]
        if not success:
            # Existing files must survive a failed installation unchanged.
            put(destination, "existing-sentinel", b"preserve me")
            if kind == "npm":
                for name in installed:
                    put(destination, name, b"old installed binary")
        before = snapshot(destination)
        result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=15)
        detail = f"{kind}/{scenario}: {result.stdout}\n{result.stderr}"
        assert (result.returncode == 0) == success, detail
        if not success:
            assert snapshot(destination) == before, f"failed installation mutated files: {detail}"
        else:
            files = snapshot(destination)
            for name in installed[:2]:
                assert files.get(name) == BINARY, detail
            assert (NAMES[2] in files) == (scenario == "present"), detail
            if scenario == "present":
                assert files[NAMES[2]] == BINARY, detail
            if kind.startswith("plugin") and scenario != "present":
                assert (destination / ".no-indicator").exists(), detail
                requests = (base / "requests").read_text() if (base / "requests").exists() else ""
                if scenario == "cached-old-release":
                    assert not any(f"{name}-{TARGET}" in requests for name in NAMES[:2]), f"cached binaries were downloaded again: {requests}"
                shutil.rmtree(release)
                again = subprocess.run(command, env=env, capture_output=True, text=True, timeout=15)
                assert again.returncode == 0, f"cached launch failed: {again.stderr}"
                assert ((base / "requests").read_text() if (base / "requests").exists() else "") == requests


def main():
    forwarded = json.loads((ROOT / "plugins/computer-use-linux/codex-mcp.json").read_text())["mcpServers"]["computer-use-linux"]["env_vars"]
    for suffix in ("INDICATOR", "INDICATOR_HIDE_TEXT", "INDICATOR_BIN", "AGENT_NAME"):
        assert "COMPUTER_USE_LINUX_" + suffix in forwarded, f"Codex does not forward {suffix}"
    for kind in ("plugin", "npm"):
        for scenario in ("present", "old-release", "bad-sha", "missing-sha", "http500", "missing-0", "missing-1"):
            check(kind, scenario)
    check("plugin", "cached-old-release")
    check("plugin-file", "old-release")
    check("plugin-wget", "old-release")
    check("plugin-wget", "http500")
    check("npm", "network-error")
    print("indicator packaging: old releases, verified optional assets, failure isolation, cache reuse and Codex env OK")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Opt-in Xvfb/MCP regression for issue #272, isolated from the real desktop.

Requires Xvfb, xterm, xdotool, setxkbmap and dbus-run-session. Checks byte-exact
stable input and detection of a layout replaced and restored within one call.
"""
import json
import os
from pathlib import Path
import select
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time


def rpc(proc, request):
    proc.stdin.write(json.dumps(request) + "\n")
    proc.stdin.flush()
    if "id" not in request:
        return None
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if not select.select([proc.stdout], [], [], 1)[0]:
            continue
        line = proc.stdout.readline()
        if not line:
            raise RuntimeError("MCP exited")
        response = json.loads(line)
        if response.get("id") == request["id"]:
            assert "error" not in response, response
            return response["result"]
    raise TimeoutError("MCP response deadline exceeded")


def main():
    binary = str(Path(sys.argv[1]).resolve())
    cache = Path.home() / ".cache"
    cache.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="cul-x11-type-", dir=cache) as scratch:
        if not os.environ.get("CUL_X11_PRIVATE_BUS"):
            env = os.environ.copy()
            runtime = Path(scratch) / "runtime"
            runtime.mkdir(mode=0o700)
            bus_config = Path(scratch) / "bus.conf"
            # No service directories: this test must not activate desktop
            # portals, mount a document FUSE filesystem, or start host apps.
            bus_config.write_text(
                '<busconfig><type>session</type>'
                f'<listen>unix:tmpdir={runtime}</listen>'
                '<policy context="default"><allow own="*"/>'
                '<allow send_destination="*"/><allow receive_sender="*"/>'
                '</policy></busconfig>')
            env.update(CUL_X11_PRIVATE_BUS="1", XDG_CACHE_HOME=scratch,
                       XDG_CONFIG_HOME=scratch, XDG_RUNTIME_DIR=str(runtime),
                       COMPUTER_USE_LINUX_INDICATOR="0",
                       COMPUTER_USE_LINUX_SCREENSHOT_BACKEND="x11")
            for key in ("AT_SPI_BUS_ADDRESS", "WAYLAND_DISPLAY", "DISPLAY"):
                env.pop(key, None)
            subprocess.run(["dbus-run-session", "--config-file", str(bus_config),
                            "--", sys.executable, __file__, binary],
                           env=env, check=True, timeout=120)
            return
        env = os.environ.copy()
        env.update(XDG_SESSION_TYPE="x11", XDG_CURRENT_DESKTOP="Xvfb",
                   COMPUTER_USE_LINUX_XDOTOOL_TYPE_DELAY_MS="30")
        shim_dir = Path(scratch) / "bin"
        shim_dir.mkdir()
        shim = shim_dir / "xdotool"
        shim.write_text("#!/usr/bin/env python3\nimport os, socket, sys\n"
                        "if sys.argv[1:2] == ['type']:\n"
                        "    with open(os.environ['CUL_X11_DISPATCH_MARKER'], 'a') as marker:\n"
                        "        marker.write('type\\n')\n"
                        "    with socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM) as notify:\n"
                        "        notify.sendto(b'type', os.environ['CUL_X11_NOTIFY'])\n"
                        f"os.execv({shutil.which('xdotool')!r}, sys.argv)\n")
        shim.chmod(0o755)
        env["PATH"] = str(shim_dir) + os.pathsep + env["PATH"]
        read_fd, write_fd = os.pipe()
        xvfb = subprocess.Popen(["Xvfb", "-displayfd", str(write_fd), "-screen", "0",
                                 "800x600x24", "-nolisten", "tcp"],
                                pass_fds=(write_fd,), stdout=subprocess.DEVNULL,
                                stderr=subprocess.DEVNULL)
        os.close(write_fd)
        try:
            assert select.select([read_fd], [], [], 10)[0], "Xvfb did not start"
            env["DISPLAY"] = ":" + os.read(read_fd, 64).decode().strip()
            os.close(read_fd)
            for index, (label, text, flip) in enumerate([
                ("stable-ascii", "br:;'\"[]{}<>~^\\|!@#%&*()_+=?/abcXYZ0198\n", False),
                ("stable-unmapped-symbol", "hello Ω Ж 😀\n", False),
                ("layout-restored-1", "TESTE:;'\"[]{}<>~^\\|!@#%&*()_+=?/abcXYZ0198" * 3, True),
                ("layout-restored-2", "TESTE:;'\"[]{}<>~^\\|!@#%&*()_+=?/abcXYZ0198" * 3, True),
                ("layout-restored-3", "TESTE:;'\"[]{}<>~^\\|!@#%&*()_+=?/abcXYZ0198" * 3, True),
            ]):
                capture = Path(scratch) / f"{index}.txt"
                marker = Path(scratch) / f"{index}.dispatch"
                env["CUL_X11_DISPATCH_MARKER"] = str(marker)
                notify = socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM)
                notify.bind(str(Path(scratch) / f"{index}.sock"))
                env["CUL_X11_NOTIFY"] = notify.getsockname()
                terminal = subprocess.Popen(
                    ["xterm", "-title", "CUL-X11-TYPE-TEST", "-e", "sh", "-c",
                     'stty raw -echo; exec cat > "$1"', "sh", str(capture)], env=env,
                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                mcp = None
                errors = []
                flipper = None
                try:
                    window = subprocess.check_output(
                        ["xdotool", "search", "--sync", "--onlyvisible", "--name", "^CUL-X11-TYPE-TEST$"],
                        env=env, timeout=10).decode().splitlines()[-1]
                    subprocess.run(["xdotool", "windowfocus", "--sync", window],
                                   env=env, check=True, timeout=5)
                    subprocess.run(["setxkbmap", "br"], env=env, check=True, timeout=5)
                    mcp = subprocess.Popen([binary, "mcp"], env=env, stdin=subprocess.PIPE,
                                           stdout=subprocess.PIPE, text=True)
                    rpc(mcp, {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                        "protocolVersion": "2025-11-25", "capabilities": {},
                        "clientInfo": {"name": "x11-type-guard-test", "version": "1"}}})
                    rpc(mcp, {"jsonrpc": "2.0", "method": "notifications/initialized"})
                    if flip:
                        def replace_layout():
                            try:
                                if not select.select([notify], [], [], 15)[0]:
                                    raise TimeoutError("xdotool dispatch did not start")
                                assert notify.recv(64) == b"type"
                                time.sleep(0.5)
                                subprocess.run(["setxkbmap", "us"], env=env, check=True, timeout=5)
                                time.sleep(0.3)
                                subprocess.run(["setxkbmap", "br"], env=env, check=True, timeout=5)
                            except Exception as error:
                                errors.append(error)
                        flipper = threading.Thread(target=replace_layout)
                        flipper.start()
                    result = rpc(mcp, {"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                                       "params": {"name": "type_text", "arguments": {
                                           "text": text, "screenshot_after": not flip}}})
                    if flipper:
                        flipper.join(timeout=20)
                        assert not flipper.is_alive() and not errors, errors
                    output = result.get("structuredContent") or json.loads(
                        next(item["text"] for item in result["content"] if item["type"] == "text"))
                    assert marker.read_text().splitlines() == ["type"], "text was replayed"
                    if flip:
                        assert not output["ok"], output
                        assert "layout changed during type_text" in json.dumps(output), output
                    else:
                        assert output["ok"], output
                        assert "WARNING: X11 keyboard layout" in output["message"], output
                        assert "Field contents were not verified" in output["message"], output
                        assert "screenshot" in output, output
                        assert capture.read_bytes() == text.replace("\n", "\r").encode(), (label, capture.read_bytes())
                    print(f"PASS {label}", flush=True)
                finally:
                    if flipper:
                        flipper.join(timeout=12)
                    if mcp:
                        mcp.terminate()
                        mcp.wait(timeout=5)
                    terminal.terminate()
                    terminal.wait(timeout=5)
                    notify.close()
        finally:
            xvfb.terminate()
            xvfb.wait(timeout=5)


if __name__ == "__main__":
    main()

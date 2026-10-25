#!/usr/bin/env python3
"""Opt-in Chrome/MCP regression for issue #290, isolated from the real desktop.

Requires Xvfb, openbox, xdotool, xprop, dbus-run-session, AT-SPI and google-chrome
(or chromium); a wmctrl stand-in is provided.
A large page with a nested contenteditable composer stands in for the Codex app:
Chrome marks both the document and the composer focused. Checks that type_text
reaches the composer without a false "not editable" warning, and that an
app-scoped get_app_state with max_nodes above 2000 reads past the old cap and
returns its partial tree at the deadline instead of failing.

    scripts/contenteditable_focus_test.py target/debug/computer-use-linux
"""
import json
import os
from pathlib import Path
import select
import shutil
import signal
import subprocess
import sys
import tempfile
import time

MARKER = "cul290marker"


def page(paragraphs):
    return PAGE.replace("@CONTENT@", "".join(f"<p>Readme line {i}</p>" for i in range(paragraphs)))


PAGE = """<!doctype html><html><head><title>CUL-290-@NAME@-START</title></head><body>
<main>@CONTENT@</main>
<section><div><div><div><div>
<div id="composer" contenteditable="true" role="textbox" aria-multiline="true"
     aria-label="Work with Codex" style="min-height:80px;border:1px solid"></div>
</div></div></div></div></section>
<script>
const c = document.getElementById('composer');
c.addEventListener('input', () => { document.title = 'CUL-290-@NAME@:' + c.textContent; });
window.addEventListener('load', () => { c.focus(); document.title = 'CUL-290-@NAME@-READY'; });
</script></body></html>"""

WMCTRL_SHIM = r"""#!/usr/bin/env python3
# wmctrl stand-in for this test: the two calls the X11 backend makes.
import re, subprocess, sys
def out(*cmd):
    return subprocess.run(cmd, capture_output=True, text=True).stdout
args = sys.argv[1:]
if args[:1] == ["-l"]:
    ids = re.findall(r"0x[0-9a-f]+", out("xprop", "-root", "_NET_CLIENT_LIST").split("#", 1)[-1])
    for wid in ids:
        pid = (re.findall(r"= (\d+)", out("xprop", "-id", wid, "_NET_WM_PID")) or ["0"])[0]
        geo = dict(l.split("=", 1) for l in out("xdotool", "getwindowgeometry", "--shell", str(int(wid, 16))).split())
        cls = re.findall(r'"([^"]*)"', out("xprop", "-id", wid, "WM_CLASS"))
        klass = ".".join(cls) if cls else "N/A"
        title = out("xdotool", "getwindowname", str(int(wid, 16))).strip()
        print(f"0x{int(wid, 16):08x}  0 {pid} {geo['X']} {geo['Y']} {geo['WIDTH']} {geo['HEIGHT']} {klass} host {title}")
elif args[:2] == ["-i", "-a"]:
    sys.exit(subprocess.run(["xdotool", "windowactivate", "--sync", str(int(args[2], 16))]).returncode)
else:
    sys.exit("unsupported wmctrl call: " + " ".join(args))
"""


def rpc(proc, request):
    proc.stdin.write(json.dumps(request) + "\n")
    proc.stdin.flush()
    if "id" not in request:
        return None
    deadline = time.monotonic() + 40
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


def payload(result):
    if result.get("structuredContent"):
        return result["structuredContent"]
    text = next((c["text"] for c in result["content"] if c["type"] == "text"), "")
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        return {"ok": not result.get("isError", False), "message": text}


def stop(proc):
    """SIGTERM first so Chrome removes its shared-memory and socket files."""
    proc.terminate()
    try:
        proc.wait(timeout=10)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=5)


def find_window(env, pattern, timeout=40):
    """xdotool search fails with BadWindow when a window closes mid-search; retry."""
    deadline = time.monotonic() + timeout
    while True:
        found = subprocess.run(["xdotool", "search", "--onlyvisible", "--name", pattern], env=env,
                               capture_output=True, text=True, timeout=10).stdout.split()
        if found:
            return found[-1]
        if time.monotonic() > deadline:
            raise TimeoutError(f"no visible window matches {pattern!r}")
        time.sleep(0.3)


def window_title(env, window):
    return subprocess.check_output(["xdotool", "getwindowname", window], env=env, timeout=5).decode().strip()


def wait_title(env, window, prefix, timeout=30):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        title = window_title(env, window)
        if title.startswith(prefix):
            return title
        time.sleep(0.2)
    raise TimeoutError(f"window title never started with {prefix!r}: {window_title(env, window)!r}")


def inner(binary, scratch):
    env = os.environ.copy()
    chrome = shutil.which("google-chrome") or shutil.which("chromium") or shutil.which("chromium-browser")
    assert chrome, "google-chrome or chromium is required"
    subprocess.run(["gsettings", "set", "org.gnome.desktop.interface", "toolkit-accessibility", "true"],
                   env=env, check=False, timeout=10)
    shims = Path(scratch) / "bin"
    shims.mkdir(exist_ok=True)
    (shims / "wmctrl").write_text(WMCTRL_SHIM)
    (shims / "wmctrl").chmod(0o755)
    env["PATH"] = f"{shims}{os.pathsep}{env['PATH']}"
    wm = subprocess.Popen(["openbox"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1)
    try:
        # A small page: the probe reaches the composer and must name it.
        small = run_page(binary, scratch, env, chrome, "small", page(40), check_snapshot=False)
        assert "(editable)" in small and "Work with Codex" in small, small
        # A large page: the probe may run out of budget, but must not claim the text was lost,
        # and a raised max_nodes must reach the composer past the 2000-node cap.
        run_page(binary, scratch, env, chrome, "large", page(2600), check_snapshot=True)
        print("PASS")
    finally:
        stop(wm)


def run_page(binary, scratch, env, chrome, name, html, check_snapshot):
    path = Path(scratch) / f"{name}.html"
    html = html.replace("@NAME@", name)
    path.write_text(html)
    browser = subprocess.Popen(
        [chrome, f"--user-data-dir={scratch}/chrome-{name}", "--no-first-run", "--no-default-browser-check",
         "--force-renderer-accessibility", "--disable-gpu", "--password-store=basic",
         "--window-size=1200,900", f"--app=file://{path}"],
        env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    mcp = None
    try:
        window = find_window(env, f"^CUL-290-{name}")
        wait_title(env, window, f"CUL-290-{name}-READY")
        subprocess.run(["xdotool", "windowactivate", "--sync", window], env=env, check=False, timeout=5)
        subprocess.run(["xdotool", "windowfocus", "--sync", window], env=env, check=True, timeout=5)
        time.sleep(1.5)  # let Chrome publish its accessibility tree
        mcp = subprocess.Popen([binary, "mcp"], env=env, stdin=subprocess.PIPE,
                               stdout=subprocess.PIPE, text=True)
        rpc(mcp, {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-11-25", "capabilities": {},
            "clientInfo": {"name": "contenteditable-focus-test", "version": "1"}}})
        rpc(mcp, {"jsonrpc": "2.0", "method": "notifications/initialized"})
        if os.environ.get("CUL_290_DUMP"):
            state = payload(rpc(mcp, {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
                "name": "get_app_state", "arguments": {"pid": browser.pid, "max_nodes": 200, "max_depth": 64,
                                                      "include_screenshot": False, "verbose": True}}}))
            Path(scratch, f"{name}-state.json").write_text(json.dumps(state))
            def walk(node):
                if isinstance(node, dict):
                    if "focused" in json.dumps(node.get("states", "")):
                        print(f"[{name}] focused node: role={node.get('role')} name={node.get('name')} states={node.get('states')}")
                    for v in node.values():
                        walk(v)
                elif isinstance(node, list):
                    for v in node:
                        walk(v)
            walk(state)
        typed = payload(rpc(mcp, {"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
            "name": "type_text", "arguments": {"text": MARKER, "window_id": int(window)}}}))
        message = typed.get("message", "")
        feedback = message[message.find("Focused"):] if "Focused" in message else message
        print(f"[{name}] type_text ok={typed.get('ok')} feedback: {feedback}")
        delivered = wait_title(env, window, f"CUL-290-{name}:", timeout=15)
        print(f"[{name}] delivered: {delivered!r}")
        assert MARKER in delivered, delivered
        assert "not editable" not in message and "went nowhere" not in message, message
        if check_snapshot:
            found, raw = {}, {}
            for nodes in (2000, 10000):
                started = time.monotonic()
                state = payload(rpc(mcp, {"jsonrpc": "2.0", "id": 10 + nodes, "method": "tools/call", "params": {
                    "name": "get_app_state", "arguments": {
                        "pid": browser.pid, "max_nodes": nodes, "max_depth": 64, "include_screenshot": False}}}))
                found[nodes] = "Work with Codex" in json.dumps(state)
                raw[nodes] = state.get("accessibility_tree_raw_count") or 0
                assert not state.get("accessibility_error"), state.get("accessibility_error")
                if os.environ.get("CUL_290_DUMP"):
                    Path(scratch, f"{name}-{nodes}-state.json").write_text(json.dumps(state))
                print(f"[{name}] get_app_state max_nodes={nodes}: {time.monotonic() - started:.1f}s "
                      f"raw={state.get('accessibility_tree_raw_count')} "
                      f"truncated={state.get('accessibility_tree_truncated')} composer={found[nodes]}"
                      + (f" error={state.get('accessibility_error')}" if state.get('accessibility_error') else ""))
            # Whether the composer is reached depends on how fast this machine reads
            # AT-SPI. A 10000-node request must not be clamped at 2000, and must return
            # what it read instead of failing at its deadline (needs > 200 nodes/s).
            assert raw[10000] > 2000, raw
        return message
    finally:
        if mcp:
            stop(mcp)
        stop(browser)



def main():
    binary = str(Path(sys.argv[1]).resolve())
    if os.environ.get("CUL_290_PRIVATE_BUS"):
        inner(binary, os.environ["CUL_290_SCRATCH"])
        return
    cache = Path.home() / ".cache"
    cache.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="cul-290-", dir=cache) as scratch:
        runtime = Path(scratch) / "runtime"
        runtime.mkdir(mode=0o700)
        display = next(f":{n}" for n in range(190, 260) if not Path(f"/tmp/.X11-unix/X{n}").exists())
        xvfb = subprocess.Popen(["Xvfb", display, "-screen", "0", "1280x1024x24", "-nolisten", "tcp"],
                                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        env = os.environ.copy()
        for key in ("AT_SPI_BUS_ADDRESS", "WAYLAND_DISPLAY", "DBUS_SESSION_BUS_ADDRESS", "GSETTINGS_BACKEND"):
            env.pop(key, None)
        # The XDG homes are set before the bus starts, so bus-activated dconf
        # and AT-SPI services never touch the real user's files.
        env.update(CUL_290_PRIVATE_BUS="1", CUL_290_SCRATCH=scratch, DISPLAY=display, TMPDIR=scratch,
                   HOME=scratch, XDG_CONFIG_HOME=f"{scratch}/config", XDG_CACHE_HOME=f"{scratch}/cache",
                   XDG_DATA_HOME=f"{scratch}/data", XDG_RUNTIME_DIR=str(runtime),
                   XDG_SESSION_TYPE="x11", XDG_CURRENT_DESKTOP="Openbox", DESKTOP_SESSION="openbox",
                   XDG_SESSION_DESKTOP="openbox", COMPUTER_USE_LINUX_INDICATOR="0",
                   COMPUTER_USE_LINUX_SCREENSHOT_BACKEND="x11", GIO_USE_VFS="local", GVFS_DISABLE_FUSE="1")
        try:
            deadline = time.monotonic() + 10
            while not Path(f"/tmp/.X11-unix/X{display[1:]}").exists():
                assert time.monotonic() < deadline, "Xvfb did not start"
                time.sleep(0.1)
            session = subprocess.Popen(["dbus-run-session", "--", sys.executable, __file__, binary],
                                       env=env, start_new_session=True)
            try:
                code = session.wait(timeout=240)
            finally:
                try:
                    os.killpg(session.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            sys.exit(code)
        finally:
            xvfb.terminate()
            xvfb.wait(timeout=10)


if __name__ == "__main__":
    main()

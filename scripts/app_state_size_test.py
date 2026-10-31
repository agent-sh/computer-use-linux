#!/usr/bin/env python3
"""Opt-in Chrome/MCP regression for issue #338, isolated from the real desktop.

Requires the same tools as contenteditable_focus_test.py: Xvfb, openbox,
xdotool, xprop, dbus-run-session, AT-SPI and google-chrome (or chromium).
Opens two windows of one Chrome process, so both sit under one AT-SPI
application root, and checks that get_app_state:

- scoped by title reads only that window;
- returns compact nodes by default and full nodes with verbose=true;
- with interactive_only=true keeps the controls, headings and status region,
  and an element_index from it presses the right button.

Prints bytes per node for each encoding. CUL_338_MEASURE_ONLY=1 prints the
numbers without the checks, for measuring an older binary.

    scripts/app_state_size_test.py target/debug/computer-use-linux
"""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
from contenteditable_focus_test import (  # noqa: E402
    WMCTRL_SHIM, find_window, payload, rpc, stop, wait_title, wire_bytes)

import shutil  # noqa: E402

MEASURE_ONLY = bool(os.environ.get("CUL_338_MEASURE_ONLY"))


def page(name, marker):
    cards = "".join(
        f"<div class=row><div class=cell><section><h2>{name} card {i}</h2>"
        f"<p>{marker} paragraph {i} with <a href='#c{i}'>a link</a> inside.</p>"
        f"<button onclick=\"document.title='CUL-338-{name}-pressed-{i}'\">Press {i}</button>"
        f"</section></div></div>"
        for i in range(30))
    nav = "".join(f"<li><a href='#n{i}'>Nav {i}</a></li>" for i in range(15))
    return f"""<!doctype html><html><head><title>CUL-338-{name}-START</title></head><body>
<header><nav><ul>{nav}</ul></nav></header>
<main><h1>{name} dashboard</h1>{cards}
<form><label>Search <input id=q></label> <select><option>One</option><option>Two</option></select>
<label><input type=checkbox> Remember</label> <textarea aria-label=Notes></textarea>
<input type=range min=0 max=10 value=4 aria-label=Level></form>
<div role=status>{name} saved</div></main>
<script>window.addEventListener('load', () => {{ document.title = 'CUL-338-{name}-READY'; }});</script>
</body></html>"""


def tree_of(state):
    """The whole tree: the spill file when the result was cut, else the inline nodes."""
    spill = state.get("accessibility_tree_spill")
    if spill:
        return json.loads(Path(spill["path"]).read_text()), spill["bytes"]
    nodes = state.get("accessibility_tree") or []
    return nodes, wire_bytes(nodes)


def app_state(mcp, request_id, **arguments):
    arguments.setdefault("include_screenshot", False)
    arguments.setdefault("max_nodes", 4000)
    arguments.setdefault("max_depth", 64)
    return payload(rpc(mcp, {"jsonrpc": "2.0", "id": request_id, "method": "tools/call", "params": {
        "name": "get_app_state", "arguments": arguments}}))


def report(label, state):
    nodes, size = tree_of(state)
    per_node = size / max(len(nodes), 1)
    print(f"[{label}] raw={state.get('accessibility_tree_raw_count')} nodes={len(nodes)} "
          f"tree_bytes={size} bytes_per_node={per_node:.0f} result_bytes={wire_bytes(state)} "
          f"truncated={state.get('accessibility_tree_truncated')}")
    return nodes, size


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
    paths = {}
    for name, marker in (("A", "AlphaWindowCopy"), ("B", "BetaWindowCopy")):
        paths[name] = Path(scratch) / f"{name}.html"
        paths[name].write_text(page(name, marker))
    flags = [f"--user-data-dir={scratch}/chrome", "--no-first-run", "--no-default-browser-check",
             "--force-renderer-accessibility", "--disable-gpu", "--password-store=basic",
             "--window-size=1200,900"]
    browser = subprocess.Popen([chrome, *flags, f"--app=file://{paths['A']}"], env=env,
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    mcp = None
    try:
        window_a = find_window(env, "^CUL-338-A")
        wait_title(env, window_a, "CUL-338-A-READY")
        # The second launch hands its window to the running browser and exits.
        subprocess.run([chrome, *flags, f"--app=file://{paths['B']}"], env=env, timeout=30,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
        window_b = find_window(env, "^CUL-338-B")
        wait_title(env, window_b, "CUL-338-B-READY")
        time.sleep(2)  # let Chrome publish both accessibility trees
        mcp = subprocess.Popen([binary, "mcp"], env=env, stdin=subprocess.PIPE,
                               stdout=subprocess.PIPE, text=True)
        rpc(mcp, {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-11-25", "capabilities": {},
            "clientInfo": {"name": "app-state-size-test", "version": "1"}}})
        rpc(mcp, {"jsonrpc": "2.0", "method": "notifications/initialized"})

        # Chrome publishes a renderer tree some time after the page loads.
        deadline = time.monotonic() + 30
        request_id = 100
        while True:
            app = app_state(mcp, request_id, pid=browser.pid)
            text = json.dumps(tree_of(app)[0])
            if ("AlphaWindowCopy" in text and "BetaWindowCopy" in text) or time.monotonic() > deadline:
                break
            request_id += 1
            time.sleep(1)
        app_nodes, _ = report("pid, both windows", app)
        scoped = app_state(mcp, 3, title="CUL-338-A")
        nodes, compact_size = report("title, default", scoped)
        full = app_state(mcp, 4, title="CUL-338-A", verbose=True)
        full_nodes, full_size = report("title, verbose", full)
        interactive = app_state(mcp, 5, title="CUL-338-A", interactive_only=True)
        kept, interactive_size = report("title, interactive_only", interactive)
        if os.environ.get("CUL_338_DUMP"):
            Path(os.environ["CUL_338_DUMP"]).write_text(json.dumps(
                {"full": full_nodes, "compact": nodes, "interactive": kept}))
        if MEASURE_ONLY:
            return

        text = json.dumps(nodes)
        assert "AlphaWindowCopy" in text, "the titled window's content is in its tree"
        assert "BetaWindowCopy" not in text, "a title target reads only that window"
        assert "BetaWindowCopy" in json.dumps(app_nodes), "a pid target still reads every window"
        assert "several windows" in scoped["message"], scoped["message"]
        assert len(full_nodes) == len(nodes)
        assert compact_size < full_size, (compact_size, full_size)
        assert all(isinstance(action, str) for node in nodes for action in node.get("actions", []))
        assert all(isinstance(action, dict) for node in full_nodes for action in node["actions"])
        assert not any("value" in node and node["value"] is None for node in nodes)

        roles = {node["role"] for node in kept}
        assert {"heading", "button", "entry", "statusbar", "link"} <= roles, roles
        assert "paragraph" not in roles and "section" not in roles, roles
        assert interactive_size < compact_size, (interactive_size, compact_size)
        indices = {node["index"] for node in kept}
        assert all(node.get("parent_index") is None or node["parent_index"] in indices for node in kept)
        button = next(node for node in kept if node.get("name") == "Press 7")
        assert button["index"] in {node["index"] for node in nodes}
        pressed = payload(rpc(mcp, {"jsonrpc": "2.0", "id": 6, "method": "tools/call", "params": {
            "name": "perform_action", "arguments": {"element_index": button["index"]}}}))
        assert pressed.get("ok"), pressed
        wait_title(env, window_a, "CUL-338-A-pressed-7", timeout=10)
        print("PASS")
    finally:
        if mcp:
            stop(mcp)
        stop(browser)
        stop(wm)


def main():
    binary = str(Path(sys.argv[1]).resolve())
    if os.environ.get("CUL_338_PRIVATE_BUS"):
        inner(binary, os.environ["CUL_338_SCRATCH"])
        return
    cache = Path.home() / ".cache"
    cache.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="cul-338-", dir=cache) as scratch:
        runtime = Path(scratch) / "runtime"
        runtime.mkdir(mode=0o700)
        display = next(f":{n}" for n in range(190, 260) if not Path(f"/tmp/.X11-unix/X{n}").exists())
        xvfb = subprocess.Popen(["Xvfb", display, "-screen", "0", "1280x1024x24", "-nolisten", "tcp"],
                                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        env = os.environ.copy()
        for key in ("AT_SPI_BUS_ADDRESS", "WAYLAND_DISPLAY", "DBUS_SESSION_BUS_ADDRESS", "GSETTINGS_BACKEND"):
            env.pop(key, None)
        env.update(CUL_338_PRIVATE_BUS="1", CUL_338_SCRATCH=scratch, DISPLAY=display, TMPDIR=scratch,
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
                code = session.wait(timeout=300)
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

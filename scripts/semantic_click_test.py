#!/usr/bin/env python3
"""Opt-in real GTK/MCP regression. Run in an isolated workspace display.

Uses a private bus/config/runtime and disables all physical pointer backends.
Checks actual button activation at GTK scales 1 and 2, not just a response string.
"""
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import tempfile
import time


def rpc(proc, request):
    proc.stdin.write(json.dumps(request) + '\n')
    proc.stdin.flush()
    if 'id' not in request:
        return
    deadline = time.monotonic() + 25
    while time.monotonic() < deadline:
        if not select.select([proc.stdout], [], [], 1)[0]:
            continue
        line = proc.stdout.readline()
        if not line:
            raise RuntimeError('MCP exited')
        response = json.loads(line)
        if response.get('id') == request['id']:
            assert 'error' not in response, response
            result = response['result']
            assert not result.get('isError'), result
            return result
    raise TimeoutError('MCP response timeout')


def payload(result):
    return result.get('structuredContent') or json.loads(next(c['text'] for c in result['content'] if c['type'] == 'text'))


def main():
    binary = str(Path(sys.argv[1]).resolve())
    if not os.environ.get('CUL_CLICK_PRIVATE_BUS'):
        with tempfile.TemporaryDirectory(prefix='cul-click-test-', dir='/tmp') as tmp:
            env = os.environ.copy()
            env.update(CUL_CLICK_PRIVATE_BUS='1', XDG_CONFIG_HOME=tmp+'/config', XDG_RUNTIME_DIR=tmp+'/runtime', GIO_USE_VFS='local', GVFS_DISABLE_FUSE='1', CU_DISABLE_ABS_POINTER='1', COMPUTER_USE_LINUX_FORCE_YDOTOOL_POINTER='1', YDOTOOL_SOCKET=tmp+'/no-pointer.sock', XDG_SESSION_TYPE='x11', XDG_CURRENT_DESKTOP='Openbox')
            for key in ('AT_SPI_BUS_ADDRESS', 'WAYLAND_DISPLAY', 'GSETTINGS_BACKEND'):
                env.pop(key, None)
            for folder in ('config', 'runtime'):
                Path(tmp, folder).mkdir(mode=0o700)
            session = subprocess.Popen(['dbus-run-session', '--', '/usr/bin/python3', __file__, binary], env=env, start_new_session=True)
            try:
                assert session.wait(timeout=90) == 0, 'private integration session failed'
            finally:
                # Own the complete test process group, including bus-activated helpers.
                try:
                    os.killpg(session.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                try:
                    session.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(session.pid, signal.SIGKILL)
                    session.wait()
                # The session leader may exit before a descendant. Do not leave
                # a test app that ignored SIGTERM behind in the isolated display.
                try:
                    os.killpg(session.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
        return
    subprocess.run(['gsettings', 'set', 'org.gnome.desktop.interface', 'toolkit-accessibility', 'true'], check=True)
    for scale in ('1', '2'):
        with tempfile.TemporaryFile(mode='w+') as evidence:
            code = '''import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, GLib
GLib.set_prgname("cul-semantic-test")
w=Gtk.Window(title="CUL semantic test")
b=Gtk.Button(label="CLICKME")
b.set_size_request(320,120)
b.connect("clicked", lambda *args: print("ACTIVATED", flush=True))
entry=Gtk.Entry()
entry.get_accessible().set_name("ENTRY_TEST")
entry.connect("activate", lambda *args: print("SUBMITTED", flush=True))
box=Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
box.pack_start(b, True, True, 0)
box.pack_start(entry, False, False, 0)
w.add(box)
w.show_all()
Gtk.main()
'''
            app = mcp = None
            try:
                app = subprocess.Popen(['/usr/bin/python3', '-c', code], env={**os.environ, 'GDK_SCALE':scale}, stdout=evidence)
                mcp = subprocess.Popen([binary, 'mcp'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
                rpc(mcp, {'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'2024-11-05','capabilities':{},'clientInfo':{'name':'semantic-click-regression','version':'1'}}})
                rpc(mcp, {'jsonrpc':'2.0','method':'notifications/initialized'})
                for attempt in range(10):
                    state = payload(rpc(mcp, {'jsonrpc':'2.0','id':10+attempt,'method':'tools/call','params':{'name':'get_app_state','arguments':{'pid':app.pid,'include_screenshot':False}}}))
                    nodes = [n for n in state['accessibility_tree'] if n.get('name') == 'CLICKME']
                    if nodes:
                        break
                    time.sleep(.2)
                assert len(nodes) == 1, state
                result = payload(rpc(mcp, {'jsonrpc':'2.0','id':30,'method':'tools/call','params':{'name':'click','arguments':{'element_index':nodes[0]['index']}}}))
                assert result['ok'] and 'AT-SPI action' in result['message'], result
                for attempt in range(20):
                    evidence.seek(0)
                    if evidence.read().strip() == 'ACTIVATED':
                        break
                    time.sleep(.05)
                else:
                    raise AssertionError('Button did not activate')
                print(json.dumps({'scale':scale,'bounds':nodes[0]['bounds'],'activated':True,'message':result['message']}), flush=True)
                blocked = payload(rpc(mcp, {'jsonrpc':'2.0','id':31,'method':'tools/call','params':{'name':'click','arguments':{'x':0,'y':0}}}))
                assert not blocked['ok'], 'physical pointer fallback was not disabled'
                entries = [n for n in state['accessibility_tree'] if n.get('name') == 'ENTRY_TEST']
                assert len(entries) == 1, state
                entry_result = payload(rpc(mcp, {'jsonrpc':'2.0','id':32,'method':'tools/call','params':{'name':'click','arguments':{'element_index':entries[0]['index']}}}))
                assert not entry_result['ok'], 'entry click bypassed the disabled pointer path'
                time.sleep(.1)
                evidence.seek(0)
                assert evidence.read().strip() == 'ACTIVATED', 'entry click submitted the form'
            finally:
                for proc in (mcp, app):
                    if proc is None:
                        continue
                    proc.terminate()
                    try:
                        proc.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        proc.kill()
                        proc.wait()


if __name__ == '__main__':
    main()

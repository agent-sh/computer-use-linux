#!/usr/bin/python3
"""Opt-in real MCP/XTEST and GTK drag checks on two private Xvfb displays.

Requires Xvfb, xdotool, bwrap, strace, dbus-run-session and GTK 3 PyGObject.
The MCP server has a private /dev without uinput, a private bus without desktop
services, and denied global-input helpers. The owner's display is never used.
"""
import contextlib
import json
import os
from pathlib import Path
import select
import subprocess
import sys
import tempfile
import time


def wait_for(probe, timeout=5):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if probe():
            return
        time.sleep(.01)
    raise TimeoutError('fixture condition did not arrive')


def stop(proc):
    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=5)


def app(root):
    import gi
    gi.require_version('Gtk', '3.0')
    gi.require_version('Gdk', '3.0')
    from gi.repository import Gdk, GLib, Gtk
    log = root / 'events'
    def record(value):
        with log.open('a') as file:
            file.write(json.dumps(value) + '\n')
    window = Gtk.Window(title='CUL private pointer fixture')
    window.set_decorated(False)
    window.set_default_size(800, 600)
    window.move(0, 0)
    fixed = Gtk.Fixed()
    window.add(fixed)
    def box(text, x, y, width, height):
        widget = Gtk.EventBox()
        widget.add(Gtk.Label(label=text))
        widget.set_size_request(width, height)
        fixed.put(widget, x, y)
        return widget
    click = box('Click target', 50, 20, 200, 80)
    click.connect('button-press-event', lambda _, event: record({'click': [event.x_root, event.y_root]}) if event.type == Gdk.EventType.BUTTON_PRESS else False)
    source = box('Drag this payload', 50, 160, 200, 200)
    target = box('Drop here', 450, 160, 200, 200)
    entries = [Gtk.TargetEntry.new('text/plain', 0, 0)]
    source.drag_source_set(Gdk.ModifierType.BUTTON1_MASK, entries, Gdk.DragAction.COPY)
    source.connect('drag-data-get', lambda _, context, data, info, stamp: data.set_text('cul-drag-payload', -1))
    target.drag_dest_set(Gtk.DestDefaults.ALL, entries, Gdk.DragAction.COPY)
    def received(_, context, x, y, data, info, stamp):
        record({'drop': data.get_text()})
        target.get_child().set_text('Received: ' + data.get_text())
    target.connect('drag-data-received', received)
    window.show_all()
    GLib.idle_add(lambda: (root / 'ready').touch() or False)
    Gtk.main()


def rpc(proc, ident, method, params):
    proc.stdin.write(json.dumps(dict(jsonrpc='2.0', id=ident, method=method, params=params)) + '\n')
    proc.stdin.flush()
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        if select.select([proc.stdout], [], [], .1)[0]:
            line = proc.stdout.readline()
            if not line:
                raise RuntimeError('MCP exited')
            response = json.loads(line)
            if response.get('id') == ident:
                assert 'error' not in response, response
                return response['result']
    raise TimeoutError(method)


def xvfb(env, cleanup):
    read, write = os.pipe()
    proc = subprocess.Popen(['Xvfb', '-displayfd', str(write), '-screen', '0', '800x600x24', '-nolisten', 'tcp'],
                            pass_fds=(write,), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    cleanup.callback(stop, proc)
    os.close(write)
    try:
        assert select.select([read], [], [], 10)[0], 'Xvfb startup timed out'
        return ':' + os.read(read, 64).decode().strip()
    finally:
        os.close(read)


def run(binary, root):
    failures = []
    with contextlib.ExitStack() as cleanup:
        env = dict(os.environ, XDG_SESSION_TYPE='x11', XDG_CURRENT_DESKTOP='Xvfb',
                   GDK_BACKEND='x11', COMPUTER_USE_LINUX_INDICATOR='0', COMPUTER_USE_LINUX_SCREENSHOT_BACKEND='x11',
                   AT_SPI_BUS_ADDRESS='unix:path=' + str(root / 'no-a11y'))
        for key in tuple(env):
            if key in ('WAYLAND_DISPLAY', 'WAYLAND_SOCKET', 'DISPLAY', 'CU_DISABLE_ABS_POINTER') or key.startswith('COMPUTER_USE_LINUX_FORCE_'):
                env.pop(key, None)
        env['DISPLAY'] = xvfb(env, cleanup)
        other = dict(env, DISPLAY=xvfb(env, cleanup))
        def xd(*args, target=env):
            return subprocess.check_output(['/usr/bin/xdotool', *map(str, args)], env=target, timeout=5).decode()
        before_other = xd('getmouselocation', '--shell', target=other)
        fixture = subprocess.Popen(['/usr/bin/python3', __file__, '--app', str(root)], env=env,
                                   stdout=subprocess.DEVNULL)
        cleanup.callback(stop, fixture)
        wait_for(lambda: (root / 'ready').exists())
        def events():
            return [json.loads(line) for line in (root / 'events').read_text().splitlines()] if (root / 'events').exists() else []
        def drops():
            return sum('drop' in event for event in events())
        # Prove the GTK fixture accepts the measured 20-step, 30ms gesture.
        xd('mousemove', '100', '250', 'mousedown', '1')
        try:
            for step in range(1, 21):
                xd('mousemove', 100 + 25 * step, 250)
                time.sleep(.03)
        finally:
            xd('mouseup', '1')
        wait_for(lambda: drops() == 1)
        shim = root / 'bin'
        shim.mkdir()
        for helper in ('ydotool', 'wtype', 'systemctl', 'gnome-screenshot'):
            path = shim / helper
            path.write_text('#!/bin/sh\nexit 1\n' if helper == 'systemctl' else '#!/bin/sh\nprintf "%s\\n" "$0" >> "$CUL_FORBIDDEN_LOG"\nexit 99\n')
            path.chmod(0o700)
        env.update(PATH=str(shim) + ':' + env['PATH'], CUL_FORBIDDEN_LOG=str(root / 'forbidden'))
        trace = root / 'opens'
        errors = cleanup.enter_context((root / 'server-errors').open('w'))
        command = ['/usr/bin/bwrap', '--die-with-parent', '--ro-bind', '/', '/', '--dev', '/dev',
                   '--bind', str(root), str(root), '--', '/usr/bin/strace', '-f', '-e', 'trace=openat',
                   '-o', str(trace), binary, 'mcp']
        proc = subprocess.Popen(command, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                stderr=errors, text=True)
        cleanup.callback(stop, proc)
        rpc(proc, 1, 'initialize', dict(protocolVersion='2024-11-05', capabilities={}, clientInfo=dict(name='private-x11-pointer', version='1')))
        proc.stdin.write(json.dumps(dict(jsonrpc='2.0', method='notifications/initialized')) + '\n')
        proc.stdin.flush()
        ident = 1
        def call(name, arguments):
            nonlocal ident
            ident += 1
            result = rpc(proc, ident, 'tools/call', dict(name=name, arguments=arguments))
            payload = result.get('structuredContent')
            if payload is None:
                payload = next(json.loads(item['text']) for item in result['content'] if item['type'] == 'text')
            return result, payload
        for repeat in range(2):
            if repeat:
                xd('mousemove', 200, 60)
            result, output = call('click', dict(x=100, y=50))
            assert output['ok'], output
            wait_for(lambda: sum('click' in event for event in events()) >= repeat + 1)
            assert [event['click'] for event in events() if 'click' in event][-1] == [100, 50], events()
        result, output = call('drag', dict(start_x=100, start_y=250, end_x=600, end_y=250))
        if not output['ok']:
            failures.append('X11 drag did not dispatch through XTEST: ' + output['message'])
        else:
            try:
                wait_for(lambda: drops() == 2)
            except TimeoutError:
                failures.append('MCP drag did not deliver the GTK payload')
        # A failed mid-drag motion must still release, without replay.
        wrapper = shim / 'xdotool'
        wrapper.write_text('#!/usr/bin/python3\nimport os,sys\n'
                           'from pathlib import Path\n'
                           'args=sys.argv[1:]\n'
                           f'with open({str(root / "xdotool-calls")!r}, "a") as log: log.write(" ".join(args)+"\\n")\n'
                           'if len(args)==4 and args[:2]==["mousemove","--"] and args[3]=="250" and int(args[2])>=150: sys.exit(9)\n'
                           'os.execv("/usr/bin/xdotool", sys.argv)\n')
        wrapper.chmod(0o700)
        result, output = call('drag', dict(start_x=100, start_y=250, end_x=600, end_y=250))
        assert not output['ok'] and result.get('isError'), output
        assert (root / 'xdotool-calls').read_text().splitlines()[-1] == 'mouseup 1'
        clicks_before = sum('click' in event for event in events())
        result, output = call('click', dict(x=100, y=50))
        assert output['ok'], output
        wait_for(lambda: sum('click' in event for event in events()) > clicks_before)
        # A missing executable must fail before any global fallback is tried.
        stop(proc)
        wrapper.unlink()
        (shim / 'sh').symlink_to('/bin/sh')
        no_xdotool = dict(env, PATH=str(shim))
        command[command.index('-o') + 1] = str(root / 'opens-missing')
        command[command.index('--'):command.index('--')] = ['--ro-bind', '/dev/null', '/usr/bin/xdotool']
        proc = subprocess.Popen(command, env=no_xdotool, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                stderr=errors, text=True)
        cleanup.callback(stop, proc)
        rpc(proc, 1, 'initialize', dict(protocolVersion='2024-11-05', capabilities={}, clientInfo=dict(name='missing-xdotool', version='1')))
        proc.stdin.write(json.dumps(dict(jsonrpc='2.0', method='notifications/initialized')) + '\n')
        proc.stdin.flush()
        for name, arguments in [('click', dict(x=100, y=50)), ('scroll', dict(x=100, y=50, direction='down')), ('drag', dict(start_x=100, start_y=250, end_x=600, end_y=250))]:
            result, output = call(name, arguments)
            assert not output['ok'] and 'requires xdotool' in output['message'], output
        if any('/dev/uinput' in path.read_text() for path in root.glob('opens*')):
            failures.append('native X11 tried to open the system-wide uinput device')
        if (root / 'forbidden').exists() and (root / 'forbidden').read_text().strip():
            failures.append('a denied global-input helper was invoked')
        assert xd('getmouselocation', '--shell', target=other) == before_other, 'other display pointer moved'
        print(json.dumps(dict(display=env['DISPLAY'], repeated_clicks=2, gtk_drops=drops(), failures=failures)), flush=True)
    assert not failures, failures


def main():
    binary = str(Path(sys.argv[1]).resolve())
    with tempfile.TemporaryDirectory(prefix='cul-xptr-', dir=Path.home() / '.cache') as directory:
        root = Path(directory)
        runtime = root / 'run'
        runtime.mkdir(mode=0o700)
        config = root / 'bus.conf'
        config.write_text('<busconfig><type>session</type>' + f'<listen>unix:tmpdir={runtime}</listen>' +
                          '<policy context="default"><allow own="*"/><allow send_destination="*"/>' +
                          '<allow receive_sender="*"/></policy></busconfig>')
        env = dict(os.environ, XDG_CACHE_HOME=directory, XDG_CONFIG_HOME=directory, XDG_RUNTIME_DIR=str(runtime))
        subprocess.run(['dbus-run-session', '--config-file', str(config), '--', '/usr/bin/python3',
                        __file__, binary, '--private', directory], env=env, check=True, timeout=90)


if __name__ == '__main__':
    if sys.argv[1] == '--app':
        app(Path(sys.argv[2]))
    elif len(sys.argv) > 2 and sys.argv[2] == '--private':
        run(str(Path(sys.argv[1]).resolve()), Path(sys.argv[3]))
    else:
        main()

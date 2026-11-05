# SPDX-License-Identifier: MIT
"""Directories for test fixtures that bind or connect to Unix sockets, and
the config of the private D-Bus session they run in.

A socket path must fit sun_path, so a fixture under a long TMPDIR fails with
"AF_UNIX path too long" or dbus-daemon's "Socket name too long", which read
like product bugs. Fixtures make their directory here instead: in the temp
dir (or the base they name) when the longest socket path fits, else in
XDG_RUNTIME_DIR, else they stop with a message that names the limit and the
path. src/test_sockets.rs applies the same rule to the Rust unit tests.
"""
import os
from pathlib import Path
import shutil
import tempfile
from xml.sax.saxutils import escape as xml_escape

# sun_path holds 108 bytes, including the terminating NUL.
SOCKET_PATH_MAX = 107
# dbus-daemon refuses a listen path longer than this.
DBUS_SOCKET_PATH_MAX = 99
# The socket a private bus makes for <listen>unix:dir=DIR</listen>.
DBUS_DIR_SOCKET = 'dbus-' + 'X' * 10
# The real server's overlay socket in XDG_RUNTIME_DIR. Its capture handshake
# uses it even with the indicator off.
INDICATOR_SOCKET = 'computer-use-linux-indicator.sock'
# The name tempfile.TemporaryDirectory(dir=...) gives without a prefix.
TEMPDIR_NAME = 'tmp' + 'X' * 8


def socket_tempdir(prefix, sockets, bus_dir=None, dir=None):
    """tempfile.TemporaryDirectory(prefix=prefix) whose socket paths fit.

    sockets are the socket paths the fixture binds or connects to, relative
    to the new directory. bus_dir is the directory, relative to it ('' for
    itself), that a private bus listens in (see private_bus_config).
    The bus is checked against dbus-daemon's own limit even when another
    socket is longer, so a layout change cannot drop the check. dir is the
    preferred base; it defaults to the temp dir.
    """
    # TemporaryDirectory adds eight random characters to the prefix.
    name = prefix + 'X' * 8
    checks = [(path, SOCKET_PATH_MAX) for path in sockets]
    if bus_dir is not None:
        checks.append((os.path.join(bus_dir, DBUS_DIR_SOCKET), DBUS_SOCKET_PATH_MAX))

    def overflow(base):
        for relative, limit in checks:
            path = os.path.join(base, name, relative)
            length = len(os.fsencode(path))
            if length > limit:
                return f'{path} is {length} bytes, over the {limit}-byte limit'
        return None

    base = tempfile.gettempdir() if dir is None else os.fspath(dir)
    first = overflow(base)
    if first is None:
        return tempfile.TemporaryDirectory(prefix=prefix, dir=base)
    runtime = os.environ.get('XDG_RUNTIME_DIR', '')
    if os.path.isabs(runtime) and os.path.isdir(runtime):
        second = overflow(runtime)
        if second is None:
            return tempfile.TemporaryDirectory(prefix=prefix, dir=runtime)
        fallback = f'under XDG_RUNTIME_DIR, {second}'
    elif runtime:
        fallback = f'XDG_RUNTIME_DIR ({runtime}) is not an absolute directory'
    else:
        fallback = 'XDG_RUNTIME_DIR is unset'
    remedy = 'Set TMPDIR' if dir is None else f'Move {base}'
    raise SystemExit(f'Unix socket path too long: {first}; {fallback}. '
                     f'{remedy} or XDG_RUNTIME_DIR to a shorter path.')


def private_bus_config(path, listen_dir, services=()):
    """Write a private session bus config to path and return path.

    Pass it to dbus-run-session --config-file. The bus listens in listen_dir,
    inside the fixture's own directory, so the socket goes away with the
    fixture even when the daemon is killed. dbus-daemon does not remove its
    socket on SIGKILL.

    The bus can activate only the services named, such as ca.desrt.dconf for
    gsettings or org.a11y.Bus for AT-SPI. Their .service files are copied
    from XDG_DATA_DIRS into a directory next to path. Nothing else starts, so
    the bus never reaches the desktop's portals, and the document portal
    never mounts a FUSE filesystem in the fixture that a killed portal would
    leave behind.
    """
    path = Path(path)
    servicedir = ''
    if services:
        target = path.with_name(path.stem + '-services')
        target.mkdir(mode=0o700)
        data_dirs = os.environ.get('XDG_DATA_DIRS') or '/usr/local/share:/usr/share'
        for name in services:
            sources = (Path(base, 'dbus-1', 'services', name + '.service')
                       for base in data_dirs.split(':') if base)
            source = next((source for source in sources if source.is_file()), None)
            if source is None:
                raise SystemExit(f'No D-Bus service file for {name} under XDG_DATA_DIRS ({data_dirs}).')
            shutil.copyfile(source, target / source.name)
        servicedir = f'<servicedir>{xml_escape(str(target))}</servicedir>'
    # A D-Bus address value keeps [-0-9A-Za-z_/.\*] and writes other bytes
    # as %XX, which leaves nothing to escape in XML.
    plain = b'-_/.\\*'
    listen = ''.join(chr(byte) if bytes([byte]).isalnum() or byte in plain else f'%{byte:02x}'
                     for byte in os.fsencode(listen_dir))
    path.write_text('<busconfig><type>session</type>'
                    f'<listen>unix:dir={listen}</listen>{servicedir}'
                    '<policy context="default"><allow own="*"/>'
                    '<allow send_destination="*"/><allow receive_sender="*"/>'
                    '</policy></busconfig>')
    return path

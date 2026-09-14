#!/usr/bin/env bash
# Opt-in integration test. Run inside an isolated graphical display; this script
# creates a private D-Bus session and private dconf/runtime state, never the host's.
set -euo pipefail
if [[ ${CUL_GUARD_TEST_PRIVATE_BUS:-} != 1 ]]; then
  scratch=$(mktemp -d /tmp/cul-guard-test-XXXXXXXX)
  trap 'find "$scratch" -xdev -depth -delete' EXIT
  export XDG_CONFIG_HOME="$scratch/config" XDG_RUNTIME_DIR="$scratch/runtime"
  export CUL_GUARD_TEST_SCRATCH="$scratch"
  export GIO_USE_VFS=local GVFS_DISABLE_FUSE=1
  unset AT_SPI_BUS_ADDRESS WAYLAND_DISPLAY
  mkdir -m 700 -p "$XDG_CONFIG_HOME" "$XDG_RUNTIME_DIR"
  dbus-run-session -- env CUL_GUARD_TEST_PRIVATE_BUS=1 bash "$0" "$@"
  exit
fi
binary=$(realpath "${1:-target/debug/computer-use-linux}")
scratch=${CUL_GUARD_TEST_SCRATCH:?}
guard_pid= gsd_pid= gtk_pid=
cleanup() {
  result=$?
  if [[ $result != 0 ]]; then
    for log in "$scratch/guard.log" "$scratch/gsd.log" "$scratch/gtk.log"; do
      if [[ -f "$log" ]]; then tail -30 "$log" >&2; fi
    done
  fi
  for child in "$gtk_pid" "$guard_pid" "$gsd_pid"; do
    if [[ -n "$child" ]]; then kill "$child" 2>/dev/null || true; wait "$child" 2>/dev/null || true; fi
  done
}
trap cleanup EXIT
setting() { gsettings "$@" org.gnome.desktop.interface toolkit-accessibility; }
wait_enabled() {
  for ((attempt=0; attempt<50; attempt++)); do
    [[ $(setting get) == true ]] && return
    sleep 0.1
  done
  return 1
}
gsettings set org.gnome.desktop.interface toolkit-accessibility false
/usr/libexec/gsd-a11y-settings >"$scratch/gsd.log" 2>&1 & gsd_pid=$!
sleep 0.3
# Establish that GSD really resets the key without our guard (the red control).
gsettings set org.gnome.desktop.a11y.applications screen-magnifier-enabled true
wait_enabled
gsettings set org.gnome.desktop.a11y.applications screen-magnifier-enabled false
for ((attempt=0; attempt<50; attempt++)); do
  [[ $(setting get) == false ]] && break
  sleep 0.1
done
[[ $(setting get) == false ]]
"$binary" guard-accessibility >"$scratch/guard.log" 2>&1 & guard_pid=$!
wait_enabled
for ((attempt=0; attempt<50; attempt++)); do
  if grep -q 'guard active:' "$scratch/guard.log"; then break; fi
  kill -0 "$guard_pid"
  sleep 0.1
done
grep -q 'guard active:' "$scratch/guard.log"
# Exercise the real GSD last-feature-off trigger without changing host settings.
gsettings set org.gnome.desktop.a11y.applications screen-magnifier-enabled true
sleep 0.2
gsettings set org.gnome.desktop.a11y.applications screen-magnifier-enabled false
sleep 0.2
wait_enabled
kill -0 "$gsd_pid"
# A new GTK process must expose a tree after the reset has been repaired.
/usr/bin/python3 -c 'import gi; gi.require_version("Gtk", "3.0"); from gi.repository import Gtk; w=Gtk.Window(title="CUL Guard Test"); w.add(Gtk.Button(label="GUARD_TEST_TARGET")); w.connect("destroy", Gtk.main_quit); w.show_all(); Gtk.main()' >"$scratch/gtk.log" 2>&1 & gtk_pid=$!
for ((attempt=0; attempt<20; attempt++)); do
  "$binary" state >"$scratch/tree.json"
  if grep -q 'GUARD_TEST_TARGET' "$scratch/tree.json"; then break; fi
  sleep 0.2
done
grep -q 'GUARD_TEST_TARGET' "$scratch/tree.json"
kill -TERM "$guard_pid"
wait "$guard_pid"
guard_pid=
gsettings set org.gnome.desktop.interface toolkit-accessibility false
sleep 1.2
[[ $(setting get) == false ]]
# Losing the monitor must fail closed, release the listener, and stop writes.
"$binary" guard-accessibility >"$scratch/guard.log" 2>&1 & guard_pid=$!
wait_enabled
for ((attempt=0; attempt<50; attempt++)); do
  monitor_pid=$(pgrep -P "$guard_pid" -f '^gsettings monitor org.gnome.desktop.interface toolkit-accessibility$' || true)
  [[ -n "$monitor_pid" ]] && break
  sleep 0.1
done
[[ "$monitor_pid" =~ ^[0-9]+$ ]]
kill -TERM "$monitor_pid"
if wait "$guard_pid"; then echo 'guard ignored monitor failure' >&2; exit 1; fi
guard_pid=
gsettings set org.gnome.desktop.interface toolkit-accessibility false
sleep 1.2
[[ $(setting get) == false ]]
echo 'PASS: GSD unguarded reset control, guarded reset, new GTK tree, SIGTERM, monitor failure, no writes after stop'

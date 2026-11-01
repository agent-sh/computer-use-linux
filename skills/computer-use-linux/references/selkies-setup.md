---
name: selkies-setup
description: "Run computer-use-linux inside a linuxserver.io Selkies desktop container."
---

# Selkies Container Setup

linuxserver.io's Selkies images (`webtop`, `chrome`, and the other desktop apps) run a full desktop in a container. Run `computer-use-linux` inside the container and let the MCP client on the host reach it over stdio through `docker exec`. No port is opened.

## Start the container with `PELORUS=true`

```bash
docker run -d --name desktop --shm-size=1gb -e PELORUS=true -p 3001:3001 lscr.io/linuxserver/chrome:latest
```

`PELORUS=true` turns on pixelflux's computer-use server, so screenshots and input work without a consent prompt. On labwc images such as `chrome` it also starts the session bus and the AT-SPI registry. As a side effect it runs linuxserver.io's own Pelorus agent on port 5100 inside the container, which stays private unless you publish that port.

## Install the binary

Install as `abc`, the desktop user, into `/config`, which survives container restarts:

```bash
docker exec -u abc desktop sh -c '
set -e
mkdir -p /config/.local/bin && cd /config/.local/bin
asset=computer-use-linux-$(uname -m)-unknown-linux-gnu
base=https://github.com/agent-sh/computer-use-linux/releases/latest/download
curl -fsSLO "$base/$asset" -O "$base/$asset.sha256"
sha256sum -c "$asset.sha256"
mv "$asset" computer-use-linux && chmod +x computer-use-linux && rm "$asset.sha256"'
```

Release binaries need glibc. Alpine-based tags such as `webtop:alpine-kde` use musl, so pick a Debian, Ubuntu, Fedora, or Arch tag, for example `webtop:debian-kde`.

## Add the session launcher

`docker exec` starts outside the desktop session, with no session bus and no `WAYLAND_DISPLAY`. Selkies saves the variables its own services use (`XDG_RUNTIME_DIR`, `PIXELFLUX_CU`, `PIXELFLUX_CU_TOKEN`) in `/run/s6/container_environment`, where s6's `with-contenv` loads them. The launcher does that, then takes the session bus from a desktop process: `plasmashell` on KDE webtops, which also gives it the display, or `labwc` on app images. labwc's own `WAYLAND_DISPLAY` is `wayland-1`, Selkies' display, so the launcher points at the desktop's `wayland-0` instead.

```bash
docker exec -i -u abc desktop sh -c 'cat > /config/.local/bin/computer-use-linux-selkies && chmod +x /config/.local/bin/computer-use-linux-selkies' <<'EOF'
#!/command/with-contenv bash
# Run computer-use-linux inside the Selkies desktop session.
# with-contenv supplies XDG_RUNTIME_DIR and the pixelflux variables.
if session=$(pgrep -xo plasmashell); then
    # KDE webtops: plasmashell carries the whole session environment.
    vars='DBUS_SESSION_BUS_ADDRESS|DISPLAY|WAYLAND_DISPLAY|XDG_CURRENT_DESKTOP|XDG_SESSION_TYPE'
elif session=$(pgrep -xo labwc); then
    # labwc images: labwc's own WAYLAND_DISPLAY is Selkies' display, so take only its bus.
    export WAYLAND_DISPLAY=wayland-0
    vars='DBUS_SESSION_BUS_ADDRESS'
else
    echo "computer-use-linux-selkies: no running plasmashell or labwc session" >&2
    exit 1
fi
while IFS= read -r var; do
    export "${var?}"
done < <(tr '\0' '\n' < "/proc/${session}/environ" | grep -E "^(${vars})=")
exec /config/.local/bin/computer-use-linux "${@:-mcp}"
EOF
```

## Configure the MCP client

On the host, point the client at the launcher through `docker exec -i`:

```json
{
  "command": "docker",
  "args": ["exec", "-i", "-u", "abc", "desktop", "/config/.local/bin/computer-use-linux-selkies"]
}
```

## Check it

```bash
docker exec -u abc desktop /config/.local/bin/computer-use-linux-selkies doctor
```

`capabilities.screenshot` and `capabilities.input` should start with `pixelflux`, and `capabilities.window_control` should list `labwc` or `kwin`.

On KDE webtops, AT-SPI starts disabled. Call `setup_accessibility` once, then restart any app that was already open so it publishes its accessibility tree.

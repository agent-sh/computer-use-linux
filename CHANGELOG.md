# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- The GNOME keymap preflight for `type_text` parses the compositor's keymap with the pure-Rust `kbvm` crate instead of loading `libxkbcommon.so.0` at runtime. No XKB shared library is needed, so the preflight also works in static musl builds. The minimum Rust version is now 1.95, which `kbvm` requires.

### Fixed

- The `mcp --listen` integration tests kill and reap their server when a test fails. A failed assertion used to leave the test's listener running after the test run ended.

## [0.7.52] - 2026-11-12

### Added

- `screenshot`, `get_app_state` and the `screenshot_after` capture of every mutating tool take `screenshot_file_path`, an absolute path where the image is saved instead of being returned as MCP image content, so a capture costs no image tokens until the agent opens the file. The result keeps its screenshot metadata (size, coordinate size, `scale`, `format`, `bytes`) and adds `file_path`. The parent directory must exist, and the extension must match `format` (`.png`, or `.jpg`/`.jpeg` for jpeg). File output creates a new file and never replaces one: any existing entry is refused, and the image moves into place from a same-directory temp file with a step that fails if anything appeared at the path meanwhile. A bad path is refused before any capture or input. A failed write is the `screenshot` call's error, `screenshot_error` on `get_app_state`, and `screenshot_error` with `ok` kept on a mutating tool; the image is never returned inline instead. Over HTTP (`mcp --listen`) the parameter is refused, so a remote client gets no file writes on the host. Thanks to [@xchacha20-poly1305](https://github.com/xchacha20-poly1305) ([#446](https://github.com/agent-sh/computer-use-linux/issues/446)).

### Changed

- The `type_text` description, the server instructions, the skill and the npm README describe the AT-SPI fallback below. They no longer say that every text backend leaves field contents unverified. The Pi extension's catalog hash for `type_text` changes with the description.

### Fixed

- On GNOME, `type_text` no longer refuses text only because one keyboard layout group cannot type a character, such as Latin text with `us` and `il` groups ([#451](https://github.com/agent-sh/computer-use-linux/issues/451)). The portal route still refuses it, since a background client cannot read the active group. The text then goes into the focused editable field of the verified target window through AT-SPI EditableText: inserted at the caret, replacing a selection, or set when the field is empty or fully selected. The field must be in the AT-SPI window that matches the target, and that window must be active, so a field left focused in another window of the same app is never used. InsertText's length is in UTF-16 code units for Qt apps and in bytes for other toolkits, as the AT-SPI documentation counts it; non-ASCII text keeps the refusal when the app's toolkit cannot be read. Each edit call has a 5-second limit and runs to the end even if the request is cancelled. The call reports `ok: true` only when the field reads back the text at the insertion point and the rest of its text unchanged (in a field over 4096 characters, the 32 on each side of the insert), and the message then starts with "Typed through AT-SPI EditableText". Failures after an edit start with "AT-SPI EditableText insert was not verified"; a mismatch quotes what the field holds. A password field, focus outside an editable field, and text with a newline, tab or other control character keep the refusal, since inserting a newline is not pressing Enter. The refusal now says whether the group has no key for the character or has it only at a shift level the portal cannot type.
- `get_app_state` reports text selections. The AT-SPI proxy called `GetNselections`, which toolkits do not implement, so every selection list came back empty. The method is now called by its real name, `GetNSelections`.

## [0.7.51] - 2026-11-11

### Fixed

- A pid target reads every AT-SPI root its process owns. A Qt Quick app can register an empty GTK root for native dialogs next to its populated Qt root, and `get_app_state(pid=...)` used to read only the first, so it returned no controls. A pid that owns no root now returns an empty scoped tree with a note, not the whole desktop. Thanks to [@OnlyHa1bara](https://github.com/OnlyHa1bara) ([#445](https://github.com/agent-sh/computer-use-linux/pull/445)).
- The `include_tree` observation of `perform_action` and `set_value` reads the acted-on element's app. It used to read the focused app's tree after an action on a background element, so the readback could describe another app ([#445](https://github.com/agent-sh/computer-use-linux/pull/445)).
- A window whose pid owns no AT-SPI root still gets its tree, found by the window title. Flatpak apps register through `xdg-dbus-proxy`, so their window's pid never owns a root. The title is used only for a window the compositor lists, and the tree records no pid, so element calls check each node's owner ([#445](https://github.com/agent-sh/computer-use-linux/pull/445)).

## [0.7.50] - 2026-11-11

### Security

- `rustix` 1.1.6, which fixes RUSTSEC-2026-0486. `cargo audit` passes again.

### Added

- `mcp --listen ADDR:PORT` serves MCP over streamable HTTP at `/mcp` (#362). It needs a bearer token from `COMPUTER_USE_LINUX_MCP_TOKEN_FILE` or `COMPUTER_USE_LINUX_MCP_TOKEN`, refuses requests with an `Origin` header or a `Host` outside `--allowed-host`, and refuses to start with `COMPUTER_USE_LINUX_ENABLE_SHELL=1`. Over HTTP, element calls by index or selector must pass `snapshot_id`. Without `--listen` the server is unchanged and opens no socket. Thanks @berney (#444).

## [0.7.49] - 2026-11-09

### Security

- `rmcp` 3.9.0, which fixes RUSTSEC-2026-0462. `cargo audit` passes again.

### Changed

- Output schemas list nullable fields as required, as `rmcp` 3.9.0 generates them: such a field is always present and is `null` when it has no value. The server already sent these fields as `null`, so results are unchanged and still validate against their schemas. Tool names, descriptions, input schemas and annotations are unchanged. The Pi extension's catalog hashes change with the schemas, so the extension and the server are released together.

## [0.7.48] - 2026-11-09

### Added

- Snapshot handles. `get_app_state`, an `include_tree` observation and `get_tree_page` return the `snapshot_id` of their tree. `click`, `scroll`, `perform_action`, `set_value` and `get_tree_page` take it. The server caches one tree, and every new tree replaces it. A call that picks its element by `element_index` or selector and names a replaced snapshot is refused before any window is raised or input is sent. Before, an index from an older tree resolved silently in the newer one and could name another element: after an `include_tree` result, or when two clients share one server. Without `snapshot_id`, the element resolves in the latest tree, as before. An `element_identifier` names the AT-SPI object and is not checked. Each server process has its own id prefix, so an id from before a restart is refused. Thanks to [@berney](https://github.com/berney), whose listener proposal raised the two-client case ([#362](https://github.com/agent-sh/computer-use-linux/issues/362)).

### Fixed

- `scroll` with an app target and an `element_index` checks that the element belongs to the target app when the call passes only `x` or only `y`. The point then comes from the element, as with no point at all, but the check ran only when both `x` and `y` were missing. An index from another app's snapshot could then pick the wheel's point.
- A coordinate scroll on Wayland sends the wheel from the uinput absolute pointer, the device that moved the pointer. It used to move with uinput and send the wheel through ydotool's own device 35 ms later. GNOME then routed the wheel to a stale pointer position, and XWayland windows such as WeChat never scrolled while the call reported success. The route no longer needs a ydotool daemon. Thanks to [@ClarkYWHsu](https://github.com/ClarkYWHsu) for the diagnosis and the fix ([#433](https://github.com/agent-sh/computer-use-linux/pull/433)).
- `direction: "left"` and `"right"` scroll the way they say on the uinput and ydotool routes. Both sent left as a positive `REL_HWHEEL`, which the kernel and libinput read as a scroll right. The portal and X11 routes were already right ([#433](https://github.com/agent-sh/computer-use-linux/pull/433)).
- The `editable_text_normalization_mismatch_is_not_verified` test accepts `ReadbackTimedOut` as well as `MismatchAfterDeadline`, like the two other mismatch tests. Its last poll can start just before the one-second deadline and still be reading when it passes. On a slow CI runner the test failed this way. Both outcomes leave the write unverified, and neither allows a replay.

## [0.7.47] - 2026-11-05

### Fixed

- `setup_window_targeting` and `setup-window-targeting` install the user copy of the extension in `$XDG_DATA_HOME/gnome-shell/extensions`, where GNOME Shell loads user extensions. They fall back to `~/.local/share` only when `XDG_DATA_HOME` is unset or empty, as GLib does. When GNOME Shell already loaded a user copy, setup updates the copy the Shell reports, since the Shell can run with another `XDG_DATA_HOME` than the MCP host. Before, setup always wrote under `~/.local/share`. With `XDG_DATA_HOME` set elsewhere, the Shell never loaded that copy, and the reload setup asked for did not help. The Pi extension and the Codex plugin now pass `XDG_DATA_HOME` to the server. `install.sh` already used this path ([#426](https://github.com/agent-sh/computer-use-linux/issues/426)).
- The `setup-window-targeting` help names all five display-only metadata fields: `name`, `description`, `url`, `version-name` and `donations`. It named the first three ([#426](https://github.com/agent-sh/computer-use-linux/issues/426)).
- Tests no longer leave D-Bus sockets in `/tmp`. Each private bus listens in its fixture's directory, which the test removes, and can start only the services the test needs. Four Rust fixtures listened in `/tmp` and killed their bus with SIGKILL, which leaves the socket behind, so a full `cargo test` left 28. The script fixtures no longer start the desktop portals, whose document portal mount could outlive a killed test. `install_sh_test.sh` no longer leaves two files in `TMPDIR` ([#425](https://github.com/agent-sh/computer-use-linux/issues/425)).

## [0.7.46] - 2026-11-05

### Fixed

- `computer-use-linux-indicator` and `computer-use-linux-cosmic` check their command line before they do anything, as `computer-use-linux` does since 0.7.44. Both print help on `-h` or `--help` and exit 0. The indicator takes no arguments, and the COSMIC helper takes only its five commands and `--window-id`. Anything else prints an error on stderr and exits 2. The indicator used to start on `--help` or any flag: it created its lock files in `XDG_RUNTIME_DIR` and connected to the compositor. The COSMIC helper exited 1 on a bad line, and `probe --help` failed instead of printing help. The server starts both as before ([#419](https://github.com/agent-sh/computer-use-linux/issues/419)).
- `setup_window_targeting` and `setup-window-targeting` no longer ask for a new login when only the extension's display metadata changed. GNOME Shell reads `metadata.json` when it loads the extension. It only checks that `name` and `description` are present, and only displays them, `url`, `version-name` and `donations`. 0.7.44 added `url`, so the next setup with the extension enabled said GNOME Shell must reload, though `extension.js` was unchanged. A changed `extension.js`, or a change to any other field, such as `shell-version`, still sets `requires_shell_reload`. `install.sh` follows the same rule for the `name`, `description`, `url` and `version-name` lines ([#420](https://github.com/agent-sh/computer-use-linux/issues/420)).
- Tool, parameter and CLI text match the code ([#418](https://github.com/agent-sh/computer-use-linux/issues/418)). The `press_key` description and the `type_text` sentence of the server instructions name pixelflux. Both send there when `PIXELFLUX_CU` and `PIXELFLUX_CU_TOKEN` are set. `setup --help` says setup requests `org.a11y.Status IsEnabled` only when the key does not read back true or the tree could not be built. The `terminal_command` descriptions say the match ignores ASCII case, not all case. The `pid` description of `perform_action` and `set_value` says when it must match exactly one window. The `click` `x` and `y` descriptions name the `coordinate_space="atspi"` exception. The `set_value` `value` description says which text counts as a number, `inf`, `NaN` and `1e3` included.

## [0.7.45] - 2026-11-05

### Fixed

- `click` and `scroll` `x`/`y`, the `drag` endpoints, `perform_action` `action` and `element_identifier`, and `set_value` `value` and `element_identifier` have parameter descriptions: the pixel space, the action name or index rule, and how a value is applied.

## [0.7.44] - 2026-11-05

### Added

- The Claude Code and Codex plugins have an icon: a penguin whose belly a pointer clicks. Both manifests point at `plugins/computer-use-linux/assets/icon.png`; its SVG source sits next to it.

### Changed

- The CLI checks the whole command line before it does anything. An unknown command, an unknown flag, or a missing or extra argument prints an error and a pointer to `--help` on stderr and exits with status 2. An unknown command used to exit 1. `--` ends the flags, so `state -- --help` reads an app named `--help`.
- The top-level help lists every command with a one-line summary, `guard-accessibility` included. Only the `abs-test` development probe stays out.
- The Pi extension downloads its binary on the first tool call when the package arrived without it. npm 12 skips install scripts it was not told to allow, so `pi install npm:@agent-sh/computer-use-linux` left no binary. The extension runs the package's `npm/install.js`, the same first-launch install the CLI wrapper runs, and uses that binary ahead of one on PATH. When the download fails, a binary on PATH runs instead. If there is none, or it fails, as a different version does, the error names the failed download and the next call tries the download again. A cancelled call stops waiting for the download and never starts one. Nothing is downloaded at session start. Set `COMPUTER_USE_LINUX_BIN` to run a PATH or development build instead.
- The npm package declares `"libc": ["glibc"]`, so npm refuses it on musl systems, where the glibc release binaries it downloads do not run.
- The npm package's optional Pi peers require `>=0.84.4`, the documented minimum, instead of any version.
- The crate declares `rust-version = "1.89"`. It uses `File::try_lock`, stable since Rust 1.89; older compilers failed with E0658.
- The crate's documentation link points at docs.rs. The `homepage` that repeated the repository URL is gone, and so is `authors`, which Cargo deprecates.
- The crate no longer ships files that its build and tests do not read: `scripts/mcp_safety_check.py`, the manual test notes, the `install.sh` test fixtures and the Python example.
- The GNOME Shell extension's metadata names the project URL.

### Fixed

- Tools the server runs, such as `gsettings`, see the same environment under Codex as under Pi. The Codex plugin now passes `XDG_DATA_DIRS`, `XDG_CONFIG_HOME`, `GSETTINGS_SCHEMA_DIR`, `GIO_EXTRA_MODULES`, `GI_TYPELIB_PATH`, `LD_LIBRARY_PATH`, `NIX_LD`, `NIX_LD_LIBRARY_PATH`, `GDMSESSION`, `LANGUAGE` and `RUST_LOG`, and the env forwarding check fails when Codex drops a variable Pi passes.
- Subcommands no longer ignore their flags. `setup --help` ran setup and wrote GNOME settings, `setup-window-targeting --help` installed and enabled the extension, `guard-accessibility --help` started the guard, which never exits, and `abs-test --help` clicked at 0,0. Every command now prints its own help on `-h` or `--help` and exits 0 without touching the desktop. `abs-test`, the development click probe, now needs integer X and Y.
- Pi passes `COMPUTER_USE_LINUX_HYPRLAND_BACKGROUND`, `COMPUTER_USE_LINUX_PERSIST_REMOTE_DESKTOP`, `COMPUTER_USE_LINUX_XDOTOOL_TYPE_DELAY_MS`, `NIRI_SOCKET`, `PIXELFLUX_CU` and `PIXELFLUX_CU_TOKEN` to the server. They did nothing under Pi.
- Codex passes `COMPUTER_USE_LINUX_HYPRLAND_BACKGROUND`, `COMPUTER_USE_LINUX_PORTAL_SCROLL_INVERT`, `PIXELFLUX_CU` and `PIXELFLUX_CU_TOKEN` to the server. They did nothing under Codex. A CI check now fails when the server reads a variable that the Pi extension or the Codex manifest does not pass.
- `npm run test:wrapper` tests the checkout's debug build. It used to download the published release and test that. CI runs it now.
- Tool descriptions, parameter descriptions and server instructions match what the server does: the pointer and screenshot backend order, which backends `move_window` and `resize_window` support (and that `resize_window` sets the client area on X11), the readiness fields that call for `setup_window_targeting`, which results carry no post-action observation, and which keyboard routes warn about a non-editable focus. The off-screen warning names the backends `move_window` and `resize_window` need. Selector, button, click count, scroll, key, text and window-target parameters now have descriptions. The `get_app_state` `terminal_pid` description named the terminal emulator's pid; the field matches the shell or foreground process on the terminal's tty.

## [0.7.43] - 2026-11-05

### Fixed

- `drag` results get the off-screen and monitor-gap warnings that `click` and `scroll` results get, one for each endpoint, naming the start or end coordinate. A drag used to get none on any route. On X11 and the uinput pointer it reported plain success when an endpoint off the screenshot had been moved to an edge. It did the same when an endpoint between monitors had landed at a monitor edge or on no visible pixel. A failed xdotool or ydotool drag on X11 gets no warning, as a failed click does.
- A uinput `drag` result says where an off-screen endpoint was clamped to, as a click result does, and checks the clamped point against the monitors.
- A portal `drag` with an endpoint off the shared monitors still sends nothing. Its error now names each endpoint that stopped it and says whether that point is off the screenshot or on no monitor.
- Tests no longer fail when `TMPDIR` is long. Socket fixtures move to `XDG_RUNTIME_DIR` when a path would pass the Unix socket limit, and the crate ships `scripts/socket_fixture.py`, which the packaged test scripts import ([#401](https://github.com/agent-sh/computer-use-linux/issues/401)).

## [0.7.42] - 2026-11-05

### Changed

- KWin element clicks get no off-screen or monitor-gap warning on either input route, since KWin maps the element point onto an output. The portal route dropped the warning that 0.7.41 kept; the uinput route still reports where a point was clamped ([#397](https://github.com/agent-sh/computer-use-linux/pull/397), [#407](https://github.com/agent-sh/computer-use-linux/issues/407)).

### Fixed

- A click or scroll coordinate between monitors gets a warning. Monitors of different heights or offset rows leave parts of the screenshot that no monitor shows, and a point there used to pass as on-screen. The server reads the monitor layout once per screenshot, from the portal session or the compositor, or from RandR on X11, and checks the point in screenshot pixels. It never runs xrandr on Wayland, where that could start Xwayland. Without a layout that fits the screenshot, only its outer rectangle is checked, as before. No screenshot is taken for the check.
- Off-screen warnings say what the input route did with the point. The portal sends nothing, and its warning no longer claims the input landed. On X11 the server keeps the pointer on the screen, so a point off the screenshot lands at an edge; the warning used to say it landed on no visible pixel. On X11 and the uinput pointer, a point between monitors lands at a monitor edge or on no visible pixel. A point clamped into a gap, by the X server or the uinput device, gets that warning too. A failed X11 dispatch gets no warning, since nothing was sent.
- When the uinput pointer's stale device cannot be replaced, the result says the device was removed and where a retry goes: a new device from a fresh screenshot first, then the remote desktop portal, ydotool, or a refusal on Wayland.
- A KWin element click whose uinput device cannot be replaced reports that device failure. It used to report a KWin coordinate mapping failure and ask for a new `get_app_state`, though the mapping had worked.
- The KWin element mapping text no longer names the backend a second time.

## [0.7.41] - 2026-11-04

### Fixed

- An untargeted `scroll` whose remote desktop portal could not start (denied consent, portal startup failure) and whose ydotool fallback then failed reports the portal error after the ydotool error. It used to show only the ydotool error.
- KWin element clicks keep the uinput edge-clamp note and the portal off-screen warning. The KWin coordinate mapping text is appended to the result message instead of replacing it.
- When a later screenshot reports a different desktop size, the next click, coordinate scroll or drag replaces the uinput absolute pointer, sized to that screenshot, as KWin element clicks already did. The compositor spreads the device's range across the current layout, so a device sized for an earlier desktop scaled every point to the wrong place, and an off-screen coordinate got no note. With a matching device, an off-screen coordinate is clamped to the desktop edge, and a click or scroll result says where it landed. If the replacement fails, the action is not sent and the result says why.

## [0.7.40] - 2026-11-04

### Changed

- The Pi extension's type check runs on TypeScript 7.0.2, the native Go compiler, instead of TypeScript 5.9.3. Same `tsc --noEmit` command and tsconfig; the check takes about 0.6 s instead of 2.7 s.

### Fixed

- A uinput absolute-pointer click whose press or release write fails sends one more release, so the button cannot stay held, and the error says whether that release went through. A drag reports its release the same way. A failed absolute-pointer action drops the device, and the next action creates a fresh one.
- A cancelled remote desktop portal click, drag, key chord or keysym text keeps the server's input lock until its release cleanup has sent the pending button or key release, so another backend cannot send input while a button or key is still down. A cancelled portal scroll runs to the end under the lock.
- A cancelled KDE clipboard paste still puts the user's clipboard back: the paste runs to the end with the input and clipboard locks held. When writing the text to the clipboard fails, the write may still have landed, so the previous contents are restored and the text is not retyped through ydotool.
- A `wtype` or `xdotool` on `PATH` without an execute bit is no longer selected, so input takes the next route, as when it is not installed. A selected wtype that cannot start (a missing interpreter, a `noexec` mount) makes `type_text` fall back to ydotool, as a missing wtype does; nothing was typed.
- Focus verification before targeted input gives each focused-window query up to 5 s and runs one more query after the 1 s transition allowance, so a slow backend (Hyprland runs two `hyprctl` calls per query) is not cut off while focus lands. Queries that keep reporting another window still fail after about 1 s.
- Portal pointer input on niri maps coordinates with niri's logical output layout, read over its IPC socket or `niri msg`, before it falls back to wlr-output-management.
- The server takes `NIRI_SOCKET` from the session's environment, like `WAYLAND_DISPLAY`, when the MCP host did not pass it.
- `doctor` reports the RemoteDesktop keyboard check as `portals.remote_desktop_keyboard`; readiness used it, but the report did not show it. The pointer check reads the ScreenCast portal version and, from version 2, requires the hidden cursor mode that portal pointer sessions request; version 1 portals pass, since a hidden cursor is their default.
- `doctor` no longer lists `wtype` as an input backend with `COMPUTER_USE_LINUX_FORCE_PORTAL_KEYBOARD=1`, or with `COMPUTER_USE_LINUX_FORCE_XDOTOOL_KEYBOARD=1` when xdotool and `DISPLAY` are available, since keyboard input then goes to that backend.

## [0.7.39] - 2026-11-04

### Changed

- The crates.io package carries only what the crate builds and tests from (src, tests, examples, the four Python scripts the tests run, the MCP safety check, the GNOME Shell extension, manifest, lockfile, README, CHANGELOG, LICENSE): 75 files instead of 135. The npm wrapper, Pi extension, plugins and CI config are no longer published to crates.io.

### Fixed

- The npm wrapper downloads the release binaries on first launch when npm skipped the package's install script, as npm 12 does for scripts it was not told to allow. A global install under npm 12 used to fail on every launch with `computer-use-linux binary not found`. The download's output goes to stderr, so an MCP client's stdout stays clean, each binary is written beside its destination and renamed into place, main binary last, so a server starting at the same time never runs a half-written file or misses a helper. A download that receives no data for 30 s fails, and a package directory that cannot be written is refused before anything is downloaded.
- The npm installer escapes CR, LF and the other C0 control characters in what it logs, so a path or URL from the environment (`COMPUTER_USE_LINUX_DOWNLOAD_BASE`, a local binary path) cannot add forged lines to the install output.

## [0.7.38] - 2026-11-04

### Fixed

- Window bounds from the i3 backend on Sway are scaled from Sway's layout coordinates to capture pixels when every active output has the same scale (never below 1, as grim captures), with edges rounded outward, and left out with mixed scales. Disabled and non-desktop outputs, which Sway lists without a scale, no longer make the output layout unreadable. On a scaled output, a window-cropped screenshot or a window-relative click used the layout rect as pixels and hit the wrong region ([#375](https://github.com/agent-sh/computer-use-linux/issues/375)).
- The i3 backend works on Sway without the i3 package: it runs `swaymsg` when `i3-msg` is missing, and takes the first of `I3SOCK` and `SWAYSOCK` that names a socket file, then the newest i3 socket, then the newest `sway-ipc.*.sock` in `XDG_RUNTIME_DIR`. A failed `swaymsg` command reports the error it printed on stdout ([#376](https://github.com/agent-sh/computer-use-linux/issues/376)).

## [0.7.37] - 2026-11-04

### Fixed

- `activate_window` with the i3 backend on Sway focuses Xwayland windows. The backend sent the window id in hex, which Sway rejects with `The value for 'id' should be numeric`; it now sends the decimal id, which i3 also accepts ([#357](https://github.com/agent-sh/computer-use-linux/issues/357)).
- The i3 backend on Sway lists native Wayland windows, so `list_windows`, `focused_window`, `activate_window` and targeted input reach them. It used to skip every window without an X11 id. A native Wayland window's `window_id` is 2^32 plus its Sway container id, above every X11 id, and focus matches `con_id`; X11 windows keep their X11 id. Sway windows carry the pid from the tree; i3 windows still read it through `xprop` ([#357](https://github.com/agent-sh/computer-use-linux/issues/357)).
- The i3 backend marks a window hidden when it sits in the scratchpad (under `__i3_scratch`). It used to read the window's `scratchpad_state`, which Sway keeps `fresh` after the window is shown, so a shown scratchpad window was refused for window-cropped screenshots; i3 reports `none` there, so a hidden i3 scratchpad window was never marked hidden.
- An element action with an exact window target is refused when the app's cached tree was narrowed to one window but recorded none, or recorded one that another window backend listed. A tree records no window when it was narrowed through the cached scope from another pid or frame, or when `get_app_state` took a title the window list could not resolve. Before, the action passed on the shared pid, so an index from one Chromium window could act on another ([#370](https://github.com/agent-sh/computer-use-linux/issues/370)).

Thanks @RaphaelFakhri for the decimal window id fix on Sway ([#367](https://github.com/agent-sh/computer-use-linux/pull/367)), and @joshuadavidthomas for the Sway report ([#357](https://github.com/agent-sh/computer-use-linux/issues/357)).

## [0.7.36] - 2026-11-01

### Added

- List and focus windows through `zwlr_foreign_toplevel_manager_v1` on wlroots compositors without a dedicated backend, such as stock labwc, phoc and river-classic. Windows carry no pids or bounds, and focus requires a unique app id and title match.
- Selkies container guide: run the server inside a linuxserver.io Selkies desktop container, reached from the host's MCP client over `docker exec`, with a launcher that joins the desktop session (`skills/computer-use-linux/references/selkies-setup.md`).
- `press_key` sends chords through `wtype` on Wayland sessions without ydotool whose session bus answers that no RemoteDesktop portal exists, as on wlroots compositors with xdg-desktop-portal-wlr (Sway, labwc, river, phoc, cage). Chords used to fail there. The portal stays preferred when it exists, when a keyboard session is already active, or with `COMPUTER_USE_LINUX_FORCE_PORTAL_KEYBOARD=1`. A missing or unreachable bus, a timeout or any other error keeps the portal route, and a failed or denied portal request is never replayed through wtype.

### Fixed

- Hyprland background input releases a key left down when a `send_key_state` batch stops between its down and up events, or when the batch's outcome is unknown, so the target app does not start its own key repeat. The error says whether the release went through, and the release waits at most a second.
- An element action whose target window shares its pid with another window of the same app (Chromium) is refused when the cached tree was narrowed to the other window and the target names that one window (a `window_id`, or an `app_id`, `wm_class` or title that matches it alone). Before, a matching pid passed the check. An app-wide tree, or an `app_id` shared by several windows, still passes on the pid.
- A post-action tree keeps the window it was taken for, so a follow-up action on the same window with an index from that tree passes the ownership check instead of asking for a new `get_app_state`. A window with no pid (i3, sway, COSMIC) is recorded only when the tree was narrowed to its frame, and its tree no longer borrows the previous snapshot's app. A tree narrowed through the previous snapshot's window scope (a target with no `window_id` or title) is recorded as that snapshot's window only when it came from the same pid and matched the same frame; otherwise it records no window. The snapshot's window, encoding and window scope are now stored together, so a concurrent call can no longer stamp one snapshot's window scope onto another.
- `perform_action`, `set_value`, and `press_key` and `type_text` in Hyprland background mode refuse a `screenshot_after: "window"` crop when the target window is not focused, since they do not raise it and the crop could show whatever covers it. No backend reports whether a window is covered, so on tiling layouts (sway, i3, Hyprland, labwc) this also refuses a window that sits visible beside the focused one, as `get_app_state` already does for its targeted screenshot. Use `screenshot_after: "screen"`, or the `screenshot` tool, which raises the target first.
- A compositor title that decorates the accessibility frame name, such as `PlanetScale - Chromium (Profile 1)` for the frame `PlanetScale - Chromium`, now narrows the tree to that window: the longest frame name the title starts with, followed by ` (...)` or ` - ...`.
- Hyprland background mode checks that the target is a Hyprland window before it refuses input it cannot map, and takes the `press_key` grammar of every other backend, so modifiers with no key (`Ctrl+Shift`) are refused. Background window capture reads the window's `stableId` from the listing that resolved it instead of listing every client again.
- `perform_action` and `set_value` with a window-style target and a `pid` fall back to the pid for the ownership check when the window listing is unavailable, as on GNOME without the Shell extension. Without a pid they still refuse.

Thanks @joshuadavidthomas for the wlr foreign toplevel backend ([#359](https://github.com/agent-sh/computer-use-linux/pull/359)) and the wtype chord route ([#360](https://github.com/agent-sh/computer-use-linux/pull/360)), and @berney for the Selkies container guide ([#361](https://github.com/agent-sh/computer-use-linux/pull/361)).

## [0.7.35] - 2026-10-31

### Added

- Hyprland background mode: with `COMPUTER_USE_LINUX_HYPRLAND_BACKGROUND=1`, `type_text` and `press_key` calls that name a target window send keys through Hyprland's `send_key_state` dispatcher without focusing it, and `get_app_state` and `screenshot` capture an unfocused target alone with `grim -T` instead of raising it or using the portal. Targets on hidden workspaces are captured until two frames match, because Hyprland repaints them only when captured. The user's focus, cursor and workspace stay put, so an agent can work on a window on another workspace. Keys go over Hyprland's control socket, never in a process's arguments, and show on the action indicator (masked unless the target's field is known not to be secret). Text is mapped for a US layout and refused whole otherwise.

### Fixed

- `get_app_state` refuses a screenshot of a targeted window that is not focused before it captures anything. It used to capture the whole desktop first and then refuse, so on a portal session the call raised a consent prompt and grabbed every monitor only to discard the image.
- Keep the remote desktop restore grant when EIS setup fails after `Start` on a non-GNOME session such as COSMIC, so persistence does not drop the grant and prompt for consent again.
- Connect the pixelflux client to the configured loopback host instead of always dialing 127.0.0.1, so `[::1]:PORT` reaches an IPv6-only server. Read screenshot dimensions from the PNG header instead of fully decoding the image, cap the typing request timeout at two minutes, and refuse text past the timeout budget before anything is sent instead of timing out mid-typing.

Thanks @NjengaFelix for Hyprland background mode ([#350](https://github.com/agent-sh/computer-use-linux/pull/350)) and the unfocused-target refusal ([#349](https://github.com/agent-sh/computer-use-linux/pull/349)).

## [0.7.34] - 2026-10-31

### Added

- `get_app_state` takes `interactive_only: true` to return only nodes with an action, editable text or a value, plus headings, status and live regions, windows and dialogs. Containers are collapsed and `element_index` values match the full tree ([#338](https://github.com/agent-sh/computer-use-linux/issues/338)).

### Changed

- Accessibility trees in `get_app_state`, `get_tree_page`, `include_tree` results and spill files use a compact node encoding: null, empty, zero and false fields are left out, containers drop the U+FFFC placeholder text, and `actions` lists action names. `get_app_state` with `verbose: true` returns the full encoding. Compaction also drops containers whose only text is that placeholder and whose only actions are the `doDefault`, `showContextMenu` and `clickAncestor` that Chromium lists on every element ([#338](https://github.com/agent-sh/computer-use-linux/issues/338)).
- A `title` or `window_id` target narrows the tree to that window when the app exposes several windows under one AT-SPI application root, as Chromium does. The app's popups, dialogs and unnamed windows stay, since Chromium lists them beside the window; `include_tree` results use the same scope ([#338](https://github.com/agent-sh/computer-use-linux/issues/338)).

### Fixed

- Give `perform_action` and `set_value` the optional app target `click` takes (`pid`, `window_id`, `app_id`, `wm_class`, `window_title`). With a target, an `element_index` or selector from another app's `get_app_state` snapshot is refused with the message `click` returns, instead of acting on that app. When the window backend reports no pid for the target window (i3, sway, COSMIC), all four tools compare the snapshot's window instead and refuse when they differ or when the snapshot was not taken for a window. An `element_identifier` whose bus owner is a different pid than the target is refused. The window is not focused. Calls without a target behave as before ([#337](https://github.com/agent-sh/computer-use-linux/issues/337)).
- Build the release binaries with cargo-zigbuild against glibc 2.28, so they run on RHEL 8, Debian 10, Ubuntu 20.04 and newer. v0.7.33 needed glibc 2.38 for the main binary and 2.43 for the indicator, so it failed on Ubuntu 22.04, and the indicator also failed on Ubuntu 24.04 and Debian 13. The release job now fails if a binary needs a newer glibc. Thanks @zqj2021 for the report and symbol analysis ([#339](https://github.com/agent-sh/computer-use-linux/issues/339)).
- Keep KWin window IDs within JavaScript's exact integer range (2^53-1), so JavaScript MCP clients can pass a `window_id` from `list_windows` back unchanged. KWin hashed the window UUID into all 64 bits, and clients that parse JSON numbers as doubles rounded the ID. KWin IDs change once with this release and stay stable per window after it. Every other backend already returned IDs in range. Tool schemas now advertise the maximum, and a `window_id` that is not an integer from 0 to 2^53-1, including a rounded ID above the u64 range, is rejected with a prompt to re-run `list_windows` instead of being matched by rounding. Thanks @Movoid for the report ([#341](https://github.com/agent-sh/computer-use-linux/issues/341)).

## [0.7.33] - 2026-10-30

### Added

- Drive Selkies desktops through the configured pixelflux computer-use API for screenshots, clicks, scrolling, drags and keyboard input. Map element targets from logical layout coordinates and keep uncertain input outcomes from being replayed.

### Fixed

- Keep pixelflux input serialized until its request completes, including after the caller cancels it.
- Use the portal's negotiated EIS pointer regions on GNOME so fractional scaling can reach the right and bottom screen edges. Keep the legacy D-Bus path when EIS is unsupported.
- Bound wlroots output discovery even when a Wayland peer stops responding, and include pixelflux in input readiness diagnostics.

Thanks @berney for the pixelflux backend ([#331](https://github.com/agent-sh/computer-use-linux/pull/331)) and @DenysFizer for the GNOME fractional-scaling report ([#330](https://github.com/agent-sh/computer-use-linux/issues/330)).

## [0.7.32] - 2026-10-29

### Added

- List and activate labwc windows through the Selkies IPC extension. Require matching compositor identities and unique window matches; omit bounds when output scaling cannot be verified.

### Fixed

- Keep native X11 clicks, scrolls and drags on the selected X display through XTEST. Repeated clicks land after another device moves the pointer, and missing xdotool returns an error without a system-wide fallback.
- Send paced intermediate motion for XTEST, uinput and portal drags. Preserve valid portal drags across monitor gaps and attempt button release after failed presses or motion.
- Keep the terminal paste chord when Konsole exposes focused container widgets, while retaining ordinary paste for editable fields.
- Read KDE logical output geometry from KWin for portal mapping and keep layout revalidation on the selected source.
- Avoid duplicate GNOME indicator hides during capture acknowledgement. Preserve compositor settling and tolerate short renderer stalls without replaying an action.
- Keep labwc window identities within JavaScript's exact integer range, reject ambiguous activation, and preserve listings containing byte-truncated UTF-8 titles.
- Snapshot forced GTK/Qt accessibility bridge state from the selected compositor so readiness and capabilities agree.

Thanks @berney for the KDE and labwc contributions ([#316](https://github.com/agent-sh/computer-use-linux/pull/316), [#317](https://github.com/agent-sh/computer-use-linux/pull/317), [#318](https://github.com/agent-sh/computer-use-linux/pull/318)), @qixuancao for the GNOME capture fix ([#319](https://github.com/agent-sh/computer-use-linux/issues/319), [#320](https://github.com/agent-sh/computer-use-linux/pull/320)), and @martin152 for the X11 pointer and drag reproductions ([#321](https://github.com/agent-sh/computer-use-linux/issues/321), [#322](https://github.com/agent-sh/computer-use-linux/issues/322), [#323](https://github.com/agent-sh/computer-use-linux/issues/323), [#326](https://github.com/agent-sh/computer-use-linux/pull/326)).

## [0.7.31] - 2026-10-28

### Fixed

- Use per-monitor RemoteDesktop portal streams for COSMIC pointer coordinates. Its uinput absolute pointer maps onto the active monitor, which could send clicks to the wrong window on mixed-size displays.
- Use COSMIC's EIS connection when available and wait for its input devices to resume before sending the first event, preventing text prefixes and clicks from being lost during portal startup.
- Preserve the receiving window or app target across portal consent and verify its focus again before typing, key chords, and targeted pointer input. Cancelled portal keyboard startup and failed focus verification send no input.
- Verify existing GNOME Introspect app focus without calling Shell FocusApp, which opens the overview. Window activation requires the GNOME Shell extension.
- Read COSMIC window geometry and logical output origins for targeted screenshots and relative input. Wait for both snapshots and keep window IDs within JavaScript's exact integer range. PID remains optional because the protocol does not expose it.

Thanks @madnix007 for the report ([#312](https://github.com/agent-sh/computer-use-linux/issues/312), [#313](https://github.com/agent-sh/computer-use-linux/pull/313)).

## [0.7.30] - 2026-10-28

### Changed

- Return from short commands as soon as they finish, instead of polling their output every 10 ms. On KDE Plasma, `doctor` takes about 43 ms instead of 221 ms, and a `get_app_state` call that reuses cached diagnostics takes about 50 ms less.

Thanks @ofek29 for the performance improvement and regression coverage ([#308](https://github.com/agent-sh/computer-use-linux/pull/308)).

## [0.7.29] - 2026-10-27

### Changed

- Reuse the desktop diagnostics behind the `get_app_state` readiness block for up to 30 seconds, so a repeat call on KDE Plasma takes about 140 ms less and starts 6 of the 26 diagnostic subprocesses. Those 6 recheck accessibility on every call, so switching it on or off takes effect at once. A call runs all the diagnostics before it answers when it passes `verbose: true`, when its screenshot, window lookup, or accessibility tree read fails, or when it finds a window that the cached diagnostics say no backend can list. The call after a failed observation, a failed action, or a setup tool runs them too, and `doctor` always runs them.

Thanks @ofek29 for the performance improvement and regression coverage ([#305](https://github.com/agent-sh/computer-use-linux/pull/305)).

## [0.7.28] - 2026-10-27

### Fixed

- Report `backend: "unavailable"` when `focused_window`, `activate_window`, or `list_windows` fails, instead of claiming `gnome-shell-introspect`. On niri, KDE, Hyprland, and other non-GNOME sessions the old label pointed troubleshooting at GNOME Shell. The `windows` CLI command already used this value. When no window has focus, `focused_window` now names the backend that answered.

Thanks @lihaoze123 for the fix and @luochen1990 for the report ([#301](https://github.com/agent-sh/computer-use-linux/pull/301), [downstream issue](https://github.com/lihaoze123/niri-computer-use/issues/2)).

## [0.7.27] - 2026-10-25

### Added

- `get_tree_page` pages through the latest cached tree by node offset, for hosts that cannot read the spill file.

### Changed

- Keep tool results under 32,000 bytes when they carry a large accessibility tree: the result holds the leading nodes, and the whole compacted tree goes to a private file named in `accessibility_tree_spill`, with the same `element_index` values. A 2,000-node tree was about 1.2 MB of JSON, far past what MCP hosts show a model. The file lasts until the next snapshot or until the server exits.

## [0.7.26] - 2026-10-25

### Fixed

- Keep searching a focused Chromium document for its focused contenteditable editor, and stop claiming that typed text went nowhere when focus is on a non-editable element. Thanks @pnll1991 for the report and the patch. (#290)
- Return the partial accessibility tree, marked truncated, when a snapshot reaches its 10-second deadline, instead of failing with an empty tree.
- Allow `max_nodes` up to 10000, so a snapshot can read past a large embedded browser document to the app's own controls. The default stays 1000. (#290)
- Isolate the text preflight test from an inherited KDE desktop session. Thanks @ArcticLampyrid. (#293)

## [0.7.25] - 2026-10-24

### Fixed

- Accept global AT-SPI logical pointer coordinates with `coordinate_space: "atspi"`, map them through verified KWin window and output geometry, and preserve raw screenshot pixels as the default. (#281)
- Reject ambiguous same-process window ownership before checking a caller-supplied point, including when an explicit window ID is supplied.
- Capture through an already-running desktop portal without requiring a D-Bus activation file, supporting supervised isolated desktops.

## [0.7.24] - 2026-10-24

### Fixed

- Keep semantic recovery trees local and invalidate cached indices before a recovered action, requiring fresh state instead of silently renumbering the caller's last tree.
- Pin recovery to the observed process and exact window/backend identity, and reject name-only or PID-unproved rebinding after the original AT-SPI owner disappears. Thanks @ilysenko for the review.

## [0.7.23] - 2026-10-24

### Fixed

- Report failed desktop actions and intentional refusals as MCP tool errors with `isError: true`, while preserving successful results when only post-action observation fails. Thanks @OnlyHa1bara. (#280)
- Read stable machine-readable AT-SPI action names so Qt buttons use native click/press/toggle activation instead of pointer fallback. Thanks @sinantux for the detailed reports. (#281)
- Map KWin element clicks using coherent frame, client, output and scale geometry. Reject ambiguous window ownership, unknown desktop visibility, mixed output scales and stale pointer dimensions before sending input. (#281)
- Keep KWin Save/Open dialogs, confirmation prompts and verified interactive transients targetable, including dialogs that skip the taskbar. (#286)
- Refresh stale semantic AT-SPI targets once before action dispatch, preserving the original app scope and never replaying numeric-index or already-dispatched actions. (#282)
- Scope `get_app_state` by short or reverse-DNS `app_id` values, reject ambiguous aliases, and avoid whole-desktop fallback for unresolved explicit app IDs. (#283)
- Verify editable-text `set_value` results with exact full-content readback, report mismatches or verification failures instead of false success, and mark password fields unverifiable. (#284)
- Reject unsupported X11 PointerRoot keyboard monitoring before typing, preserve one xdotool dispatch with an explicit monitoring warning, and never replay the text. (#272)

### Security

- Update Tokio to 1.53.3 and register the GNOME indicator socket with the I/O-safe `AsyncFd::register` API, addressing RUSTSEC-2026-0386. Thanks @OnlyHa1bara. (#280)
- Update Pi and schema-check dependencies, including patched Undici, Vitest, Zod, body-parser, fast-uri and ip-address. Regenerate the bundled MCP client. (#285)

## [0.7.22] - 2026-10-20

### Fixed

- Detect keyboard grabs and focus transitions during X11 text dispatch, including a brief grab that drops a contiguous block of characters while the keymap and polled focused-window ID stay unchanged. Return an error without replaying the text. The regression exercises real byte loss, restored focus, stable ASCII/Unicode, and restored layout changes. This addresses a reproduced loss mechanism from the investigation of #272; the trigger of the reporter's original spontaneous capture is not confirmed.

## [0.7.21] - 2026-10-20

### Fixed

- Read WebKitGTK accessibility content through well-known AT-SPI child references and indexed localized action methods. Bound action discovery and revalidation so unresponsive peers do not block later input. Thanks @OnlyHa1bara. (#273)
- Return an error after X11 typing if an XKB layout replacement or a persistent mapping/group change is detected. Detect the reported br-to-us-to-br replacement even when the original layout is restored. Do not replay dispatched input. Preserve xdotool dispatch with an explicit warning when monitoring cannot start. This detects the layout-change case in #272; it does not repair dispatched text, prove field contents, detect every transient remap/group change, or fix the separately reported loss with a stable keymap. Thanks @JADRT22 for the reproduction.

## [0.7.20] - 2026-10-18

### Fixed

- Warn after X11 xdotool text dispatch that non-US or changing XKB layouts can silently substitute symbols. Require inspecting the text before submitting it. Correct the post-action test subprocess selector so its assertions execute. (#267)

## [0.7.19] - 2026-10-17

### Fixed

- MCP hosts that probe with `server/discover` before `initialize` (MCP 2026-07-28) no longer fail with "expect initialized request". The server now uses rmcp 3.5.1, keeps the connection open after an empty probe so the host can initialize, and supports metadata-bearing discovery requests.

## [0.7.18] - 2026-10-17

### Added
- Support opt-in post-action observation (`screenshot_after: true | "screen" | "window"`, `format`, `quality`, `scale`, `max_width`, `max_height`, `max_bytes`, and `include_tree: true`) on mutating tools (`click`, `drag`, `scroll`, `press_key`, `type_text`, `perform_action`, `set_value`, `activate_window`, `move_window`, and `resize_window`) so an action and its verification can complete in one MCP round trip. (#259)

## [0.7.17] - 2026-10-16

### Fixed
- Stabilize Hyprland wtype input for long text in confirmed nonsecret fields. Keep terminal and unknown-field text on private stdin, limited to 100 Unicode characters per call, and reject confirmed password fields. (#256)
- Preserve missing-focus warnings for content trees, including renderer hints computed before tree compaction. Recognize Electron editable states and qualify successful text input as dispatch-only. (#256)

## [0.7.16] - 2026-10-13

### Fixed
- Replace the KWin backend atomic update with stable Rust primitives so source builds work on Rust 1.93.1. (#252)

## [0.7.15] - 2026-10-10

### Fixed
- Treat focused AT-SPI terminals as keyboard targets without requiring an EditableText interface. Keep warnings for other non-editable controls. (#246)
- Send X11 text newlines as Return instead of Linefeed, with one newline per CRLF pair. (#247)

## [0.7.14] - 2026-10-10

### Fixed
- Wait for COSMIC focus state and verify window activation. Report missing initial state as an error, discard closed windows, and wait for complete monitor layouts. (#245)
- Preserve screenshots in native Pi codemode calls with structured results and image blocks accepted by `image()`. Expose MCP tool errors in structured results. (#243)

## [0.7.13] - 2026-10-08

### Fixed
- Remove new portal screenshot files after reading them, including invalid images and cancelled queued reads. Snapshot Pictures filenames before requesting capture, preserve existing names, and hold the returned file open so a replacement cannot reuse its inode.
- Shorten ydotool test directory names so private scratch paths fit Unix socket limits.

## [0.7.12] - 2026-10-07

### Added
- Add a native GNOME Shell activity indicator with cursor motion, click pulses, keycaps, typed-text masking, agent colors and idle fading. The helper preserves shared capture exclusion and rejects captures when hiding is not acknowledged.
- On-screen indicator, on by default: `computer-use-linux-indicator` draws a
  software cursor that glides to coordinate pointer targets and ripples on
  click, keycaps and a typing line for keyboard input,
  an edge glow, and a status pill naming the agent from the MCP `clientInfo`.
  Pointer actions wait 350 ms for the cursor to land; AT-SPI actions report
  their status without moving it. Screen captures require the capture lock
  and a valid hide acknowledgement from a running overlay. `set_value`
  values are always masked; keycaps and typed text are masked for password
  or unknown focus. Uses `wlr-layer-shell` or the GNOME Shell extension; set
  `COMPUTER_USE_LINUX_INDICATOR=0` to turn it off. Installed by `./install.sh`,
  `cargo install`, npm, and published as a release asset.

### Fixed

- Reuse desktop session discovery until its environment changes. Process screenshot files and encode images on one blocking worker so large captures keep MCP responsive without multiplying decoded-image memory.
- Refresh already-enabled GNOME extension assets during installation and report the required session restart.
- Mask generic text-box roles, terminal focus and generic enum fallback roles in typed text and keycaps. GTK obscured entries can advertise the same text-box role as public entries.
- `setup-window-targeting` enables the system extension in `/usr/share/gnome-shell/extensions` if a distribution package installed it. It installs the user extension in `~/.local/share/gnome-shell/extensions` only if there is no system extension. If there is a system extension, setup removes the user extension from earlier setups and asks for a GNOME Shell reload. Before, setup always installed the user extension, and GNOME Shell used it instead of the system extension, so package updates had no effect.
- The GNOME Shell extension supports GNOME Shell 51.
- On at-spi2-core 2.62 and later, the doctor check `screen_reader_enabled` failed with "No such property ScreenReaderEnabled", because 2.62 removed that D-Bus property. If the property does not exist, doctor now reads the `org.gnome.desktop.a11y.applications screen-reader-enabled` setting. With older at-spi2-core versions, doctor still reads the property.
- The doctor checks `at_spi_enabled` and `screen_reader_enabled` now show text such as "AT-SPI IsEnabled is false" and "Screen reader is off". Before, they showed raw D-Bus output such as `b false` or `(<false>,)`.

## [0.7.11] - 2026-10-04

### Changed
- Local development builds keep line information with less debug data, and CI host builds share their cache. Release settings are unchanged. (#225)
- Pi and schema-check dependency lockfiles address audit findings. (#221)

### Fixed

- Doctor now separates detected screenshot routes from verified capture. A detected route reports `screenshot_capture_status: "unverified"` and `can_capture_screenshots: false`; use `get_app_state` with a screenshot to obtain `verified` or `failed`. Unverified capture is a setup warning, and callers should not treat the boolean alone as a missing backend.
- GNOME literal text prefers the portal and rejects strings its keymap cannot represent before typing. Wayland coordinate input refuses inaccurate relative-motion fallback. (#226)
- Portal wheel direction matches compositor axis signs on GNOME and KDE. (#226)
- Screenshot errors retain their backend causes, and diagnostic subprocesses have bounded execution and cleanup.

## [0.7.10] - 2026-10-01

### Fixed
- MCP safety checks recognize npm launcher error output without weakening
  the failure assertions. (#214)

### Security
- COSMIC targeted input trusts compositor-reported focus rather than an
  activation cache, and no longer writes focus state into a shared temporary
  file. GNOME extension calls verify the unique owner of `org.gnome.Shell`. (#215)
- Portal responses are bound to the expected sender and request path.
  Screenshot files reject symlinks and non-regular files and enforce byte and
  decoded-pixel limits before processing. (#215)
- Automatic ydotool discovery accepts only private, same-user sockets and
  no longer falls back to shared `/tmp` sockets. The installer creates a
  mode-0600 daemon socket; `YDOTOOL_SOCKET` remains an explicit override. (#215)
- Scroll amounts must be finite and within -100 to 100 before input locks
  are taken. Pi executable and PATH overrides must use absolute paths. (#215)

## [0.7.9] - 2026-10-01

### Fixed
- Screenshot execution failures return MCP tool results with `isError: true`
  and the original error message. Window targeting, capture, crop, and resize
  failures no longer surface as JSON-RPC internal errors that some clients
  mistake for broken connections. (#210)
- KWin temporary plugin sequences use the current atomic update API so
  builds with warnings denied pass on Rust 1.99. (#211)

## [0.7.8] - 2026-10-01

### Added
- Official Claude Code and Codex plugin. The repo is a marketplace for both
  hosts: `claude plugin marketplace add agent-sh/computer-use-linux` or
  `codex plugin marketplace add agent-sh/computer-use-linux`. The plugin ships
  the skill and a launcher that downloads the release binaries pinned to the
  plugin version, verifies their sha256, and caches them per version. The
  Codex manifest forwards the desktop session variables that Codex strips
  from MCP server environments. `COMPUTER_USE_LINUX_BIN` runs a local build. (#202)

### Fixed
- Native X11 queries now enforce one deadline in the transport itself.
  Connect, handshake, and every reply share a 2-second bound, so a stalled
  X server closes the query connection instead of leaving a blocked worker
  running after a timeout. The `doctor` X11 check no longer spawns a
  detached thread. Unix sockets, literal IP addresses, and `localhost` need
  no resolver; a remote hostname in `DISPLAY` resolves through
  `getent ahosts` under the same bound, and a missing or failing lookup
  reports the X11 route as unavailable. Fix by Gary Lysenko (ilysenko),
  from ilysenko/codex-desktop-linux#1532. (#203)
- Accessibility snapshots stay stable during input. The cached nodes and
  the pid that owns them are now one snapshot, and `get_app_state`
  publishes a new one only after in-flight input finishes. An index that
  passed the owner check can no longer resolve to another app's node
  mid-action. `perform_action` and `set_value` now take the input lock that
  `click` and `scroll` already held. Fix by Gary Lysenko (ilysenko), from
  ilysenko/codex-desktop-linux#1532. (#203)
- PTY metadata alone no longer makes a window a paste-capable terminal. The
  paste chord comes only from a known terminal app id or WM_CLASS, so an
  IDE with an integrated terminal keeps the standard paste shortcut. PTY
  enrichment now also recognizes Ghostty, GNOME Terminal, and KGX by app
  id or WM_CLASS, not only by title, which keeps enrichment working for
  X11 windows with a custom class and a plain title. This replaces the
  earlier rule that defaulted an enriched window with no known identity to
  Ctrl+Shift+V. Fix by Gary Lysenko (ilysenko), from
  ilysenko/codex-desktop-linux#1532. (#203)
- The bounded process runner no longer sleeps after every chunk while a
  command is still producing output. Each drain stays capped and the
  deadline is still checked every pass, so continuous output cannot starve
  stderr or bypass the output limit. Fix by Gary Lysenko (ilysenko), from
  ilysenko/codex-desktop-linux#1532. (#203)

## [0.7.7] - 2026-09-30

### Added
- Window listing and exact focus on niri through `niri msg`, with direct
  JSON IPC as a fallback when the CLI is unavailable. Socket discovery
  matches the current Wayland display and refuses ambiguous sessions.
  Focus is verified with a fresh window query before targeted input.
  Window bounds account for output scaling and window offsets. Missing
  positions stay null; unknown or mixed scaling omits bounds. (#192)

## [0.7.6] - 2026-09-29

### Fixed
- Wayland `press_key` on the remote-desktop portal now sends modifiers and
  named keys (Enter, arrows, F-keys and the rest) as keysyms, so the
  compositor resolves them against the active keymap. With Caps Lock and
  Control swapped, `Ctrl+A` used to press the physical Control position,
  which toggled Caps Lock and typed `A`. It now selects all. Letters and
  digits stay physical keycodes: mutter drops a keysym missing from the
  current layout group, so a keysym `a` would vanish under a Hebrew or
  Cyrillic layout, and GTK and Qt match shortcuts on those layouts by the
  US keycode. KDE Plasma keeps keycodes for now. (#191)

## [0.7.5] - 2026-09-28

### Added
- Opt-in persistence for the Wayland remote-desktop portal. Set
  `COMPUTER_USE_LINUX_PERSIST_REMOTE_DESKTOP=1` to send `persist_mode=2`
  on `RemoteDesktop.SelectDevices` and reuse the single-use `restore_token`
  returned by `Start`. Nothing persistence-related is sent on
  `ScreenCast.SelectSources` (that call rejects it for a remote-desktop
  session). Pointer and keyboard sessions store separate tokens, mode
  `0600`, under `$XDG_STATE_HOME/computer-use-linux/` or
  `~/.local/state/computer-use-linux/`. The first grant still shows the
  dialog. Later processes restore until the desktop revokes the grant, or
  until the token cannot be restored, which falls back to a prompt.
  Interface version 2 is required. Unset, behavior is unchanged. (#185)

### Fixed
- X11 `scroll` no longer follows ydotool's absolute move with a single wheel
  event. That move warps the pointer through (0, 0). GTK 3 resets its XI2
  scroll valuators when the pointer re-enters the window, so the one wheel
  event only re-baselines them and the window does not move, while the tool
  still returns success. Native X11 now sends the same notch count as XTEST
  wheel buttons through xdotool (4 up, 5 down, 6 left, 7 right), which have
  no scroll valuators. Wayland portal scroll is unchanged. (#184)

## [0.7.4] - 2026-09-27

### Fixed
- The #167 index check no longer trusts a pid the snapshot did not match.
  When `get_app_state` named a pid with no accessibility root plus an app
  name, the tree came from the app-name match but was recorded under the
  requested pid. An index from that tree then passed a click aimed at the
  requested pid and was refused for the app that owns it. The snapshot now
  records a pid only when its roots were selected by that pid; otherwise each
  node is checked against its owner on the accessibility bus.

## [0.7.3] - 2026-09-26

### Fixed
- `click` and `scroll` by `element_index` or selector now refuse an index from
  another app's snapshot. `get_app_state` records the pid it snapshotted;
  when the action's target resolves to a different pid, the call fails and
  asks for a `get_app_state` of the target, instead of acting on the other
  app's node and reporting `ok: true`. An untargeted snapshot mixes apps, so
  its node is checked against the pid that owns it on the accessibility bus.
  (#167)
- Portal pointer input on scaled GNOME Wayland (for example 125 %) now lands
  where it is aimed. mutter maps a stream point to `monitor.x + stream_x /
  scale` when its layout mode is logical, which is the GNOME 50 default, so
  logical coordinates arrived at 1/scale of the target and still reported
  success. The pointer path reads the layout mode from
  `org.gnome.Mutter.DisplayConfig` and sends each stream's point multiplied
  by its monitor scale. Click, scroll, and drag share the path. Unscaled
  monitors, physical layout mode, and other compositors are unchanged. (#169)
- The Pi extension falls back to `computer-use-linux` on `PATH` when neither
  `COMPUTER_USE_LINUX_BIN` nor the downloaded package binary is available.
  Temporary extensions (`pi -e npm:@agent-sh/computer-use-linux`) are staged
  without that binary and warned "binary not found" even with a global
  install. The PATH lookup was dropped in the native-tools rewrite. (#170)
- `type_text` and `press_key` no longer warn "no focused element" when the
  focus probe merely ran out of budget. The 400-node search now reports when a
  node, depth, or read limit stopped it, and the feedback says the input could
  not be verified instead. With a target pid, the probe searches only that
  app; when no AT-SPI app belongs to it (xterm, Electron without
  `--force-renderer-accessibility`), feedback says so instead of warning or
  reporting another app's focused widget. (#168)
- KDE Plasma clipboard paste into xterm, uxterm, rxvt, urxvt, and koi8rxterm
  now sends Shift+Insert. Those terminals have no Ctrl+Shift+V binding by
  default, so the paste was dropped; Klipper sets the selection as well as
  the clipboard, so Shift+Insert pastes the new text. When AT-SPI reports a
  focused element that is not a terminal (a search field in a terminal
  window), paste falls back to Ctrl+V. The check reads the AT-SPI role enum,
  not the localized role name. Behavior ported from
  ilysenko/codex-desktop-linux#1413.
- Helper spawns (ydotool, wtype, xdotool, gnome-screenshot, and every command
  run through the supervised runner) retry a transient `ETXTBSY` ("Text file
  busy") for up to 75 ms. The error appears when a helper binary is being
  replaced, and it made xdotool fall back to ydotool as if it were missing.
  It also made tests that write and exec fake helper scripts fail under the
  parallel suite, which failed the first v0.7.2 tag run.

## [0.7.2] - 2026-09-26

### Added
- Native X11 screenshot route: one `GetImage` on the root window over the
  X11 connection, used only on a native X11 session (never XWayland) and
  tried after GNOME Shell and the portal, before `gnome-screenshot`.
  Pixels are device pixels, the space xdotool input and X11 window origins
  use. MATE/X11 had no working route: xdg-desktop-portal-gtk has no
  Screenshot, and gnome-screenshot 41 blacks out 3/4 of the frame at
  window-scaling-factor 2. `doctor` reports the route as
  `platform.x11_display` and `capabilities.screenshot` entry `x11`;
  `COMPUTER_USE_LINUX_SCREENSHOT_BACKEND=x11` pins it. (#155)

### Fixed
- `doctor` no longer reports the XDG Screenshot, ScreenCast, or InputCapture
  portal as available when the portal does not export that interface.
  `busctl introspect` exits 0 with only a header line for a missing interface,
  so each entry now requires its methods (Screenshot: `Screenshot`). Readiness
  gains `can_capture_screenshots` and a blocker when no screenshot route is
  detected; this is detection, not a test capture. (#156)
- X11/EWMH window origins now come from the X server
  (`TranslateCoordinates` to the root window) instead of `wmctrl -lG`, which
  counts the client's offset inside its frame twice. Window-targeted
  screenshots cropped the wrong area, relative clicks were offset by the same
  amount, off-screen warnings misfired on maximized windows, and
  `move_window` reported a mismatch after a correct move. Bounds are the
  client area; `move_window` now verifies the frame origin it requested. When
  the X server cannot be asked, the origin is reported unknown instead of the
  shifted wmctrl value. (#157)

## [0.7.1] - 2026-09-19

### Changed
- `get_app_state` now returns its screenshot as a structured `image` content
  block (the same shape as the `screenshot` tool) followed by the JSON report
  as text and `structuredContent`. The JSON `screenshot` field keeps the
  metadata (dimensions, scale, format, bytes) but no longer embeds the base64
  `data_url`, which hosts were counting as ~30K text tokens per call. Callers
  that read `screenshot.data_url` from the JSON should read the image block
  instead. (#145)

### Fixed
- `type_text` on X11 no longer delivers characters out of order. The xdotool
  backend passed `--delay 0`, which lets XTEST key events race each other on
  some X servers (seen on Cinnamon / Mint 22). It now uses xdotool's 12ms
  per-character default and scales the command timeout with text length.
  `COMPUTER_USE_LINUX_XDOTOOL_TYPE_DELAY_MS` overrides the delay. (#147)
- The unscoped-tree warning from `get_app_state` no longer tells a caller that
  already passed a `pid` or window target to pass a target. When the target
  matched no AT-SPI application root, the warning now points at the app's
  missing accessibility support, `list_apps`, and a lower `max_nodes` instead.

## [0.7.0] - 2026-09-16

### Added
- `get_app_state` reports `tree_scoped` and `accessibility_tree_truncated`,
  warns in `message` when no app target narrowed the AT-SPI tree to one app,
  and documents every scoping parameter in its schema, the server
  instructions, the skill, and the Pi guide. Unscoped calls used to return the
  whole desktop tree silently and could exhaust a small context window.

### Fixed
- Skill Install section now separates Pi native install from PATH CLI/MCP
  install, so `pi install` is not described as sufficient for shell `doctor`
  and `setup`.

## [0.6.0] - 2026-09-15

### Added
- GTK4 delivered-event coordinate probe and an opt-in GTK3/MCP semantic-click
  regression that verifies actual button activation at scales 1 and 2.
- Explicit foreground `guard-accessibility` command with a passive AT-SPI
  listener and verified GNOME toolkit-accessibility reassertion. Ctrl-C or
  SIGTERM stops the guard without disabling other accessibility clients or
  restoring an old saved setting. Never started automatically by MCP or setup.
- Optional `complete_interaction` MCP notification, enabled by
  `COMPUTER_USE_LINUX_NOTIFY_ON_COMPLETE=1`, with bounded notification execution
  and graceful handling of unavailable desktop notification services.

### Fixed
- KDE Plasma terminal paste uses terminal shortcuts, including uxterm detection.
- Setup verifies saved GNOME accessibility settings instead of reporting an
  unverified write as success.
- Plain left element/selector clicks prefer recognized native AT-SPI activation
  actions over toolkit bounds, avoiding GTK3 HiDPI and GTK4 zero-origin pointer
  conversion when a matching action is available. Explicit coordinates, right
  clicks, and multi-clicks preserve pointer semantics.
- Clarified that relative click/scroll coordinates use the clipped window
  screenshot crop origin before preview resizing, not raw surface or widget
  coordinates, and require a window target.

## [0.5.0] - 2026-08-31

### Added
- The Pi package now exposes native `computer_use_linux_*` tools without a
  separate MCP adapter. One small loader stays active initially, then uses
  Pi's additive dynamic-tool loading to expose selected tools with their exact
  generated MCP schemas only when desktop control is needed.
- Pi keeps one lazily started, session-scoped computer-use-linux process,
  forwards cancellation, serializes desktop actions, preserves image results,
  bounds text output, and rejects binary/schema version drift.

### Changed
- Pi installation is now one command (`pi install
  npm:@agent-sh/computer-use-linux`) and no longer writes
  `~/.pi/agent/mcp.json`. Existing adapter-based entries are detected and
  reported for non-destructive manual cleanup.

## [0.4.10] - 2026-08-29

### Added
- An explicitly opt-in `run_shell` MCP tool supports bounded same-user shell
  execution for trusted remote MCP deployments. The tool is absent unless
  `COMPUTER_USE_LINUX_ENABLE_SHELL=1`, clears ambient credentials, requires
  visible environment additions, enforces timeout/output limits and process-
  group cleanup, and emits command-digest audit records.

### Fixed
- Wayland literal text now uses `wtype` on compatible compositors when portal
  keyboard input is unavailable, preserving Unicode before the ydotool
  fallback. Known-incompatible GNOME, KDE/Plasma, and COSMIC sessions do not
  advertise or select wtype, and a launched failure never replays the text.
- Absolute uinput pointer axes now end at the final logical desktop pixel, so
  edge coordinates are advertised and clamped consistently.
- Capability maps now advertise AT-SPI only when its bus is reachable and a
  toolkit accessibility status is actually enabled.
- Buttons outside the absolute uinput device's left, middle, and right set now
  fall through to a backend that can synthesize them instead of becoming left clicks.
- Temporary KWin script callbacks now accept one matching response from the
  current `org.kde.KWin` bus owner, reject spoofed or replayed responses, and
  time out the complete script transaction before cleaning up owned temporary
  state without disturbing a colliding callback registration.
- KWin window listings now classify Plasma 6 native Wayland and Xwayland
  clients when the legacy client flags are unavailable.
- GNOME extension setup now reports when changed files require an already-active
  Shell extension to reload before its newly installed DBus methods are served,
  and requires that reload when the previous extension state cannot be read.

## [0.4.9] - 2026-08-12

### Fixed
- Bounded AT-SPI traversals now use concurrent indexed child reads, count both
  queued references and failed child lookups against their work budgets, and
  enforce a snapshot deadline so wide, malformed, or stalled accessibility
  trees cannot allocate or retain unbounded work.

## [0.4.8] - 2026-08-12

### Fixed
- AT-SPI state capture now reaches deeply nested GTK4 controls such as Nautilus
  file cells by using a 1,000-node default budget and depth-32 traversal, with
  bounded 2,000-node and depth-64 limits shared by the MCP and CLI paths.
- `install.sh` now treats unavailable window-targeting or exact-focus support as
  a degraded platform capability when MCP registration, accessibility trees,
  and development input are ready, while missing core prerequisites remain hard
  failures. The same classification applies when `jq` is unavailable.

## [0.4.7] - 2026-08-09

### Fixed
- `install.sh` now recognizes Artix as pacman-based, chooses an explicitly
  requested or unambiguous package manager for unknown distros, treats
  ydotool as an optional fallback, installs the required xdotool keyboard
  backend on X11, distinguishes pointer-only direct uinput from keyboard-ready
  input in `doctor`, reports Wayland RemoteDesktop pointer availability
  separately from its required keyboard contract, and skips automatic ydotoold
  setup when a systemd user manager is unavailable.

## [0.4.6] - 2026-08-05

### Fixed
- Native X11 coordinate clicks now use one supervised
  `xdotool mousemove -- X Y click --repeat N BUTTON` command after the
  absolute pointer and eligible portal paths, with `ydotool` fallback only
  when `xdotool` cannot be spawned. Standard left, middle, and right buttons
  are supported; extended buttons retain the existing fallback path. Set
  `COMPUTER_USE_LINUX_FORCE_YDOTOOL_POINTER=1` to skip xdotool.

## [0.4.5] - 2026-08-01

### Fixed
- Stateful ydotool operations now complete while retaining the serialized input
  lock after caller cancellation, preventing paired button or key events from
  being interrupted. Drag also attempts button release after intermediate
  failures.
- Scaled KWin sessions now map logical window geometry through the compositor's
  virtual screen geometry while preserving logical portal input coordinates.
- Hyprland window bounds now map against grim's complete global output union,
  including negative origins and mixed or fractional scales, and are omitted
  when monitor metadata is unavailable or invalid.
- Mirrored KScreen outputs are excluded from the portal monitor layout.

## [0.4.4] - 2026-07-31

### Fixed
- KWin window listing and activation now support both Plasma 6
  (`windowList`/`activeWindow`) and Plasma 5 (`clientList`/`activeClient`).
- ydotool is now used only when its CLI, daemon, and socket support the raw
  event semantics Computer Use emits. Compatibility probing works without
  `XDG_RUNTIME_DIR`, uses an isolated private socket, and rejects CLI errors
  that are returned with a successful exit status.
- X11 `xdotool type` now disables per-character delay, and a command that
  starts but fails is no longer replayed through ydotool. The ydotool fallback
  is limited to failures to launch xdotool, avoiding duplicate partial input.

## [0.4.3] - 2026-07-31

### Fixed
- Hyprland window activation now checks `hyprctl dispatch` stdout instead of
  trusting its exit status, so an exit-zero `Invalid dispatcher` response falls
  through to the compatible focus dispatcher. (#62)
- Modifier chords sent through the ydotool fallback now include a 100 ms
  inter-event delay so applications such as Firefox do not drop simultaneous
  modifier and key events. (#63)

## [0.4.2] - 2026-07-25

### Fixed
- X11 keyboard input now goes through `xdotool` (XTEST) instead of ydotool.
  ydotool injects raw evdev scancodes into a virtual uinput device, which X11
  re-interprets through the active XKB layout, so `press_key "Return"` and
  chords like `ctrl+a` landed as stray characters, and `type_text` mangled
  symbols and digits (`_` → `%`, `1` → `+`) even on a plain US layout. XTEST
  resolves keysyms against the live layout. Wayland behaviour is unchanged, and
  the ydotool path remains the fallback when `xdotool` is missing or fails.
  `doctor` now reports an `xdotool` input backend on X11. Override with
  `COMPUTER_USE_LINUX_FORCE_YDOTOOL_KEYBOARD=1` (opt out) or
  `COMPUTER_USE_LINUX_FORCE_XDOTOOL_KEYBOARD=1` (force on). (#58)
- Portal `scroll` direction on KDE Plasma Wayland: vertical discrete axis steps
  are inverted for the xdg-desktop-portal-kde discrete path so `direction:
  "up"|"down"` matches viewport motion. ydotool / REL_WHEEL polarity is
  unchanged. Override with `COMPUTER_USE_LINUX_PORTAL_SCROLL_INVERT=0|1`. (#56)

## [0.4.1] - 2026-07-15

### Fixed
- Targeted `get_app_state` screenshots now crop to the resolved target window,
  and refuse the full-desktop fallback when a target was requested but window
  bounds cannot be resolved or the window is entirely off-screen. (#48)
- Hyprland window bounds are normalized against each window's monitor origin
  and fractional scale so portal screenshot coordinates align on
  multi-monitor and scaled (e.g. 1.8x/2.0x) setups. (#48)
- Wayland sessions are recognized via `WAYLAND_DISPLAY` when
  `XDG_SESSION_TYPE` is unavailable, so the portal input fallback engages in
  environments that don't export the session type. (#48)
- Window focus verification now allows up to one second for compositor
  workspace/focus transitions before reporting activation failure. (#48)

## [0.4.0] - 2026-07-15

### Added
- Generic X11/EWMH window backend so `list_windows`, `focused_window`,
  `activate_window`, `move_window`, and `resize_window` work on X11 window
  managers without a dedicated backend (Cinnamon/Muffin, MATE/Marco,
  Xfce/xfwm4, Openbox, and others). (#41)
- Pi coding agent extension package: `pi install npm:@agent-sh/computer-use-linux`
  now provides Linux desktop control tools through `pi-mcp-adapter`'s MCP
  proxy without a separate pi-specific skill. (#42)

### Fixed
- Hyprland 0.55 exact window focus: `activate_window` now tries the 0.55 Lua
  focus dispatcher (`hl.dsp.focus`) first and falls back to the legacy
  `focuswindow` dispatcher on older releases. (#45)
- Exported MCP tool schemas no longer carry the non-standard `uint`/`uint8`/
  `uint16`/`uint32`/`uint64`/`usize` `format` annotations emitted by
  `schemars`, which produced repeated warnings in MCP clients; integer types
  and numeric constraints are preserved. (#45)
- GNOME extension package bootstrap on Ubuntu no longer tries to install the
  nonexistent `gnome-shell-extension-tool` apt package; `gnome-shell` is
  installed only when `gnome-extensions` is missing and apt can provide it. (#40)

## [0.3.1] - 2026-07-03

### Fixed
- A scroll with a window target but no x/y/element_index focused the window and
  then scrolled whatever sat under the cursor while reporting success.
  Window-targeted scroll now defaults its point to the centre of the resolved
  window (and errors with a pass-x/y hint when bounds are unavailable). Scroll
  results also gain the same off-screen coordinate warning click already had.

## [0.3.0] - 2026-07-03

### Added
- Exposed a focused Rust library surface for downstream diagnostics,
  accessibility snapshots, screenshots, and server integration while preserving
  the existing CLI binaries and standalone naming. (#35)
- Input-landing feedback: targeted keyboard input results append
  focused-element feedback from AT-SPI and warn when no editable element holds
  focus.
- Off-screen detection: screenshot, click, and input results warn when the
  target window or coordinate is partially or fully off-screen.
- Window geometry tools: `move_window` and `resize_window` via the GNOME Shell
  extension backend.

## [0.2.9] - 2026-06-22

### Fixed
- GTK4 applications (Nautilus, Text Editor, baobab, and others) now return their
  full accessibility tree instead of a single `role: "unknown"` root with
  `child_count: 0`. Reads were routed through the `atspi` `P2P` trait's
  `object_as_accessible`, whose no-peer fallback builds a proxy with a path but
  no destination; on the shared a11y bus that fails with `ServiceUnknown` for
  any app that does not advertise a peer-to-peer bus address. Modern GTK4 apps
  do not implement the legacy `GetApplicationBusAddress`, so they hit the broken
  fallback while GTK3/Chromium/Electron apps kept working. Reads now use
  `ObjectRefExt::as_accessible_proxy`, which always pins the destination to the
  object's bus name. (#31)

## [0.2.8] - 2026-06-17

### Changed
- Published a metadata-only patch release so local and downstream consumers can
  pin the already-validated `computer-use-linux` package state.

## [0.2.7] - 2026-06-16

### Changed
- The COSMIC helper source path and runtime override surface now use standalone
  `computer-use-linux` naming only.

### Fixed
- Window-relative clicks now require a verified target window and resolved
  bounds before coordinates are translated, preventing clicks from silently
  landing against stale or originless window data.
- Long `ydotool type --file -` input now gets a bounded timeout with both a
  fixed process budget and a text-length budget while stdout/stderr are drained
  asynchronously.
- KDE clipboard text input now uses the session DBus API directly and waits
  long enough for large paste payloads before restoring the previous clipboard;
  Klipper proxy creation and method calls share the same bounded DBus timeout.
- Failed accessibility tree extraction clears cached nodes so later
  element-targeted actions cannot use stale coordinates.

## [0.2.6] - 2026-06-06

### Added
- `COMPUTER_USE_LINUX_SCREENSHOT_BACKEND` to force a single screenshot backend
  (`gnome-shell`, `portal`, or `gnome-screenshot`), skipping the fallback chain
  for pinned/background deployments and debugging.
- `doctor` now probes `gnome-screenshot` and lists it under
  `capabilities.screenshot` when present.

### Fixed
- Screenshots now work from background processes (systemd user services,
  non-interactive parent shells) on GNOME Wayland. The GNOME Shell DBus method
  rejects callers that do not own an allowlisted bus name, and the XDG portal
  cancels non-interactive requests when there is no foreground window, so both
  prior backends failed in that context. `gnome-screenshot` is now a third
  capture fallback that works regardless of session context, bounded by a 20s
  timeout so a hung capture degrades to a clear error instead of blocking.

## [0.2.5] - 2026-06-05

### Added
- Added build-time GNOME extension / DBus identity overrides (`CUL_*`) so the
  `codex-desktop-linux` embedded copy can share this source while keeping its
  Codex extension identity, plus runtime `CODEX_COMPUTER_USE_*` aliases for the
  embedded input/backend knobs.

### Changed
- Cross-referenced the sibling `agent-workspace-linux` project in the README.

### Fixed
- Bounded screenshot payloads by default before returning them to MCP hosts,
  while exposing opt-in screenshot sizing controls and coordinate metadata for
  downscaled captures.
- Added opt-in JPEG screenshot output with a caller-selected quality so agents
  can choose compression before the byte cap forces additional resizing.
- Ported downstream Linux readiness fixes: `doctor` now treats direct
  `/dev/uinput` and the XDG RemoteDesktop portal as valid development-input
  backends instead of requiring `ydotoold` in every ready setup.
- Ported downstream session hydration fixes for X11 launches by carrying
  `XAUTHORITY` through environment hydration and checking the same-user namespace
  init process when it owns the graphical session environment.

### Security
- Pinned the release upload GitHub Action to a commit SHA in CI.

## [0.2.4] - 2026-05-25

Primarily a documentation release that refreshes the crates.io and npm README
pages; also bumps the MCP server's advertised version string to match.

### Added
- Documented the `screenshot` MCP tool and the `doctor` capability map, which
  were missing from the README tool list and the MCP safety-contract table.
- A new "Environment variables" section covering runtime overrides
  (`CU_DISABLE_ABS_POINTER`, the `COMPUTER_USE_LINUX_FORCE_PORTAL` /
  `FORCE_YDOTOOL` pointer and keyboard knobs) and the npm wrapper install knobs
  (`COMPUTER_USE_LINUX_BIN`, `DOWNLOAD_BASE`, `SKIP_DOWNLOAD`, `LOCAL_*`).

### Changed
- Stopped pinning explicit versions throughout the docs (README, npm README,
  Hermes skill). Install commands are now bare and download links use GitHub's
  `/releases/latest` redirect, so the docs no longer drift on every release.
- Dropped the `version` field from the Hermes skill frontmatter (optional per
  the agentskills.io standard) so it no longer mirrors the tool version.
- Friendlier README opening: warmer tagline and a verb-driven summary, with no
  sections removed.

### Fixed
- `install.sh` now reads the current `doctor` readiness schema
  (`.readiness.blockers`, empty array means ready) instead of the removed
  `.ready` / `.checks` fields, so a fully provisioned system reports ready
  instead of always failing the doctor step. Restored the `install.sh`
  executable bit. (#9)

## [0.2.3] - 2026-05-22

### Added
- A `sync reminder` CI workflow (`.github/workflows/sync-reminder.yml`) that
  opens (or updates) a `codex-sync`-labeled issue when a merge to `main` touches
  the crate sources (`src/**`, `Cargo.toml`, `gnome-shell-extension/**`), so the
  change can be propagated into the `codex-desktop-linux` embedded copy with its
  codex naming re-applied.

### Changed
- Bumped to establish version-enumeration parity with the `codex-desktop-linux`
  embedded copy (`0.2.3-linux-alpha1`). The two crates are kept on the same
  enumeration on purpose: a mismatch signals that a sync between them is pending.

## [0.2.2] - 2026-05-21

### Added
- Added a uinput absolute pointer (`abs_pointer.rs`) that creates a private
  `ABS_X`/`ABS_Y` device mapped to the portal screenshot coordinate space, so
  clicks land at the requested pixel on multi-monitor / HiDPI setups instead of
  being distorted by pointer acceleration and fractional scaling. Wired into
  `click()` with `ydotool` fallback; opt out via `CU_DISABLE_ABS_POINTER`.
- Added a Hermes-compatible skill tap at `skills/computer-use-linux/SKILL.md`
  and an `agnix` CI gate for agent-sh skill/config hygiene.
- Added agent-sh project-health files: `CONTRIBUTING.md`, `SECURITY.md`,
  `CODE_OF_CONDUCT.md`, and `CODEOWNERS`.
- Synced upstream Linux Computer Use text-input improvements: Wayland remote
  desktop portal keyboard sessions, KDE/Plasma clipboard paste fallback for
  layout-safe `type_text`, and literal keysym typing on non-KDE Wayland
  sessions before falling back to `ydotool`.
- Synced upstream Hyprland/session hydration fixes, including systemd user
  environment discovery, common command path hydration, `HYPRLAND_INSTANCE_SIGNATURE`
  inference, and rounded window-id disambiguation.

### Changed
- Updated repository, release, package, and CI links from `avifenesh` to the
  `agent-sh/computer-use-linux` org repo.
- `setup_accessibility` and `doctor` now understand the AT-SPI
  `org.a11y.Status IsEnabled` path in addition to GNOME toolkit accessibility.

### Fixed
- Omitted the debug-only `received` echo field from the generated MCP
  `outputSchema` for `ActionOutput` and `ActivateWindowOutput`. `schemars`
  serialized `Option<serde_json::Value>` as the boolean schema `true`, which
  strict MCP clients (mcphub, Claude Desktop, `@modelcontextprotocol/sdk`'s
  `AssertObjectSchema`) reject, failing the whole `tools/list` response.
  Affected `click`, `drag`, `perform_action`, `press_key`, `scroll`,
  `set_value`, `type_text`, and `activate_window`. (#1)

## [0.2.1] - 2026-05-14

### Added
- npm wrapper package (`@agent-sh/computer-use-linux`) for Node.js users. It
  downloads and verifies the matching GitHub release binaries at install time.
- Tag-driven GitHub Actions publishing for crates.io and npm using repository
  secrets.
- CI release gates for locked Rust checks, clippy, tests, private rustdoc,
  cargo publish dry-run, cargo audit, npm wrapper smoke tests, and an MCP
  protocol/safety contract check.
- MCP `ToolAnnotations` that mark read-only observation tools separately from
  mutating desktop-control tools.

### Changed
- Switch the COSMIC protocol dependency from a pinned Git revision to the
  published `cosmic-protocols` `0.2.0` crate so `computer-use-linux` can be
  published on crates.io.
- Ship the `computer-use-linux-cosmic` helper alongside prebuilt release
  binaries and install it from `install.sh` so COSMIC window targeting works
  outside `cargo install`.
- Keep the MCP `serverInfo.version` aligned with the Cargo package version.
- Remove unused direct `libc` and `png` dependencies from the crate manifest.
- Add Hermes Agent CLI setup commands and clarify the registered MCP tool
  names / toolset.
- Add npm install instructions and fix the prebuilt binary install example to
  match the release assets, which are raw binaries with `.sha256` files rather
  than tar archives.
- Document the mutating-tool safety contract for MCP hosts and npm users.
- Pin README and npm README install examples to the released `0.2.1` packages
  and `v0.2.1` GitHub release assets.

## [0.2.0] - 2026-05-14

This release tracks [`ilysenko/codex-desktop-linux`](https://github.com/ilysenko/codex-desktop-linux) through commit `4d6fd96` (May 2026), then re-applies the rebrand (DBus names, env vars, GNOME extension UUID, cache-file prefixes). Upstream credit goes to ilysenko, mosesmrima, PinguuSS, and the original Codex contributors.

### Added
- **Multi-compositor window targeting**: native backends for **KWin**
  (KDE Plasma), **Hyprland**, **i3**, and **COSMIC** Wayland alongside
  the existing GNOME Shell backend. Window listing, focus tracking, and
  activation now work across all five compositors with automatic backend
  selection at runtime.
- **COSMIC Wayland helper** (`computer-use-linux-cosmic` binary) that
  speaks the `zcosmic_toplevel_info_v1` and `zcosmic_toplevel_manager_v1`
  protocols, used by the main server when running under COSMIC.
- **`windowing/` crate-internal module** consolidating all backends behind
  a uniform `WindowBackend` trait, with a registry that picks the right
  backend per session.
- **Datagram ydotool socket support** in addition to the existing stream
  sockets. Aligns with `ydotoold`'s newer default and avoids a
  reconnection penalty per input event.
- **Raw-keycode keyboard input** path for ydotool, fixing keystroke
  delivery on layouts where the symbolic keysym path was unreliable.
- **Compact Linux accessibility trees** in `get_app_state`: deduplicated
  redundant container nodes for smaller, more focused snapshots.
- **Enriched AT-SPI state readback**: element states (`focused`,
  `selected`, `expanded`, `checked`, …) now flow through to the response
  schema for `get_app_state`.

### Changed
- Server-side rejection of empty window-backend results (returns a
  structured "no backend available" error instead of an empty list).
- Stale-client eviction in the chrome-extension host path on backend
  side (the host binary itself is not shipped here; see Removed).

### Removed
- **`codex-chrome-extension-host` binary** is intentionally not shipped
  in this fork: it is a Chrome native messaging host scoped to Codex
  browser automation (`com.openai.codexextension`), unrelated to the
  computer-use MCP. The cosmic helper binary is renamed to
  `computer-use-linux-cosmic` to match the project naming.

## [0.1.0] - 2026-05-14

Wayland first; X11 best effort through AT-SPI and ydotool. Validated against GNOME 50.1 on Wayland (Ubuntu 25.10). KDE, Sway and Hyprland untested: see the README support matrix.

### Added
- Initial public release as a standalone repository, extracted from
  [`codex-desktop-linux-local-stack`](https://github.com/avifenesh/codex-desktop-linux).
- Linux Computer Use MCP server (`computer-use-linux` binary) speaking
  [rmcp](https://docs.rs/rmcp) over stdio.
- 15 MCP tools: `doctor`, `setup_accessibility`, `setup_window_targeting`,
  `list_apps`, `get_app_state`, `list_windows`, `focused_window`,
  `activate_window`, `click`, `drag`, `scroll`, `press_key`, `type_text`,
  `perform_action`, `set_value`.
- AT-SPI accessibility tree with semantic element selectors (role / name /
  text / states) for `click`, `perform_action`, and `set_value`.
- GNOME Shell window targeting via the bundled
  `computer-use-linux@avifenesh.dev` Shell extension (DBus service
  `dev.avifenesh.ComputerUseLinux.WindowControl`), with automatic fallback to
  `org.gnome.Shell.Introspect` when the extension is not installed.
- Screenshot capture through GNOME Shell DBus (preferred) and
  `org.freedesktop.portal.Screenshot` (fallback). Supports full-screen,
  per-app, per-window, region, and per-element scopes.
- Input synthesis through the Wayland remote-desktop portal when available,
  falling back to `ydotool` / `ydotoold` for keystrokes and pointer events.
- Best-effort terminal-window enrichment: maps each terminal window to its
  active TTY and foreground process for targeted `type_text` / `press_key`.
- `doctor` subcommand reporting AT-SPI bus health, GNOME Shell introspection
  status, extension status, ydotool socket readiness, and portal coverage in
  a single JSON document.

[Unreleased]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.52...HEAD
[0.7.52]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.51...v0.7.52
[0.7.51]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.50...v0.7.51
[0.7.50]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.49...v0.7.50
[0.7.49]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.48...v0.7.49
[0.7.48]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.47...v0.7.48
[0.7.47]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.46...v0.7.47
[0.7.46]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.45...v0.7.46
[0.7.45]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.44...v0.7.45
[0.7.44]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.43...v0.7.44
[0.7.43]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.42...v0.7.43
[0.7.42]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.41...v0.7.42
[0.7.41]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.40...v0.7.41
[0.7.40]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.39...v0.7.40
[0.7.39]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.38...v0.7.39
[0.7.38]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.37...v0.7.38
[0.7.37]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.36...v0.7.37
[0.7.36]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.35...v0.7.36
[0.7.35]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.34...v0.7.35
[0.7.34]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.33...v0.7.34
[0.7.33]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.32...v0.7.33
[0.7.32]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.31...v0.7.32
[0.7.31]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.30...v0.7.31
[0.7.30]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.29...v0.7.30
[0.7.29]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.28...v0.7.29
[0.7.28]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.27...v0.7.28
[0.7.27]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.26...v0.7.27
[0.7.26]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.25...v0.7.26
[0.7.25]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.24...v0.7.25
[0.7.24]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.23...v0.7.24
[0.7.23]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.22...v0.7.23
[0.7.22]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.21...v0.7.22
[0.7.21]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.20...v0.7.21
[0.7.20]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.19...v0.7.20
[0.7.19]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.18...v0.7.19
[0.7.18]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.17...v0.7.18
[0.7.17]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.16...v0.7.17
[0.7.16]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.15...v0.7.16
[0.7.15]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.14...v0.7.15
[0.7.14]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.13...v0.7.14
[0.7.13]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.12...v0.7.13
[0.7.12]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.11...v0.7.12
[0.7.11]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.10...v0.7.11
[0.7.10]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.9...v0.7.10
[0.7.9]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.8...v0.7.9
[0.7.8]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.7...v0.7.8
[0.7.7]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.6...v0.7.7
[0.7.6]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.5...v0.7.6
[0.7.5]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.4...v0.7.5
[0.7.4]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.3...v0.7.4
[0.7.3]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.2...v0.7.3
[0.7.2]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.1...v0.7.2
[0.7.1]: https://github.com/agent-sh/computer-use-linux/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/agent-sh/computer-use-linux/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/agent-sh/computer-use-linux/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/agent-sh/computer-use-linux/compare/v0.4.10...v0.5.0
[0.4.10]: https://github.com/agent-sh/computer-use-linux/compare/v0.4.9...v0.4.10
[0.4.9]: https://github.com/agent-sh/computer-use-linux/compare/v0.4.8...v0.4.9
[0.4.8]: https://github.com/agent-sh/computer-use-linux/compare/v0.4.7...v0.4.8
[0.4.7]: https://github.com/agent-sh/computer-use-linux/compare/v0.4.6...v0.4.7
[0.4.6]: https://github.com/agent-sh/computer-use-linux/compare/v0.4.5...v0.4.6
[0.4.5]: https://github.com/agent-sh/computer-use-linux/compare/v0.4.4...v0.4.5
[0.4.4]: https://github.com/agent-sh/computer-use-linux/compare/v0.4.3...v0.4.4
[0.4.3]: https://github.com/agent-sh/computer-use-linux/compare/v0.4.2...v0.4.3
[0.4.2]: https://github.com/agent-sh/computer-use-linux/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/agent-sh/computer-use-linux/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/agent-sh/computer-use-linux/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/agent-sh/computer-use-linux/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/agent-sh/computer-use-linux/compare/v0.2.9...v0.3.0
[0.2.9]: https://github.com/agent-sh/computer-use-linux/compare/v0.2.8...v0.2.9
[0.2.8]: https://github.com/agent-sh/computer-use-linux/compare/v0.2.7...v0.2.8
[0.2.7]: https://github.com/agent-sh/computer-use-linux/compare/v0.2.6...v0.2.7
[0.2.6]: https://github.com/agent-sh/computer-use-linux/compare/v0.2.5...v0.2.6
[0.2.5]: https://github.com/agent-sh/computer-use-linux/compare/v0.2.4...v0.2.5
[0.2.4]: https://github.com/agent-sh/computer-use-linux/compare/v0.2.3...v0.2.4
[0.2.3]: https://github.com/agent-sh/computer-use-linux/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/agent-sh/computer-use-linux/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/agent-sh/computer-use-linux/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/agent-sh/computer-use-linux/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/agent-sh/computer-use-linux/releases/tag/v0.1.0

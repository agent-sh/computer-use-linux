# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Exposed a focused Rust library surface for downstream diagnostics,
  accessibility snapshots, screenshots, and server integration while preserving
  the existing CLI binaries and standalone naming.

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

### Changed
- The COSMIC helper source path and runtime override surface now use standalone
  `computer-use-linux` naming only.

## [0.2.6] - 2026-06-06

### Fixed
- Screenshots now work from background processes (systemd user services,
  non-interactive parent shells) on GNOME Wayland. The GNOME Shell DBus method
  rejects callers that do not own an allowlisted bus name, and the XDG portal
  cancels non-interactive requests when there is no foreground window, so both
  prior backends failed in that context. `gnome-screenshot` is now a third
  capture fallback that works regardless of session context, bounded by a 20s
  timeout so a hung capture degrades to a clear error instead of blocking.

### Added
- `COMPUTER_USE_LINUX_SCREENSHOT_BACKEND` to force a single screenshot backend
  (`gnome-shell`, `portal`, or `gnome-screenshot`), skipping the fallback chain
  for pinned/background deployments and debugging.
- `doctor` now probes `gnome-screenshot` and lists it under
  `capabilities.screenshot` when present.

## [0.2.5] - 2026-06-05

### Added
- Added build-time GNOME extension / DBus identity overrides (`CUL_*`) so the
  `codex-desktop-linux` embedded copy can share this source while keeping its
  Codex extension identity, plus runtime `CODEX_COMPUTER_USE_*` aliases for the
  embedded input/backend knobs.

### Documentation
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

### Fixed
- Omitted the debug-only `received` echo field from the generated MCP
  `outputSchema` for `ActionOutput` and `ActivateWindowOutput`. `schemars`
  serialized `Option<serde_json::Value>` as the boolean schema `true`, which
  strict MCP clients (mcphub, Claude Desktop, `@modelcontextprotocol/sdk`'s
  `AssertObjectSchema`) reject, failing the whole `tools/list` response.
  Affected `click`, `drag`, `perform_action`, `press_key`, `scroll`,
  `set_value`, `type_text`, and `activate_window`. (#1)

### Changed
- Updated repository, release, package, and CI links from `avifenesh` to the
  `agent-sh/computer-use-linux` org repo.
- `setup_accessibility` and `doctor` now understand the AT-SPI
  `org.a11y.Status IsEnabled` path in addition to GNOME toolkit accessibility.

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

### Documentation
- Add Hermes Agent CLI setup commands and clarify the registered MCP tool
  names / toolset.
- Add npm install instructions and fix the prebuilt binary install example to
  match the release assets, which are raw binaries with `.sha256` files rather
  than tar archives.
- Document the mutating-tool safety contract for MCP hosts and npm users.
- Pin README and npm README install examples to the released `0.2.1` packages
  and `v0.2.1` GitHub release assets.

## [0.2.0] - 2026-05-14

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
- **Compact Linux accessibility trees** in `get_app_state` — deduplicated
  redundant container nodes for smaller, more focused snapshots.
- **Enriched AT-SPI state readback** — element states (`focused`,
  `selected`, `expanded`, `checked`, …) now flow through to the response
  schema for `get_app_state`.

### Changed
- Server-side rejection of empty window-backend results (returns a
  structured "no backend available" error instead of an empty list).
- Stale-client eviction in the chrome-extension host path on backend
  side (host binary itself not shipped here — see Removed).

### Removed
- **`codex-chrome-extension-host` binary** is intentionally not shipped
  in this fork — it is a Chrome native messaging host scoped to Codex
  browser automation (`com.openai.codexextension`), unrelated to the
  computer-use MCP. The cosmic helper binary is renamed to
  `computer-use-linux-cosmic` to match the project naming.

### Synced from upstream
- This release tracks
  [`ilysenko/codex-desktop-linux`](https://github.com/ilysenko/codex-desktop-linux)
  through commit `4d6fd96` (May 2026), then re-applies the rebrand
  (DBus names, env vars, GNOME extension UUID, cache-file prefixes).
  Upstream credit goes to ilysenko, mosesmrima, PinguuSS, and the
  original Codex contributors.

## [0.1.0] - 2026-05-13

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

### Architecture
- Wayland-first; X11 best-effort through AT-SPI + ydotool.
- Validated against GNOME 50.1 on Wayland (Ubuntu 25.10).
- KDE / Sway / Hyprland untested — see README support matrix.

[Unreleased]: https://github.com/agent-sh/computer-use-linux/compare/v0.2.8...HEAD
[0.2.8]: https://github.com/agent-sh/computer-use-linux/compare/v0.2.7...v0.2.8
[0.2.7]: https://github.com/agent-sh/computer-use-linux/compare/v0.2.6...v0.2.7
[0.2.6]: https://github.com/agent-sh/computer-use-linux/compare/v0.2.5...v0.2.6
[0.2.5]: https://github.com/agent-sh/computer-use-linux/compare/v0.2.4...v0.2.5
[0.2.4]: https://github.com/agent-sh/computer-use-linux/compare/v0.2.3...v0.2.4
[0.2.3]: https://github.com/agent-sh/computer-use-linux/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/agent-sh/computer-use-linux/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/agent-sh/computer-use-linux/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/agent-sh/computer-use-linux/releases/tag/v0.2.0
[0.1.0]: https://github.com/agent-sh/computer-use-linux/releases/tag/v0.1.0

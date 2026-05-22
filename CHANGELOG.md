# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/agent-sh/computer-use-linux/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/agent-sh/computer-use-linux/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/agent-sh/computer-use-linux/releases/tag/v0.2.0
[0.1.0]: https://github.com/agent-sh/computer-use-linux/releases/tag/v0.1.0

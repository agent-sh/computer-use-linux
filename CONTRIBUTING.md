# Contributing to computer-use-linux

## Development Setup

```bash
git clone https://github.com/agent-sh/computer-use-linux.git
cd computer-use-linux
cargo check --locked
cargo test --locked
```

Development and test builds keep file and line information for backtraces with
`debug = "line-tables-only"`. For a debugging session that needs local variables
and types, use `CARGO_PROFILE_DEV_DEBUG=2 cargo build --locked`. CI disables dev
debug information. Release settings are unchanged.

For npm wrapper work:

```bash
node --check npm/install.js
node --check npm/bin/computer-use-linux.js
npm pack --dry-run
```

## Before Opening a PR

Run the same gates CI runs:

```bash
cargo fmt --all -- --check
cargo check --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --no-fail-fast
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps --document-private-items
cargo publish --dry-run --locked
cargo audit --deny warnings
bash -n install.sh scripts/install_sh_test.sh
scripts/install_sh_test.sh
cargo build --locked
scripts/mcp_safety_check.py --binary target/debug/computer-use-linux
scripts/check_plugins.sh --binary target/debug/computer-use-linux
agnix .
node --check npm/install.js
node --check npm/bin/computer-use-linux.js
python3 scripts/indicator_packaging_test.py
npm run pack:check
npm ci --prefix scripts/zod-check --ignore-scripts --no-audit --no-fund
node scripts/zod-check/check.mjs --command target/debug/computer-use-linux
npm ci --prefix pi --ignore-scripts --no-audit --no-fund
npm run build:check --prefix pi
npm run typecheck --prefix pi
COMPUTER_USE_LINUX_TEST_BINARY="$PWD/target/debug/computer-use-linux" npm test --prefix pi
python3 scripts/generate_pi_tool_catalog.py --binary target/debug/computer-use-linux --check
```

`check_plugins.sh` without `--binary` skips the launcher download test. CI
also runs the X11 pointer regression below, installs the npm wrapper against
the local binaries, and smoke-tests the installed Pi package.

A script that a Rust test runs, or that such a script imports, must be listed
in `Cargo.toml` `include`.
`cargo publish --dry-run` builds the packaged crate but does not run its tests,
so CI does not catch a missing one.

The COSMIC protocol and portal regressions use private fixture peers and send no
input to the host desktop. The portal fixture needs PyGObject (`python3-gi`) and
`dbus-run-session`:

```bash
python3 scripts/cosmic_focus_test.py target/debug/computer-use-linux-cosmic
/usr/bin/python3 scripts/cosmic_input_test.py target/debug/computer-use-linux
```

The GNOME indicator regressions also use PyGObject and `dbus-run-session` and
run automatically with `cargo test`. They exercise the real MCP server and
indicator on a private D-Bus session with fixture input and screenshot commands.
To run them directly:

```bash
/usr/bin/python3 scripts/gnome_indicator_test.py target/debug/computer-use-linux target/debug/computer-use-linux-indicator
```

The real MCP/GTK X11 pointer regression runs in CI's MCP safety contract job,
which also gates release artifacts. To run it locally after building the binary:

```bash
/usr/bin/python3 scripts/x11_pointer_test.py target/debug/computer-use-linux
```

The X11 fixture requires Xvfb, xdotool, bubblewrap, strace, `dbus-run-session`
and GTK 3 PyGObject (`python3-gi` and `gir1.2-gtk-3.0` on Debian/Ubuntu).
It uses two private displays and a private D-Bus session, hides uinput, and
denies global-input helpers. It checks repeated clicks, GTK drag payloads,
release after failure, missing xdotool, and isolation from the other display.

The Chromium accessibility-tree regressions run Chrome on a private Xvfb display
and D-Bus session. They need Xvfb, openbox, xdotool, xprop, `dbus-run-session`,
the AT-SPI registry, and google-chrome or chromium. `app_state_size_test.py`
opens two windows of one browser, checks window scoping, the compact encoding
and `interactive_only`, and prints bytes per node; `CUL_338_MEASURE_ONLY=1`
prints the numbers for an older binary without the checks:

```bash
/usr/bin/python3 scripts/app_state_size_test.py target/debug/computer-use-linux
/usr/bin/python3 scripts/contenteditable_focus_test.py target/debug/computer-use-linux
```

A Unix socket path must fit in 107 bytes. Tests that bind or connect to
sockets take their directory from `src/test_sockets.rs` or
`scripts/socket_fixture.py`: the temp dir (or the base a script names) when
the longest socket path fits, else `XDG_RUNTIME_DIR`. When neither fits, the
test fails and names the limit and the path. A long `TMPDIR` moves the
fixtures instead of breaking them.

## PR Guidelines

- Keep changes focused and explain the desktop/session you tested on.
- Include `computer-use-linux doctor` output for compositor, portal, or accessibility issues.
- Preserve the MCP safety annotations when adding or changing tools.
- Update `README.md`, `npm/README.md`, and `skills/computer-use-linux/SKILL.md` when user-facing commands change.
- After editing `skills/computer-use-linux/`, copy it into `plugins/computer-use-linux/skills/`. `scripts/check_plugins.sh` fails while the copies differ.
- After changing a tool schema or description, rebuild and regenerate the Pi catalog: `python3 scripts/generate_pi_tool_catalog.py --binary target/debug/computer-use-linux`.
- Use conventional commit prefixes when practical (`fix:`, `feat:`, `docs:`, `chore:`).

## Releases

A release changes these ten files:

- `Cargo.toml` and `Cargo.lock`: the package version.
- `package.json`: the npm version.
- `src/server.rs`: the MCP server version.
- `plugins/computer-use-linux/bin/computer-use-linux`: the launcher's pinned version.
- `plugins/computer-use-linux/.claude-plugin/plugin.json` and `plugins/computer-use-linux/.codex-plugin/plugin.json`.
- `.claude-plugin/marketplace.json`.
- `pi/extension/generated-tools.ts`: rebuild, then regenerate it with `python3 scripts/generate_pi_tool_catalog.py --binary target/debug/computer-use-linux`.
- `CHANGELOG.md`: the `[Unreleased]` entries become the version's section, with its compare link.

`scripts/check_plugins.sh` fails when a plugin version differs from `Cargo.toml`, CI fails when `package.json` does, the MCP safety check fails when the server version does, and the Pi catalog `--check` fails until the catalog is regenerated. Push a `vX.Y.Z` tag to publish.

## Security

Do not open public issues for vulnerabilities. See [SECURITY.md](SECURITY.md).

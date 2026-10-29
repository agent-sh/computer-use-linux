# Contributing to computer-use-linux

Thanks for helping improve Linux desktop control for MCP hosts.

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
scripts/install_sh_test.sh
scripts/mcp_safety_check.py
scripts/check_plugins.sh
agnix .
```

If you changed release packaging, also run:

```bash
cargo publish --dry-run --locked
npm pack --dry-run
```

The COSMIC protocol and portal regressions use private fixture peers and send no
input to the host desktop. The portal fixture needs PyGObject (`python3-gi`) and
`dbus-run-session`:

```bash
python3 scripts/cosmic_focus_test.py "$CARGO_TARGET_DIR/debug/computer-use-linux-cosmic"
/usr/bin/python3 scripts/cosmic_input_test.py "$CARGO_TARGET_DIR/debug/computer-use-linux"
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

## PR Guidelines

- Keep changes focused and explain the desktop/session you tested on.
- Include `computer-use-linux doctor` output for compositor, portal, or accessibility issues.
- Preserve the MCP safety annotations when adding or changing tools.
- Update `README.md`, `npm/README.md`, and `skills/computer-use-linux/SKILL.md` when user-facing commands change.
- After editing `skills/computer-use-linux/`, copy it into `plugins/computer-use-linux/skills/`. A release bumps the version in `Cargo.toml`, `package.json`, the plugin launcher, both plugin manifests, and `.claude-plugin/marketplace.json`. `scripts/check_plugins.sh` enforces both.
- Use conventional commit prefixes when practical (`fix:`, `feat:`, `docs:`, `chore:`).

## Security

Do not open public issues for vulnerabilities. See [SECURITY.md](SECURITY.md).

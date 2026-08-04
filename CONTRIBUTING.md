# Contributing to computer-use-linux

Thanks for helping improve Linux desktop control for MCP hosts.

## Development Setup

```bash
git clone https://github.com/agent-sh/computer-use-linux.git
cd computer-use-linux
cargo check --locked
cargo test --locked
```

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
agnix .
```

If you changed release packaging, also run:

```bash
cargo publish --dry-run --locked
npm pack --dry-run
```

## PR Guidelines

- Keep changes focused and explain the desktop/session you tested on.
- Include `computer-use-linux doctor` output for compositor, portal, or accessibility issues.
- Preserve the MCP safety annotations when adding or changing tools.
- Update `README.md`, `npm/README.md`, and `skills/computer-use-linux/SKILL.md` when user-facing commands change.
- Use conventional commit prefixes when practical (`fix:`, `feat:`, `docs:`, `chore:`).

## Security

Do not open public issues for vulnerabilities. See [SECURITY.md](SECURITY.md).

#!/usr/bin/env python3
"""Fail when the server reads an environment variable a host does not pass.

MCP hosts start the server with a filtered environment. The native Pi
extension passes only FORWARDED_ENVIRONMENT in pi/extension/index.ts, plus
every LC_ variable. Codex passes its own defaults plus env_vars in
plugins/computer-use-linux/codex-mcp.json. A setting missing from either list
does nothing under that host.

Every SCREAMING_SNAKE string literal in the non-test Rust sources under src/
counts as a variable the server may read. Each one must be passed by both
hosts or be listed below with the reason it is not. Literals inside env! and
option_env! are read at build time and are skipped, as are #[cfg(test)]
items and modules. The Codex plugin launcher's variables must be in the Codex
list too, since Codex starts the launcher.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PI_EXTENSION = ROOT / "pi" / "extension" / "index.ts"
CODEX_MANIFEST = ROOT / "plugins" / "computer-use-linux" / "codex-mcp.json"
LAUNCHER = ROOT / "plugins" / "computer-use-linux" / "bin" / "computer-use-linux"

# Codex passes these to every stdio MCP server on Unix (DEFAULT_ENV_VARS in
# codex-rs/rmcp-client/src/utils.rs).
CODEX_DEFAULTS = {
    "HOME",
    "LOGNAME",
    "PATH",
    "SHELL",
    "USER",
    "__CF_USER_TEXT_ENCODING",
    "LANG",
    "LC_ALL",
    "TERM",
    "TMPDIR",
    "TZ",
}

# Literals in the server sources that name no environment variable.
NOT_ENVIRONMENT = {
    "ALT": "modifier name",
    "CTRL": "modifier name",
    "F10": "key name",
    "F11": "key name",
    "F12": "key name",
    "GET_FOCUSED_WINDOW": "labwc IPC request",
    "GET_WINDOWS": "labwc IPC request",
    "IHDR": "PNG chunk type",
    "PICTURES": "xdg-user-dir argument",
    "SHIFT": "modifier name",
    "SUPER": "modifier name",
}

# Variables the server reads that hosts are not expected to pass.
NOT_PASSED = {
    "GTK_MODULES": "describes the desktop's apps; the server reads it from the compositor process when the host does not pass it",
    "LC_": "prefix for run_shell's inherited locale; Codex passes LANG and LC_ALL, Pi passes every LC_ variable",
    "QT_LINUX_ACCESSIBILITY_ALWAYS_ON": "describes the desktop's apps; the server reads it from the compositor process when the host does not pass it",
    "WAYLAND_SOCKET": "a file descriptor a compositor hands to a process it starts; it does not survive an MCP host in between",
}

# Variables one host leaves out on purpose.
PI_NOT_PASSED = {
    "COMPUTER_USE_LINUX_NOTIFY_ON_COMPLETE": "adds complete_interaction to tools/list, and the Pi client refuses a catalog that differs from its generated one",
}
CODEX_NOT_PASSED: dict[str, str] = {}

NAME = re.compile(r"[A-Z][A-Z0-9_]{2,}")
TOKEN = re.compile(
    r"""
    (?P<comment>//[^\n]*|/\*.*?\*/)
    | (?P<raw>b?r(?P<hashes>\#*)"(?P<raw_body>.*?)"(?P=hashes))
    | (?P<string>b?"(?P<body>(?:\\.|[^"\\])*)")
    | (?P<char>b?'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]+\}|.)|[^'\\])')
    | (?P<lifetime>'[A-Za-z_][A-Za-z0-9_]*)
    | (?P<ident>[A-Za-z_][A-Za-z0-9_]*)
    | (?P<punct>[^\sA-Za-z0-9_])
    | (?P<other>\S)
    """,
    re.VERBOSE | re.DOTALL,
)


def tokens(text: str) -> list[tuple[str, str]]:
    """(kind, value) pairs; strings carry their contents, comments are dropped."""
    out = []
    for match in TOKEN.finditer(text):
        kind = match.lastgroup
        if kind in ("comment", "lifetime", "char"):
            continue
        if kind == "raw":
            out.append(("string", match.group("raw_body")))
        elif kind == "string":
            out.append(("string", match.group("body")))
        elif kind == "hashes":
            continue
        else:
            out.append((kind, match.group(kind)))
    return out


def is_cfg_test(toks: list[tuple[str, str]], index: int) -> int:
    """Length of a `#[cfg(test)]` attribute starting at index, or 0."""
    want = [("punct", "#"), ("punct", "["), ("ident", "cfg"), ("punct", "("),
            ("ident", "test"), ("punct", ")"), ("punct", "]")]
    return len(want) if toks[index:index + len(want)] == want else 0


def skip_attributes(toks: list[tuple[str, str]], index: int) -> int:
    while index < len(toks) and toks[index] == ("punct", "#"):
        depth = 0
        index += 1
        while index < len(toks):
            if toks[index] == ("punct", "["):
                depth += 1
            elif toks[index] == ("punct", "]"):
                depth -= 1
                if depth == 0:
                    index += 1
                    break
            index += 1
    return index


def scan(path: Path) -> tuple[set[str], list[str]]:
    """Names read at run time, and test-only module names declared here."""
    toks = tokens(path.read_text())
    names: set[str] = set()
    test_modules: list[str] = []
    index = 0
    while index < len(toks):
        length = is_cfg_test(toks, index)
        if length:
            index = skip_attributes(toks, index + length)
            if (index + 2 < len(toks) and toks[index] == ("ident", "mod")
                    and toks[index + 2] == ("punct", ";")):
                test_modules.append(toks[index + 1][1])
            depth = 0
            while index < len(toks):
                kind, value = toks[index]
                index += 1
                if (kind, value) == ("punct", "{"):
                    depth += 1
                elif (kind, value) == ("punct", "}"):
                    depth -= 1
                    if depth == 0:
                        break
                elif (kind, value) == ("punct", ";") and depth == 0:
                    break
            continue
        kind, value = toks[index]
        if (kind == "ident" and value in ("env", "option_env")
                and toks[index + 1:index + 3] == [("punct", "!"), ("punct", "(")]):
            index += 4
            continue
        if kind == "string" and NAME.fullmatch(value):
            names.add(value)
        index += 1
    return names, test_modules


def module_file(parent: Path, name: str) -> list[Path]:
    base = parent.parent if parent.name in ("lib.rs", "main.rs", "mod.rs") else parent.with_suffix("")
    return [base / f"{name}.rs", base / name / "mod.rs"]


def server_names() -> dict[str, list[str]]:
    sources = sorted((ROOT / "src").rglob("*.rs"))
    skipped: set[Path] = set()
    found: dict[str, list[str]] = {}
    scans = {}
    for path in sources:
        scans[path] = scan(path)
        for module in scans[path][1]:
            skipped.update(module_file(path, module))
    for path in sources:
        if path in skipped:
            continue
        for name in scans[path][0]:
            found.setdefault(name, []).append(str(path.relative_to(ROOT)))
    return found


def pi_names() -> set[str]:
    text = PI_EXTENSION.read_text()
    match = re.search(r"const FORWARDED_ENVIRONMENT = \[(.*?)\]", text, re.DOTALL)
    if not match:
        sys.exit(f"check_env_forwarding: no FORWARDED_ENVIRONMENT list in {PI_EXTENSION.relative_to(ROOT)}")
    names = set(re.findall(r'"([^"]+)"', match.group(1)))
    if 'startsWith("LC_")' in text:
        names.add("LC_")
    return names


def codex_names() -> set[str]:
    manifest = json.loads(CODEX_MANIFEST.read_text())
    return set(manifest["mcpServers"]["computer-use-linux"]["env_vars"]) | CODEX_DEFAULTS


def launcher_names() -> set[str]:
    return set(re.findall(r"\$\{?([A-Z_][A-Z0-9_]*)", LAUNCHER.read_text()))


def main() -> int:
    found = server_names()
    pi = pi_names()
    codex = codex_names()
    errors = []

    for table_name, table in (("NOT_ENVIRONMENT", NOT_ENVIRONMENT), ("NOT_PASSED", NOT_PASSED),
                              ("PI_NOT_PASSED", PI_NOT_PASSED), ("CODEX_NOT_PASSED", CODEX_NOT_PASSED)):
        for name in sorted(set(table) - set(found)):
            errors.append(f"{table_name} lists {name}, which the server no longer reads; remove it")

    for name in sorted(PI_NOT_PASSED.keys() & pi):
        errors.append(f"PI_NOT_PASSED lists {name}, but FORWARDED_ENVIRONMENT passes it")
    for name in sorted(CODEX_NOT_PASSED.keys() & codex):
        errors.append(f"CODEX_NOT_PASSED lists {name}, but codex-mcp.json passes it")

    required = {name for name in found if name not in NOT_ENVIRONMENT and name not in NOT_PASSED}
    for host, passed, excluded, where in (
        ("Pi", pi, PI_NOT_PASSED, "FORWARDED_ENVIRONMENT in pi/extension/index.ts"),
        ("Codex", codex, CODEX_NOT_PASSED, "env_vars in plugins/computer-use-linux/codex-mcp.json"),
    ):
        for name in sorted(required - passed - excluded.keys()):
            errors.append(f"{host} does not pass {name} (read in {', '.join(sorted(set(found[name])))}); add it to {where}")

    for name in sorted(launcher_names() - codex):
        errors.append(f"Codex does not pass {name}, which the plugin launcher reads; add it to env_vars in plugins/computer-use-linux/codex-mcp.json")

    if errors:
        for error in errors:
            print(f"check_env_forwarding: {error}", file=sys.stderr)
        print(
            "check_env_forwarding: a variable a host should not pass goes in NOT_PASSED, "
            "PI_NOT_PASSED or CODEX_NOT_PASSED with its reason; a literal that is not a "
            "variable goes in NOT_ENVIRONMENT",
            file=sys.stderr,
        )
        return 1
    print(f"env forwarding ok: Pi and Codex pass the {len(required)} variables the server reads")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

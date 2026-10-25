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
list too, since Codex starts the launcher. Codex must also pass everything Pi
passes, so the tools the server runs see the same environment under both.

The check sees names spelled out in this crate only. Variables read inside
std (TMPDIR through env::temp_dir), dependencies, or the tools the server
runs (gsettings, ydotool) are out of its reach.
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
    "COMPUTER_USE_LINUX_MCP_TOKEN": "read only by `mcp --listen`, which an MCP host never starts; a host spawns the server over stdio",
    "CUL_INPUT_LOCK_HOLD_MS": "debug-build test hook for tests/mcp_http_listen.rs; release builds never read it",
    "GTK_MODULES": "describes the desktop's apps; the server reads it from the compositor process when the host does not pass it",
    "QT_LINUX_ACCESSIBILITY_ALWAYS_ON": "describes the desktop's apps; the server reads it from the compositor process when the host does not pass it",
    "WAYLAND_SOCKET": "a file descriptor a compositor hands to a process it starts; it does not survive an MCP host in between",
}

# Variables one host leaves out on purpose.
PI_NOT_PASSED = {
    "COMPUTER_USE_LINUX_NOTIFY_ON_COMPLETE": "adds complete_interaction to tools/list, and the Pi client refuses a catalog that differs from its generated one",
}
CODEX_NOT_PASSED = {
    "LC_": "prefix for run_shell's inherited locale; Codex passes LANG and LC_ALL only",
}

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


# What a #[cfg(test)] attribute covers, after any `pub` or `pub(...)`:
# - a declaration (let, const, static, use, type) runs to its semicolon;
# - another item (fn, mod, struct, impl, ...) runs to its closing brace or
#   semicolon;
# - anything else (a match arm, field, variant or expression) also ends at a
#   comma at its own level, and at a closing brace unless the code goes on
#   (`=>`, `.`, `?`, `else`, `as`).
# All of them end where the enclosing block closes.
DECLARATION_KEYWORDS = {"const", "let", "static", "type", "use"}
ITEM_KEYWORDS = {
    "async", "enum", "extern", "fn", "impl", "macro_rules", "mod", "struct",
    "trait", "unsafe",
}
OPENERS = {"{", "(", "["}
CLOSERS = {"}", ")", "]"}
CONTINUES_AFTER_BRACE = {("punct", "."), ("punct", "?"), ("ident", "else"), ("ident", "as")}


def skip_visibility(toks: list[tuple[str, str]], index: int) -> int:
    if index < len(toks) and toks[index] == ("ident", "pub"):
        index += 1
        if index < len(toks) and toks[index] == ("punct", "("):
            depth = 0
            while index < len(toks):
                if toks[index] == ("punct", "("):
                    depth += 1
                elif toks[index] == ("punct", ")"):
                    depth -= 1
                    if depth == 0:
                        return index + 1
                index += 1
    return index


def target_kind(toks: list[tuple[str, str]], index: int) -> str:
    kind, value = toks[index] if index < len(toks) else ("", "")
    if kind != "ident":
        return "other"
    if value in DECLARATION_KEYWORDS:
        return "declaration"
    if value in ITEM_KEYWORDS:
        return "item"
    # `union` is a keyword only before a name: `union U { ... }`.
    if value == "union" and index + 1 < len(toks) and toks[index + 1][0] == "ident":
        return "item"
    return "other"


def skip_cfg_test_target(toks: list[tuple[str, str]], index: int) -> int:
    """Index just past the code a #[cfg(test)] attribute at index applies to."""
    kind_of_target = target_kind(toks, skip_visibility(toks, index))
    depth = 0
    while index < len(toks):
        kind, value = toks[index]
        if kind == "punct":
            if value in OPENERS:
                depth += 1
            elif value in CLOSERS:
                if depth == 0:
                    return index
                depth -= 1
                if depth == 0 and value == "}" and kind_of_target != "declaration":
                    following = toks[index + 1:index + 3]
                    goes_on = following[:1] and following[0] in CONTINUES_AFTER_BRACE
                    arrow = following == [("punct", "="), ("punct", ">")]
                    if kind_of_target == "item" or not (goes_on or arrow):
                        return index + 1
            elif value == ";" and depth == 0:
                return index + 1
            elif value == "," and depth == 0 and kind_of_target == "other":
                return index + 1
        index += 1
    return index


def scan(text: str) -> tuple[set[str], list[str]]:
    """Names read at run time, and test-only module names declared here."""
    toks = tokens(text)
    names: set[str] = set()
    test_modules: list[str] = []
    index = 0
    while index < len(toks):
        length = is_cfg_test(toks, index)
        if length:
            index = skip_attributes(toks, index + length)
            target = skip_visibility(toks, index)
            if (target + 2 < len(toks) and toks[target] == ("ident", "mod")
                    and toks[target + 2] == ("punct", ";")):
                test_modules.append(toks[target + 1][1])
            index = skip_cfg_test_target(toks, index)
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


SELF_TEST = r"""
//! "DOC_COMMENT_NAME" is skipped.
const VERSION: &str = env!("BUILD_TIME_NAME");
fn route(kind: Kind) -> Option<&'static str> {
    match kind {
        #[cfg(test)]
        Kind::Fake(a, b) => Some("TEST_TUPLE_ARM_NAME"),
        Kind::Real => std::env::var("AFTER_ARM_NAME").ok().map(|_| "x"),
    }
}
fn struct_arm(event: Event) -> Option<&'static str> {
    match event {
        #[cfg(test)]
        Event::Fake { name } => Some("TEST_STRUCT_ARM_NAME"),
        Event::Real => std::env::var("AFTER_STRUCT_ARM_NAME").ok().map(|_| "x"),
    }
}
fn block_arm(kind: Kind) -> Option<u8> {
    match kind {
        #[cfg(test)]
        Kind::Fake => { let _ = "TEST_ARM_NAME"; None }
        Kind::Real => std::env::var("AFTER_BLOCK_ARM_NAME").ok().map(|_| 1),
    }
}
struct Config {
    #[cfg(test)]
    fake: Vec<(u8, u8)>,
    real: Option<String>,
}
fn field() -> Option<String> { std::env::var("AFTER_FIELD_NAME").ok() }
struct Last {
    real: u8,
    #[cfg(test)]
    last: bool
}
struct Public {
    #[cfg(test)]
    pub(crate) fake: bool,
    #[arg(env = "AFTER_PUB_FIELD_NAME")]
    pub real: u8,
}
fn last_field() -> Option<String> { std::env::var("AFTER_LAST_FIELD_NAME").ok() }
fn statement() -> Option<String> {
    #[cfg(test)]
    let _ = ("TEST_STATEMENT_NAME", 1);
    #[cfg(test)]
    let _ = if ready { "TEST_LET_IF_NAME" } else { "TEST_LET_ELSE_NAME" };
    #[cfg(test)]
    use std::{env, fs};
    let raw = r#"RAW_STRING_NAME"#;
    let _ = '"';
    std::env::var("AFTER_STATEMENT_NAME").ok()
}
#[cfg(test)]
fn helper<A, B>(a: A, b: B) -> &'static str where A: Copy, B: Copy { "TEST_FN_NAME" }
#[cfg(test)]
mod fixtures;
#[cfg(test)]
pub(crate) mod shared_fixtures;
#[cfg(test)]
mod tests {
    #[test]
    fn reads() { let _ = std::env::var("TEST_MODULE_NAME"); }
}
fn last() -> Option<String> { std::env::var("LAST_NAME").ok() }
"""


def self_test() -> None:
    names, modules = scan(SELF_TEST)
    expected = {
        "AFTER_ARM_NAME", "AFTER_STRUCT_ARM_NAME", "AFTER_BLOCK_ARM_NAME", "AFTER_FIELD_NAME",
        "AFTER_LAST_FIELD_NAME", "AFTER_PUB_FIELD_NAME", "AFTER_STATEMENT_NAME",
        "RAW_STRING_NAME", "LAST_NAME",
    }
    expected_modules = ["fixtures", "shared_fixtures"]
    if names != expected or modules != expected_modules:
        sys.exit(
            f"check_env_forwarding: scanner self-test failed: found {sorted(names)} and "
            f"test modules {modules}, expected {sorted(expected)} and {expected_modules}"
        )


def module_file(parent: Path, name: str) -> list[Path]:
    base = parent.parent if parent.name in ("lib.rs", "main.rs", "mod.rs") else parent.with_suffix("")
    return [base / f"{name}.rs", base / name / "mod.rs"]


def server_names() -> dict[str, list[str]]:
    sources = sorted((ROOT / "src").rglob("*.rs"))
    skipped: set[Path] = set()
    found: dict[str, list[str]] = {}
    scans = {}
    for path in sources:
        scans[path] = scan(path.read_text())
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
    match = re.search(r"const FORWARDED_ENVIRONMENT = \[(.*?)\]\s*as const", text, re.DOTALL)
    if not match:
        sys.exit(f"check_env_forwarding: no FORWARDED_ENVIRONMENT list in {PI_EXTENSION.relative_to(ROOT)}")
    body = re.sub(r"//[^\n]*|/\*.*?\*/", "", match.group(1), flags=re.DOTALL)
    names = set(re.findall(r'"([^"]+)"', body))
    if 'entry[0].startsWith("LC_")' in text:
        names.add("LC_")
    return names


def codex_names() -> set[str]:
    manifest = json.loads(CODEX_MANIFEST.read_text())
    return set(manifest["mcpServers"]["computer-use-linux"]["env_vars"]) | CODEX_DEFAULTS


def launcher_names() -> set[str]:
    return set(re.findall(r"\$\{?([A-Z_][A-Z0-9_]*)", LAUNCHER.read_text()))


def main() -> int:
    self_test()
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

    # The tools the server runs (gsettings, gdbus, ydotool, wtype) see the same
    # environment under both hosts, so Codex passes everything Pi passes.
    for name in sorted(pi - codex - CODEX_NOT_PASSED.keys()):
        errors.append(f"Codex does not pass {name}, which Pi passes; add it to env_vars in plugins/computer-use-linux/codex-mcp.json")

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
    print(
        f"env forwarding ok: the server reads {len(required)} variables; "
        "Pi and Codex pass each one or list why not"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

# Hyprland / Electron literal-input reproduction

This manual test exercises an isolated Electron textarea using synthetic text.
It temporarily takes keyboard focus. Keep the test window focused while input
runs. It never sends a prompt to Codex or edits an existing document. Use a
separate Electron user-data directory, as below.

## Environment and source snapshot

- computer-use-linux base: `cae6aab` (`v0.7.16`)
- Hyprland: `0.56.2-2`, commit `efb50993780079460b0cbed1363e2166a2de1d9f`
- Session: Wayland, Turkish keyboard layout
- wtype: `0.4-2`; ydotool: `1.0.4-2`
- xdg-desktop-portal-hyprland: `1.4.1-2`
- Isolated target: Electron `43.6.0`, Chromium `150.0.7871.250`
- Observed on: 2026-10-15

These are results for the isolated test target. The original report concerned
Codex inside ChatGPT Desktop; that live application was not changed or used to
submit test content.

## Run

Replace `electron43` with an installed Electron executable. The script requires
Hyprland's Lua focus dispatcher and checks the focused window's PID before
injecting text. It exits nonzero if any tested route fails or changes the text.
The stdin comparison is expected to fail on an affected session.

```sh
CUL_QA_OUTPUT=/tmp/cul-wtype-comparison.json \
  electron43 --user-data-dir=/tmp/cul-input-qa-profile \
  --ozone-platform=wayland --force-renderer-accessibility \
  scripts/repro_wtype_electron.cjs

cargo build --locked --bin computer-use-linux
CUL_QA_OUTPUT=/tmp/cul-patched-mcp.json \
  CUL_QA_MODES=mcp \
  CUL_QA_BINARY="$PWD/target/debug/computer-use-linux" \
  electron43 --user-data-dir=/tmp/cul-input-qa-profile \
  --ozone-platform=wayland --force-renderer-accessibility \
  scripts/repro_wtype_electron.cjs
```

The MCP test initializes the server, calls scoped `get_app_state` without a
screenshot, then calls `type_text` with the test-window PID. Output records the
literal expected/actual strings, input events, Electron/Chromium versions, and
the MCP response. All recorded text is the fixed synthetic payload in the script.

## Observed comparison

The same 3,000-character payload was used throughout. Its first 125 characters
have a limited alphabet, followed by Turkish text, uppercase letters, digits,
quotes, parentheses, `/`, `-`, and `;`. In one counterbalanced session, argv
ran first, followed by two identical stdin trials:

Payload UTF-8 SHA-256:
`7ff8d279121d07e8e49f8e00109ca5b013798856583f4f11502f657ca287b0ea`.

| Route | Characters received | Exact match | First differing character | Events with Alt/Ctrl set |
| --- | ---: | --- | ---: | ---: |
| `wtype -- <text>` | 3,000 | yes | none | 0 |
| `wtype -`, stdin trial 1 | 1,295 | no | 125 | 38 |
| `wtype -`, stdin trial 2 | 1,295 | no | 125 | 38 |
| patched MCP `type_text`, separate session | 3,000 | yes | none | 0 |

All wtype processes exited with code zero, including the corrupted trials.
The repeated stdin trials demonstrate repeatability for this payload; they are
not independent coverage of different inputs or applications. An earlier
startup trial also lost text, but had a different failure position and is not
used for the counterbalanced comparison.

## Interpretation and proposed change

In [wtype's source](https://github.com/atx/wtype/blob/master/main.c),
`run_text_stdin` refreshes the virtual-keyboard keymap at each 100-character
batch. The argv path builds the complete keymap before the first key event.
Changing only that path avoided corruption in this experiment. This narrows
the failure to stdin/keymap-refresh handling in the tested stack; it does not
establish which compositor or Chromium implementation is responsible. The
event trace shows Control events; a permanently stuck Alt key was not proven.

The patch uses one argument after `--`, preserving option-looking text literally.
It rejects NUL and more than 131,071 UTF-8 bytes before starting wtype. Missing
wtype retains the existing ydotool fallback; an available wtype's validation,
spawn, timeout, or execution error does not replay partially sent text.

Tradeoff: unlike stdin, text in argv is inspectable in the child process's
command line while it runs. This also applies to the existing xdotool backend.
This patch is a local contribution candidate, not an installed replacement.

The command-routing regression test fails on the old stdin implementation and
passes after the change. The Electron test supplies the separate evidence that
the literal text reached a real rendered field; a command-routing test alone
cannot establish that.

## Other findings from the original report

- Implemented: an AT-SPI empty-focus result becomes informational when exact
  window focus was verified before input. It explicitly leaves field contents
  unverified; concrete non-editable focus warnings remain.
- Implemented: a complete, scoped tree containing only application/window
  containers gets a conditional Electron/Chromium accessibility hint. This is
  not positive Electron detection, and does not alter application launch flags.
- Still open: optional field-content verification. `ok: true` retains its
  dispatch meaning; the wtype result now states that contents were not verified.
- Still open: clipboard paste mode with complete MIME/ownership restoration.
  A text/plain-only backup would not satisfy the reported requirement.
- Still open: a pointer-motion/hover tool, including its backend dispatch,
  coordinate mapping, MCP annotations, and client catalogs.
- Still open: unfocused-window screenshots. Existing tests deliberately require
  focus; a separate design must distinguish a window image from a desktop-region
  crop that may contain an occluding window or an inactive workspace.

The regression, limits, scoped-tree hint, and focus-feedback cases have unit
coverage. The renderer hint and focus-feedback changes have not been validated
inside the user's running ChatGPT Desktop session.

## Validation

- `cargo fmt --all -- --check`
- `cargo check --locked --all-targets`
- `cargo clippy --locked --all-targets -- -D warnings`
- `cargo test --locked --no-fail-fast --quiet`: 428 library tests and all other
  target/integration tests passed with an isolated `XDG_CACHE_HOME`.
- `scripts/install_sh_test.sh`
- `scripts/mcp_safety_check.py --binary target/debug/computer-use-linux`
- `scripts/check_plugins.sh --binary target/debug/computer-use-linux`
- `npx --yes agnix .`
- `node --check scripts/repro_wtype_electron.cjs`
- The checked-in reproduction script's MCP mode received all 3,000 characters
  exactly, with no Alt/Ctrl-marked events.

The first full test run inside the restricted sandbox failed on local socket
and private D-Bus creation. The unchanged tests passed outside that restriction;
none were skipped or weakened. This is local validation, not upstream CI or
validation of an installed replacement.

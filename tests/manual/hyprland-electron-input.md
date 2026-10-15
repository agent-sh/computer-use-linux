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

The original `1850c7a` patch used one argument after `--`, preserving option-looking text literally.
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

- Hardened after review: an AT-SPI empty-focus result becomes informational only
  when exact window focus was verified and a complete scoped search saw only
  window/menu chrome. Trees with content retain missing-focus warnings.
- Implemented: a complete, scoped tree containing only window/menu
  chrome gets a conditional Electron/Chromium accessibility hint. This is
  not positive Electron detection, and does not alter application launch flags.
- Still open: optional field-content verification. `ok: true` retains its
  dispatch meaning; every successful text backend now states that contents were not verified.
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

## Review hardening after `1850c7a`

- Wtype NUL/size validation runs at the tool entry, before window focus or the
  text indicator, and remains at the command boundary as defense in depth.
- Only Hyprland uses argument mode. Other compatible compositors retain stdin;
  this experiment does not establish a need to change those compositors.
- Confirmed password fields are rejected before argument-mode dispatch.
  Unknown fields use stdin for at most 100 Unicode characters (one wtype batch),
  rather than publishing text in argv or reintroducing mid-text keymap refreshes.
  Larger unknown-field input must use smaller calls or expose accessibility.
  The input probe has a separate 1.5-second budget, even with the indicator off;
  the cosmetic indicator retains its independent 250 ms probe.
  This is best-effort AT-SPI classification, not protection against a target
  changing its focused field after the probe. Terminal password prompts may not
  expose a password role through AT-SPI.
- The shell-only observation hint is computed on the raw scoped snapshot before
  compaction can discard unnamed menu ancestors and reparent their buttons.
- A complete scoped shell-only focus result is separate from a content tree
  with no focused element. Exact compositor focus alone no longer softens the
  latter's warning. Failed/incomplete reads cannot establish a shell-only tree.
- Window chrome includes native menu roles, allowing the conditional renderer
  hint when Electron exposes a menu bar but no renderer content. This is a tree
  coverage heuristic, not positive Electron detection or a claim about GTK/Qt.
- Every successful text route includes the same dispatch-only qualification.
  MCP initialization instructions and both bundled skills describe the policy.
- The fake-wtype test is named for argument delivery rather than keymap behavior;
  exact `--`, `-`, and empty text are covered alongside long Unicode input.

Regression fixtures exercise early rejection with a deliberately invalid window
id, secret/unknown rejection before process/fallback launch, and content-tree
warnings even with exact window focus. A dedicated subprocess controls backend
environment variables without mutating the parent test environment.

The native script also accepts `CUL_QA_MODES=mcp,mcp-password`. Its normal-field
case expects all 3,000 characters; its synthetic password case expects explicit
rejection, an empty field, and zero keyboard events. The server's indicator is
intentionally disabled for both cases so a display setting cannot disable the
secrecy guard. This mode still requires hands-off focus and a separate Electron
profile; do not run it against an existing document or prompt.

### Native hardening result (2026-10-15)

With Electron 43.6.0 / Chromium 150.0.7871.250 and the indicator disabled:

| MCP case | Result | Field contents | Keyboard events | Alt/Ctrl events |
| --- | --- | --- | ---: | ---: |
| Normal textarea | success | exact 3,000/3,000 characters | 6,000 | 0 |
| Synthetic password input | explicit rejection | empty | 0 | 0 |

The first hardening trial rejected the normal field too. Read-only inspection
found a focused `entry` with AT-SPI's `editable` state but no EditableText
interface. The secrecy policy now accepts the editable state as well as the
interface, while checking password roles first. A regression fixture preserves
that observed interface/state combination. Focus feedback uses the same editable
classification so it does not mislabel this entry after successful dispatch.
The failed native trial and its inspection were retained separately from the
passing trial; they are not acceptance evidence.

The three primary review regressions were also run with their old behavior
restored temporarily: each failed. Restoring the fix passed the complete Rust
suite (434 library tests, plus binary/integration targets); the subprocess-only
preflight fixture is ignored in direct discovery and exercised by its parent.
Formatting, all-target checking/Clippy, MCP safety and plugin packaging checks
passed. The final feedback classification assertion was checked separately.
These are local results, not upstream CI. No installed executable was replaced.

## PR #256 review follow-up

The first review identified that the 250 ms cosmetic probe had become a hard
input gate, and that unknown fields (including terminals without AT-SPI) were
always rejected. Input routing now has a dedicated 1,500 ms budget: a confirmed
nonsecret text target uses argv; a confirmed password role is rejected; a missing,
incomplete, failed or timed-out probe uses stdin for up to 100 Unicode characters.
That bound is wtype 0.4's single stdin batch, not a claim that arbitrary long stdin
input is fixed. No automatic partial-input retry or multi-process chunking was
added. Existing non-Hyprland stdin behavior is unchanged.

The renderer hint now uses raw snapshot ancestry before display compaction. The
regression fixture passes an unnamed panel/menu-bar chain through the production
compaction-and-note helper, confirms those ancestors disappear from the returned
tree, and checks that the previously computed hint survives. Content, unscoped
and truncated cases remain excluded.

In the native follow-up, an isolated Foot window with no accessible nodes received
100 synthetic Unicode characters exactly through stdin while the indicator was
disabled. A 101-character call was explicitly rejected and the PTY received no
additional input. The receiver ran in raw mode, so no shell command was executed.
The first fixture attempt stopped before input because it incorrectly equated
`tree_scoped=true` with a nonempty accessibility tree; the actual tree was empty.
That aborted attempt was not counted as acceptance evidence.

Three new regression cases fail with the reviewed behavior restored: a 300 ms
successful input probe must outlive the overlay budget, unknown fields must have
the bounded stdin route, and menu ancestry must be evaluated before compaction.
There is also explicit coverage for the 100/101 Unicode boundary, confirmed
password rejection and the input-probe timeout. Native GTK/Qt focus-loss and
renderer-disabled menu-only Electron were not separately exercised.

### Second review: ambiguous roles are not confirmed passwords

The overlay deliberately masks terminal, enum-fallback `Entry`/`Text`, generic
`text box` and unreadable roles as well as actual password roles. The first
routing follow-up mistakenly reused that masking predicate as a password test,
so accessible terminals were rejected. A focused-terminal regression reproduced
that failure before this correction.

Input classification now has explicit password/nonsecret/ambiguous outcomes.
Only a role containing `password` (case-insensitive) confirms a password for
rejection. Masked roles and the native terminal flag use bounded stdin, even if
the editable state/interface is present; they never qualify for argv. The overlay
continues masking them. Normal nonterminal `entry`/`text` fields with the editable
state retain the long-text argument route.

The role-matrix regression exercises terminal metadata (including an inconsistent
`entry` name with the native terminal flag), every ambiguous role family, both
EditableText-interface values, 100-character stdin delivery and 101-character
rejection. Separate cases preserve password alias rejection and the 3,000-character
nonsecret-field route. This is injected AT-SPI metadata and fake-process coverage;
no native VTE/Konsole session was exercised. The earlier Foot result validates
the same unchanged stdin transport, not a terminal exposing AT-SPI.

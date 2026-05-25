#!/usr/bin/env bash
# install.sh — bootstrap the computer-use-linux MCP server on a Linux box.
#
# This script takes a fresh checkout from `git clone` to a working
# `computer-use-linux mcp` binary in PATH plus all the system-side
# prerequisites (AT-SPI, ydotoold, optional GNOME Shell extension).
#
# Each step is idempotent and individually skippable via flags.
# Re-running the script on a fully provisioned host should print all-green
# and exit 0 without changing anything.

set -euo pipefail
IFS=$'\n\t'

# -----------------------------------------------------------------------------
# Globals & helpers
# -----------------------------------------------------------------------------

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &>/dev/null && pwd)"
BIN_NAME="computer-use-linux"
COSMIC_HELPER_NAME="computer-use-linux-cosmic"
INSTALL_DIR="${HOME}/.local/bin"
INSTALL_PATH="${INSTALL_DIR}/${BIN_NAME}"
COSMIC_HELPER_INSTALL_PATH="${INSTALL_DIR}/${COSMIC_HELPER_NAME}"
EXT_UUID="computer-use-linux@avifenesh.dev"
EXT_SRC_DIR="${SCRIPT_DIR}/gnome-shell-extension/${EXT_UUID}"

# Color helpers (degrade gracefully when not on a tty).
if [[ -t 1 ]] && command -v tput >/dev/null 2>&1 && [[ "$(tput colors 2>/dev/null || echo 0)" -ge 8 ]]; then
    C_GREEN="$(tput setaf 2)"
    C_YELLOW="$(tput setaf 3)"
    C_RED="$(tput setaf 1)"
    C_BLUE="$(tput setaf 4)"
    C_BOLD="$(tput bold)"
    C_RESET="$(tput sgr0)"
else
    C_GREEN=""; C_YELLOW=""; C_RED=""; C_BLUE=""; C_BOLD=""; C_RESET=""
fi

log_section() { printf '\n%s==>%s %s%s%s\n' "${C_BLUE}" "${C_RESET}" "${C_BOLD}" "$*" "${C_RESET}"; }
log_ok()      { printf '  %sOK%s   %s\n' "${C_GREEN}" "${C_RESET}" "$*"; }
log_warn()    { printf '  %sWARN%s %s\n' "${C_YELLOW}" "${C_RESET}" "$*"; }
log_skip()    { printf '  %sSKIP%s %s\n' "${C_YELLOW}" "${C_RESET}" "$*"; }
log_fail()    { printf '  %sFAIL%s %s\n' "${C_RED}" "${C_RESET}" "$*"; }
log_info()    { printf '       %s\n' "$*"; }

die() { log_fail "$*"; exit 1; }

# Track failed checks for a final summary.
FAILED_CHECKS=()
record_failure() { FAILED_CHECKS+=("$1"); }

trap 'rc=$?; if [[ $rc -ne 0 ]]; then printf "\n%sinstall.sh aborted (exit %d)%s\n" "${C_RED}" "$rc" "${C_RESET}"; fi' EXIT

# -----------------------------------------------------------------------------
# CLI parsing
# -----------------------------------------------------------------------------

SKIP_SYSTEM_DEPS=0
SKIP_RUST=0
SKIP_BUILD=0
SKIP_ATSPI=0
SKIP_YDOTOOL=0
SKIP_GNOME_EXT=0
SKIP_DOCTOR=0
FORCE_UNKNOWN_DISTRO=0

usage() {
    cat <<EOF
${C_BOLD}install.sh${C_RESET} — provision computer-use-linux on this machine.

Usage: ./install.sh [flags]

Steps (run in order, each idempotent):
  1. Detect distro + display server
  2. Install system packages (apt/dnf/pacman)
  3. Install rustup toolchain
  4. cargo build --release  →  ~/.local/bin/${BIN_NAME} and ${COSMIC_HELPER_NAME}
  5. Enable AT-SPI toolkit accessibility (GNOME)
  6. Install + enable ydotoold systemd --user service
  7. Pack/install/enable GNOME Shell extension (Wayland + GNOME)
  8. Run \`${BIN_NAME} doctor\` and report readiness

Flags:
  --skip-system-deps      skip apt/dnf/pacman package install
  --skip-rust             skip rustup install
  --skip-build            skip cargo build (assumes target/release/${BIN_NAME} and ${COSMIC_HELPER_NAME} exist)
  --skip-atspi            skip toolkit-accessibility gsetting
  --skip-ydotool          skip ydotoold systemd unit
  --skip-gnome-extension  skip GNOME Shell extension install
  --skip-doctor           skip the final readiness check
  --force-unknown-distro  treat unrecognised distros as Debian-family (apt)
  -h, --help              show this help and exit
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --skip-system-deps)     SKIP_SYSTEM_DEPS=1 ;;
        --skip-rust)            SKIP_RUST=1 ;;
        --skip-build)           SKIP_BUILD=1 ;;
        --skip-atspi)           SKIP_ATSPI=1 ;;
        --skip-ydotool)         SKIP_YDOTOOL=1 ;;
        --skip-gnome-extension) SKIP_GNOME_EXT=1 ;;
        --skip-doctor)          SKIP_DOCTOR=1 ;;
        --force-unknown-distro) FORCE_UNKNOWN_DISTRO=1 ;;
        -h|--help)              usage; exit 0 ;;
        *)                      usage; die "unknown flag: $1" ;;
    esac
    shift
done

# -----------------------------------------------------------------------------
# Step 1: distro + display server detection
# -----------------------------------------------------------------------------

DISTRO_FAMILY=""
PKG_MANAGER=""

detect_distro() {
    log_section "Step 1/9 — detect environment"

    if [[ "$(uname -s)" != "Linux" ]]; then
        die "this script only supports Linux (got $(uname -s)). macOS/*BSD are not supported."
    fi

    if [[ ! -r /etc/os-release ]]; then
        die "/etc/os-release missing — cannot detect distro."
    fi

    # shellcheck disable=SC1091
    . /etc/os-release
    local id_like="${ID_LIKE:-} ${ID:-}"

    case " ${id_like} " in
        *" debian "*|*" ubuntu "*)
            DISTRO_FAMILY="debian"; PKG_MANAGER="apt" ;;
        *" fedora "*|*" rhel "*|*" centos "*)
            DISTRO_FAMILY="fedora"; PKG_MANAGER="dnf" ;;
        *" arch "*|*" archlinux "*|*" manjaro "*|*" endeavouros "*)
            DISTRO_FAMILY="arch"; PKG_MANAGER="pacman" ;;
        *)
            if [[ ${FORCE_UNKNOWN_DISTRO} -eq 1 ]]; then
                log_warn "unknown distro '${ID:-?}' — forcing debian/apt path"
                DISTRO_FAMILY="debian"; PKG_MANAGER="apt"
            else
                log_fail "unsupported distro: ${ID:-unknown} (${PRETTY_NAME:-?})"
                log_info "supported families: debian/ubuntu, fedora, arch"
                log_info "re-run with --force-unknown-distro to attempt apt-based install"
                exit 1
            fi ;;
    esac
    log_ok "distro family: ${DISTRO_FAMILY} (pkg manager: ${PKG_MANAGER})"

    # Display server.
    local session_type=""
    if [[ -n "${XDG_SESSION_ID:-}" ]] && command -v loginctl >/dev/null 2>&1; then
        session_type="$(loginctl show-session "${XDG_SESSION_ID}" -p Type --value 2>/dev/null || true)"
    fi
    session_type="${session_type:-${XDG_SESSION_TYPE:-unknown}}"

    case "${session_type}" in
        wayland) log_ok "display server: Wayland" ;;
        x11)     log_warn "display server: X11 — supported but degraded (some features need Wayland)" ;;
        *)       log_warn "display server: ${session_type} (unrecognised — proceeding anyway)" ;;
    esac

    local desktop="${XDG_CURRENT_DESKTOP:-unknown}"
    case "${desktop}" in
        *GNOME*)         log_ok "compositor: GNOME (${desktop})" ;;
        *KDE*|*Plasma*)  log_warn "compositor: KDE (${desktop}) — untested, AT-SPI step will be skipped" ;;
        *sway*)          log_warn "compositor: sway — untested" ;;
        *Hyprland*)      log_warn "compositor: hyprland — untested" ;;
        *)               log_warn "compositor: ${desktop} — untested" ;;
    esac
}

# -----------------------------------------------------------------------------
# Step 2: system package install
# -----------------------------------------------------------------------------

install_system_deps() {
    log_section "Step 2/9 — system packages"
    if [[ ${SKIP_SYSTEM_DEPS} -eq 1 ]]; then log_skip "--skip-system-deps"; return 0; fi

    local desktop="${XDG_CURRENT_DESKTOP:-}"
    case "${PKG_MANAGER}" in
        apt)
            local pkgs=(build-essential pkg-config libdbus-1-dev libssl-dev curl ydotool at-spi2-core)
            if [[ "${desktop}" == *GNOME* ]]; then pkgs+=(gnome-shell-extension-tool); fi
            log_info "sudo apt-get install -y ${pkgs[*]}"
            sudo apt-get update -qq
            sudo apt-get install -y "${pkgs[@]}" || { log_fail "apt-get install failed"; return 1; }
            ;;
        dnf)
            local pkgs=(gcc pkgconfig dbus-devel openssl-devel curl ydotool at-spi2-core)
            log_info "sudo dnf install -y ${pkgs[*]}"
            sudo dnf install -y "${pkgs[@]}" || { log_fail "dnf install failed"; return 1; }
            ;;
        pacman)
            local pkgs=(base-devel pkgconf dbus openssl curl ydotool at-spi2-core)
            log_info "sudo pacman -S --needed --noconfirm ${pkgs[*]}"
            sudo pacman -S --needed --noconfirm "${pkgs[@]}" || { log_fail "pacman install failed"; return 1; }
            ;;
    esac
    log_ok "system packages installed"
}

# -----------------------------------------------------------------------------
# Step 3: rustup toolchain
# -----------------------------------------------------------------------------

install_rust() {
    log_section "Step 3/9 — Rust toolchain"
    if [[ ${SKIP_RUST} -eq 1 ]]; then log_skip "--skip-rust"; return 0; fi

    if command -v cargo >/dev/null 2>&1; then
        log_ok "cargo already on PATH ($(cargo --version))"
        return 0
    fi
    if [[ -x "${HOME}/.cargo/bin/cargo" ]]; then
        log_ok "cargo at ~/.cargo/bin (sourcing into PATH)"
        export PATH="${HOME}/.cargo/bin:${PATH}"
        return 0
    fi

    log_info "installing rustup (stable, minimal profile)"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs |
        sh -s -- -y --default-toolchain stable --profile minimal --no-modify-path
    export PATH="${HOME}/.cargo/bin:${PATH}"
    command -v cargo >/dev/null 2>&1 || { log_fail "rustup install did not produce cargo"; return 1; }
    log_ok "rustup installed ($(cargo --version))"
}

# -----------------------------------------------------------------------------
# Step 4: cargo build + install binary into ~/.local/bin
# -----------------------------------------------------------------------------

build_and_install() {
    log_section "Step 4/9 — build & install binary"
    local built="${SCRIPT_DIR}/target/release/${BIN_NAME}"
    local cosmic_helper_built="${SCRIPT_DIR}/target/release/${COSMIC_HELPER_NAME}"

    if [[ ${SKIP_BUILD} -eq 1 ]]; then
        log_skip "--skip-build (expecting prebuilt binaries at ${built} and ${cosmic_helper_built})"
    else
        ( cd "${SCRIPT_DIR}" && cargo build --release ) || { log_fail "cargo build failed"; return 1; }
        log_ok "cargo build --release succeeded"
    fi

    [[ -x "${built}" ]] || { log_fail "binary not found at ${built}"; return 1; }
    [[ -x "${cosmic_helper_built}" ]] || { log_fail "COSMIC helper not found at ${cosmic_helper_built}"; return 1; }

    mkdir -p "${INSTALL_DIR}"
    if [[ -f "${INSTALL_PATH}" ]] && cmp -s "${built}" "${INSTALL_PATH}"; then
        log_ok "binary already up to date at ${INSTALL_PATH}"
    else
        install -m 0755 "${built}" "${INSTALL_PATH}"
        log_ok "installed ${INSTALL_PATH}"
    fi
    if [[ -f "${COSMIC_HELPER_INSTALL_PATH}" ]] && cmp -s "${cosmic_helper_built}" "${COSMIC_HELPER_INSTALL_PATH}"; then
        log_ok "COSMIC helper already up to date at ${COSMIC_HELPER_INSTALL_PATH}"
    else
        install -m 0755 "${cosmic_helper_built}" "${COSMIC_HELPER_INSTALL_PATH}"
        log_ok "installed ${COSMIC_HELPER_INSTALL_PATH}"
    fi

    case ":${PATH}:" in
        *":${INSTALL_DIR}:"*) ;;
        *) log_warn "${INSTALL_DIR} is not in your PATH — add it to your shell rc:"
           log_info "  export PATH=\"\$HOME/.local/bin:\$PATH\"" ;;
    esac
}

# -----------------------------------------------------------------------------
# Step 5: AT-SPI toolkit accessibility (GNOME-only)
# -----------------------------------------------------------------------------

enable_atspi() {
    log_section "Step 5/9 — AT-SPI toolkit accessibility"
    if [[ ${SKIP_ATSPI} -eq 1 ]]; then log_skip "--skip-atspi"; return 0; fi

    if [[ "${XDG_CURRENT_DESKTOP:-}" != *GNOME* ]]; then
        log_warn "non-GNOME desktop — skipping (set toolkit-accessibility manually if needed)"
        return 0
    fi
    if ! command -v gsettings >/dev/null 2>&1; then
        log_warn "gsettings not available — skipping"
        return 0
    fi

    local current
    current="$(gsettings get org.gnome.desktop.interface toolkit-accessibility 2>/dev/null || echo 'false')"
    if [[ "${current}" == "true" ]]; then
        log_ok "toolkit-accessibility already enabled"
    else
        gsettings set org.gnome.desktop.interface toolkit-accessibility true
        log_ok "toolkit-accessibility enabled"
    fi
}

# -----------------------------------------------------------------------------
# Step 6: ydotoold systemd --user service
# -----------------------------------------------------------------------------

setup_ydotoold() {
    log_section "Step 6/9 — ydotoold user service"
    if [[ ${SKIP_YDOTOOL} -eq 1 ]]; then log_skip "--skip-ydotool"; return 0; fi

    command -v ydotoold >/dev/null 2>&1 || { log_fail "ydotoold not found in PATH (install via system deps step)"; return 1; }

    # /dev/uinput permissions check.
    if [[ ! -e /dev/uinput ]]; then
        log_warn "/dev/uinput does not exist — kernel module may need loading"
        log_info "  sudo modprobe uinput"
    elif [[ ! -w /dev/uinput || ! -r /dev/uinput ]]; then
        log_warn "/dev/uinput exists but is not user-accessible"
        log_info "Remediation (pick one, then log out/in):"
        log_info "  sudo usermod -aG input \$USER"
        log_info "OR write a udev rule:"
        log_info "  echo 'KERNEL==\"uinput\", MODE=\"0660\", GROUP=\"input\", OPTIONS+=\"static_node=uinput\"' | sudo tee /etc/udev/rules.d/60-uinput.rules"
        log_info "  sudo udevadm control --reload-rules && sudo udevadm trigger"
        log_warn "skipping systemd enable — fix uinput first then re-run"
        return 0
    fi

    local unit_dir="${HOME}/.config/systemd/user"
    local unit_file="${unit_dir}/ydotoold.service"
    mkdir -p "${unit_dir}"
    cat > "${unit_file}" <<'EOF'
[Unit]
Description=ydotool user daemon
Documentation=man:ydotool(1) man:ydotoold(8)

[Service]
Type=simple
ExecStart=/usr/bin/ydotoold --socket-path=%t/.ydotool_socket --socket-own=%U:%U
Restart=on-failure

[Install]
WantedBy=default.target
EOF
    log_ok "wrote ${unit_file}"

    systemctl --user daemon-reload
    systemctl --user enable --now ydotoold.service || {
        log_fail "systemctl --user enable --now ydotoold failed"
        log_info "check: systemctl --user status ydotoold"
        return 1
    }

    # Verify socket.
    local sock="${XDG_RUNTIME_DIR:-/run/user/$UID}/.ydotool_socket"
    local tries=0
    while [[ ! -S "${sock}" && ${tries} -lt 10 ]]; do sleep 0.3; ((tries++)); done

    if [[ -S "${sock}" ]]; then
        local owner mode
        owner="$(stat -c '%u' "${sock}")"
        mode="$(stat -c '%a' "${sock}")"
        if [[ "${owner}" == "${UID}" && "${mode}" == "600" ]]; then
            log_ok "ydotoold socket ready (${sock}, mode ${mode})"
        else
            log_warn "socket exists but owner=${owner} mode=${mode} (expected ${UID}/600)"
        fi
    else
        log_warn "socket ${sock} did not appear within ~3s — check the unit"
    fi
}

# -----------------------------------------------------------------------------
# Step 7: GNOME Shell extension (Wayland + GNOME only)
# -----------------------------------------------------------------------------

install_gnome_extension() {
    log_section "Step 7/9 — GNOME Shell extension"
    if [[ ${SKIP_GNOME_EXT} -eq 1 ]]; then log_skip "--skip-gnome-extension"; return 0; fi

    if [[ "${XDG_CURRENT_DESKTOP:-}" != *GNOME* ]]; then
        log_skip "non-GNOME desktop"
        return 0
    fi
    local session_type="${XDG_SESSION_TYPE:-}"
    if [[ -z "${session_type}" ]] && command -v loginctl >/dev/null 2>&1 && [[ -n "${XDG_SESSION_ID:-}" ]]; then
        session_type="$(loginctl show-session "${XDG_SESSION_ID}" -p Type --value 2>/dev/null || true)"
    fi
    if [[ "${session_type}" != "wayland" ]]; then
        log_skip "not a Wayland session (extension only needed under GNOME Wayland)"
        return 0
    fi
    if ! command -v gnome-extensions >/dev/null 2>&1; then
        log_warn "gnome-extensions CLI missing — install gnome-shell-extension-tool"
        return 0
    fi
    if [[ ! -d "${EXT_SRC_DIR}" ]]; then
        log_warn "extension source directory not found at ${EXT_SRC_DIR} — skipping"
        return 0
    fi

    # Already installed & enabled? Nothing to do.
    if gnome-extensions list --enabled 2>/dev/null | grep -qx "${EXT_UUID}"; then
        log_ok "extension ${EXT_UUID} already enabled"
        return 0
    fi

    local pack_dir
    pack_dir="$(mktemp -d)"
    ( cd "${pack_dir}" && gnome-extensions pack "${EXT_SRC_DIR}" ) ||
        { log_fail "gnome-extensions pack failed"; rm -rf "${pack_dir}"; return 1; }

    local zipfile
    zipfile="$(find "${pack_dir}" -maxdepth 1 -name '*.shell-extension.zip' | head -n1)"
    [[ -f "${zipfile}" ]] || { log_fail "packed zip not produced"; rm -rf "${pack_dir}"; return 1; }

    gnome-extensions install --force "${zipfile}" || { log_fail "gnome-extensions install failed"; rm -rf "${pack_dir}"; return 1; }
    log_ok "extension installed (${zipfile##*/})"
    rm -rf "${pack_dir}"

    if gnome-extensions enable "${EXT_UUID}" 2>/dev/null; then
        log_ok "extension enabled"
    else
        log_warn "could not enable ${EXT_UUID} yet — GNOME Shell may not have rescanned"
        log_info "log out and back in, then run:"
        log_info "  gnome-extensions enable ${EXT_UUID}"
    fi
}

# -----------------------------------------------------------------------------
# Step 8: doctor readiness check
# -----------------------------------------------------------------------------

run_doctor() {
    log_section "Step 8/9 — doctor readiness"
    if [[ ${SKIP_DOCTOR} -eq 1 ]]; then log_skip "--skip-doctor"; return 0; fi

    [[ -x "${INSTALL_PATH}" ]] || { log_fail "${INSTALL_PATH} missing — cannot run doctor"; return 1; }

    local out
    if ! out="$("${INSTALL_PATH}" doctor 2>&1)"; then
        log_fail "doctor invocation failed"
        printf '%s\n' "${out}"
        return 1
    fi

    if command -v jq >/dev/null 2>&1 && printf '%s' "${out}" | jq -e . >/dev/null 2>&1; then
        local blockers
        blockers="$(printf '%s' "${out}" | jq -r '.readiness.blockers | if type == "array" then length else -1 end')"
        printf '%s\n' "${out}" | jq -r '
            .readiness as $r |
            "ready: \($r.blockers | type == "array" and length == 0)\n" +
            ((($r.blockers // []) | map("  - \(.)") | join("\n")))
        '
        if [[ "${blockers}" -eq 0 ]]; then
            log_ok "doctor reports ready"
        elif [[ "${blockers}" -eq -1 ]]; then
            log_fail "doctor output missing blockers field — unexpected JSON structure"
            return 1
        else
            log_fail "doctor reports NOT ready"
            while IFS= read -r line; do
                FAILED_CHECKS+=("${line}")
            done < <(printf '%s' "${out}" | jq -r '.readiness.blockers[]?')
            return 1
        fi
    else
        # Raw fallback.
        printf '%s\n' "${out}"
        if printf '%s' "${out}" | grep -qiE '"blockers"[[:space:]]*:[[:space:]]*\[\]'; then
            log_ok "doctor reports ready (raw)"
        else
            log_fail "doctor did not report ready (install jq for a structured summary)"
            return 1
        fi
    fi
}

# -----------------------------------------------------------------------------
# Driver
# -----------------------------------------------------------------------------

main() {
    printf '%scomputer-use-linux installer%s — repo: %s\n' "${C_BOLD}" "${C_RESET}" "${SCRIPT_DIR}"

    detect_distro
    install_system_deps  || record_failure "system deps"
    install_rust         || record_failure "rust toolchain"
    build_and_install    || record_failure "build/install"
    enable_atspi         || record_failure "atspi"
    setup_ydotoold       || record_failure "ydotoold"
    install_gnome_extension || record_failure "gnome extension"
    run_doctor           || record_failure "doctor"

    log_section "Step 9/9 — summary"
    if [[ ${#FAILED_CHECKS[@]} -eq 0 ]]; then
        log_ok "all steps completed successfully"
        exit 0
    else
        log_fail "completed with failures:"
        for f in "${FAILED_CHECKS[@]}"; do log_info "  - ${f}"; done
        exit 1
    fi
}

main "$@"

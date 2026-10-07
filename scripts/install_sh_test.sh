#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
INSTALLER="${ROOT_DIR}/install.sh"
FIXTURE_DIR="${ROOT_DIR}/tests/fixtures"

assert_eq() {
    local actual="$1" expected="$2"
    if [[ "${actual}" != "${expected}" ]]; then
        printf 'expected %q, got %q\n' "${expected}" "${actual}" >&2
        return 1
    fi
}

assert_contains() {
    local actual="$1" expected="$2"
    if [[ "${actual}" != *"${expected}"* ]]; then
        printf 'expected output to contain %q\n%s\n' "${expected}" "${actual}" >&2
        return 1
    fi
}

assert_not_contains() {
    local actual="$1" unexpected="$2"
    if [[ "${actual}" == *"${unexpected}"* ]]; then
        printf 'expected output not to contain %q\n%s\n' "${unexpected}" "${actual}" >&2
        return 1
    fi
}

create_fake_doctor() {
    local path="$1"
    printf '%s\n' \
        '#!/usr/bin/env bash' \
        'printf '\''%s\n'\'' "${COMPUTER_USE_LINUX_TEST_DOCTOR_OUTPUT}"' \
        >"${path}"
    chmod +x "${path}"
}

run_fake_doctor() {
    local doctor_output="$1" hide_jq="${2:-0}" fake_dir status
    fake_dir="$(mktemp -d)"
    INSTALL_PATH="${fake_dir}/computer-use-linux"
    create_fake_doctor "${INSTALL_PATH}"
    export COMPUTER_USE_LINUX_TEST_DOCTOR_OUTPUT="${doctor_output}"
    if [[ "${hide_jq}" -eq 1 ]]; then
        command() {
            if [[ "$1" == "-v" && "$2" == "jq" ]]; then return 1; fi
            builtin command "$@"
        }
    fi
    status=0
    run_doctor || status=$?
    rm -rf -- "${fake_dir}"
    return "${status}"
}

test_artix_selects_pacman() (
    export COMPUTER_USE_LINUX_OS_RELEASE_FILE="${FIXTURE_DIR}/os-release.artix"
    export XDG_SESSION_TYPE=x11
    export XDG_CURRENT_DESKTOP=unknown

    # shellcheck source=../install.sh
    source "${INSTALLER}"
    package_manager_available() { [[ "$1" == "pacman" ]]; }

    detect_distro >/dev/null || return 1
    assert_eq "${PKG_MANAGER}" "pacman" || return 1
    assert_eq "${DISTRO_FAMILY}" "arch" || return 1
)

test_unknown_distro_selects_only_available_manager() (
    export COMPUTER_USE_LINUX_OS_RELEASE_FILE="${FIXTURE_DIR}/os-release.unknown"
    export XDG_SESSION_TYPE=x11
    export XDG_CURRENT_DESKTOP=unknown

    # shellcheck source=../install.sh
    source "${INSTALLER}"
    FORCE_UNKNOWN_DISTRO=1
    package_manager_available() { [[ "$1" == "dnf" ]]; }

    detect_distro >/dev/null || return 1
    assert_eq "${PKG_MANAGER}" "dnf" || return 1
    assert_eq "${DISTRO_FAMILY}" "fedora" || return 1
)

test_skip_system_deps_needs_no_package_manager() (
    export COMPUTER_USE_LINUX_OS_RELEASE_FILE="${FIXTURE_DIR}/os-release.unknown"
    export XDG_SESSION_TYPE=x11
    export XDG_CURRENT_DESKTOP=unknown

    # shellcheck source=../install.sh
    source "${INSTALLER}"
    FORCE_UNKNOWN_DISTRO=1
    SKIP_SYSTEM_DEPS=1
    package_manager_available() { return 1; }

    detect_distro >/dev/null || return 1
    assert_eq "${PKG_MANAGER}" "" || return 1
    assert_eq "${DISTRO_FAMILY}" "unknown" || return 1
)

test_skip_system_deps_allows_missing_override() (
    export COMPUTER_USE_LINUX_OS_RELEASE_FILE="${FIXTURE_DIR}/os-release.unknown"
    export XDG_SESSION_TYPE=x11
    export XDG_CURRENT_DESKTOP=unknown

    # shellcheck source=../install.sh
    source "${INSTALLER}"
    PACKAGE_MANAGER_OVERRIDE=apt
    SKIP_SYSTEM_DEPS=1
    package_manager_available() { return 1; }

    detect_distro >/dev/null || return 1
    assert_eq "${PKG_MANAGER}" "apt" || return 1
    assert_eq "${DISTRO_FAMILY}" "debian" || return 1
)

test_startx_system_deps_include_xdotool() (
    export COMPUTER_USE_LINUX_OS_RELEASE_FILE="${FIXTURE_DIR}/os-release.artix"
    export XDG_SESSION_TYPE=tty
    export DISPLAY=:0
    unset WAYLAND_DISPLAY XDG_SESSION_ID
    export XDG_CURRENT_DESKTOP=unknown

    # shellcheck source=../install.sh
    source "${INSTALLER}"
    package_manager_available() { [[ "$1" == "pacman" ]]; }
    sudo() { printf 'sudo %s\n' "$*"; }
    install_optional_ydotool() { :; }

    local output
    detect_distro >/dev/null || return 1
    output="$(install_system_deps)" || return 1
    assert_eq "${X11_KEYBOARD_BACKEND_REQUIRED}" "1" || return 1
    assert_contains "${output}" "pacman -S --needed --noconfirm" || return 1
    assert_contains "${output}" "xdotool" || return 1
)

test_wayland_system_deps_exclude_xdotool() (
    export COMPUTER_USE_LINUX_OS_RELEASE_FILE="${FIXTURE_DIR}/os-release.artix"
    export XDG_SESSION_TYPE=wayland
    export WAYLAND_DISPLAY=wayland-0
    export DISPLAY=:0
    unset XDG_SESSION_ID
    export XDG_CURRENT_DESKTOP=unknown

    # shellcheck source=../install.sh
    source "${INSTALLER}"
    package_manager_available() { [[ "$1" == "pacman" ]]; }
    sudo() { printf 'sudo %s\n' "$*"; }
    install_optional_ydotool() { :; }

    local output
    detect_distro >/dev/null || return 1
    output="$(install_system_deps)" || return 1
    assert_eq "${X11_KEYBOARD_BACKEND_REQUIRED}" "0" || return 1
    assert_not_contains "${output}" "xdotool" || return 1
)

test_non_systemd_host_gets_manual_guidance() (
    export XDG_RUNTIME_DIR="/run/user/test"
    local uinput
    uinput="$(mktemp)"
    trap 'rm -f "${uinput}"' EXIT
    export COMPUTER_USE_LINUX_UINPUT_DEVICE="${uinput}"

    # shellcheck source=../install.sh
    source "${INSTALLER}"
    systemd_user_manager_available() { return 1; }
    ydotoold() { :; }

    local output
    output="$(setup_ydotoold)" || return 1
    assert_contains "${output}" "systemd --user is unavailable" || return 1
    assert_contains "${output}" "configure your per-user supervisor to run" || return 1
    assert_contains "${output}" "ydotoold --socket-path=/run/user/test/.ydotool_socket" || return 1
    assert_contains "${output}" "--socket-perm=0600" || return 1
    assert_contains "${output}" "do not run ydotoold as root" || return 1
)

test_non_systemd_host_requires_uinput_access() (
    local uinput
    uinput="$(mktemp)"
    chmod 000 "${uinput}"
    trap 'chmod 600 "${uinput}"; rm -f "${uinput}"' EXIT
    export COMPUTER_USE_LINUX_UINPUT_DEVICE="${uinput}"

    # shellcheck source=../install.sh
    source "${INSTALLER}"
    systemd_user_manager_available() { return 1; }
    ydotoold() { :; }

    local output
    output="$(setup_ydotoold)" || return 1
    assert_contains "${output}" "exists but is not user-accessible" || return 1
    assert_not_contains "${output}" "configure your per-user supervisor to run" || return 1
)

test_systemd_ydotoold_unit_forces_private_socket() (
    local scratch uinput unit_content systemctl_calls
    scratch="$(mktemp -d)"
    uinput="${scratch}/uinput"
    export HOME="${scratch}/home"
    export XDG_RUNTIME_DIR="${scratch}/runtime"
    export COMPUTER_USE_LINUX_UINPUT_DEVICE="${uinput}"
    mkdir -p "${HOME}" "${XDG_RUNTIME_DIR}"
    : >"${uinput}"

    # shellcheck source=../install.sh
    source "${INSTALLER}"
    trap 'rm -rf -- "${scratch}"' EXIT
    systemd_user_manager_available() { return 0; }
    ydotoold() { :; }
    systemctl() { printf '%s ' "$@" >>"${scratch}/systemctl.calls"; printf '\n' >>"${scratch}/systemctl.calls"; }
    verify_ydotoold_socket() { :; }

    setup_ydotoold >/dev/null || return 1
    unit_content="$(<"${HOME}/.config/systemd/user/ydotoold.service")"
    systemctl_calls="$(<"${scratch}/systemctl.calls")"
    assert_contains "${unit_content}" "UMask=0077" || return 1
    assert_contains "${unit_content}" "--socket-perm=0600" || return 1
    assert_contains "${systemctl_calls}" "daemon-reload" || return 1
    assert_contains "${systemctl_calls}" "enable --now ydotoold.service" || return 1
)

test_ydotoold_rejects_unsafe_socket_metadata() (
    local scratch owner mode owner_and_mode output status systemctl_calls
    scratch="$(mktemp -d)"

    # shellcheck source=../install.sh
    source "${INSTALLER}"
    trap 'rm -rf -- "${scratch}"' EXIT
    systemctl() { printf '%s ' "$@" >>"${scratch}/systemctl.calls"; printf '\n' >>"${scratch}/systemctl.calls"; }

    for owner_and_mode in "${UID} 660" "$((UID + 1)) 600"; do
        IFS=' ' read -r owner mode <<<"${owner_and_mode}"
        : >"${scratch}/systemctl.calls"
        status=0
        output="$(validate_ydotoold_socket_security "/run/user/${UID}/.ydotool_socket" "${owner}" "${mode}")" || status=$?
        systemctl_calls="$(<"${scratch}/systemctl.calls")"

        assert_eq "${status}" "1" || return 1
        assert_contains "${output}" "refusing unsafe ydotoold socket" || return 1
        assert_contains "${systemctl_calls}" "disable --now ydotoold.service" || return 1
    done
)

test_ydotoold_rejects_symlink_socket_path() (
    local scratch output status systemctl_calls
    scratch="$(mktemp -d)"
    : >"${scratch}/target"
    ln -s "${scratch}/target" "${scratch}/socket"

    # shellcheck source=../install.sh
    source "${INSTALLER}"
    trap 'rm -rf -- "${scratch}"' EXIT
    systemctl() { printf '%s ' "$@" >>"${scratch}/systemctl.calls"; printf '\n' >>"${scratch}/systemctl.calls"; }

    status=0
    output="$(verify_ydotoold_socket "${scratch}/socket")" || status=$?
    systemctl_calls="$(<"${scratch}/systemctl.calls")"

    assert_eq "${status}" "1" || return 1
    assert_contains "${output}" "refusing symlink at ydotoold socket path" || return 1
    assert_contains "${systemctl_calls}" "disable --now ydotoold.service" || return 1
)

test_doctor_accepts_platform_capability_blockers() (
    # shellcheck source=../install.sh
    source "${INSTALLER}"
    local status output doctor_output
    doctor_output='{"readiness":{"can_register_mcp_tools":true,"can_build_accessibility_tree":true,"can_query_windows":false,"can_focus_apps":false,"can_focus_windows":false,"can_send_development_input":true,"recommended_next_step":"Use global input without targeted window focus.","blockers":["Window introspection is unavailable; targeted window focus and verification will be disabled."]}}'

    status=0
    output="$(run_fake_doctor "${doctor_output}")" || status=$?

    assert_eq "${status}" "0" || return 1
    assert_contains "${output}" "installation succeeded; 1 platform capability is unavailable" || return 1
    assert_contains "${output}" "Window introspection is unavailable" || return 1
    assert_not_contains "${output}" "doctor reports NOT ready" || return 1
)

test_doctor_rejects_missing_install_prerequisite() (
    # shellcheck source=../install.sh
    source "${INSTALLER}"
    local status output doctor_output
    doctor_output='{"readiness":{"can_register_mcp_tools":true,"can_build_accessibility_tree":true,"can_query_windows":false,"can_focus_apps":false,"can_focus_windows":false,"can_send_development_input":false,"recommended_next_step":"Start a keyboard-capable input backend.","blockers":["Window introspection is unavailable.","Development keyboard input is unavailable."]}}'

    status=0
    output="$(run_fake_doctor "${doctor_output}")" || status=$?

    assert_eq "${status}" "1" || return 1
    assert_contains "${output}" "doctor reports NOT ready" || return 1
    assert_contains "${output}" "Development keyboard input is unavailable" || return 1
)

test_doctor_raw_fallback_accepts_platform_capability_blockers() (
    # shellcheck source=../install.sh
    source "${INSTALLER}"
    local status output doctor_output
    doctor_output='{"readiness":{"can_register_mcp_tools":true,"can_build_accessibility_tree":true,"can_query_windows":false,"can_focus_apps":false,"can_focus_windows":false,"can_send_development_input":true,"recommended_next_step":"Use global input without targeted window focus.","blockers":["Window introspection is unavailable."]}}'

    status=0
    output="$(run_fake_doctor "${doctor_output}" 1)" || status=$?

    assert_eq "${status}" "0" || return 1
    assert_contains "${output}" "installation succeeded; platform capabilities are unavailable" || return 1
    assert_not_contains "${output}" "doctor did not report ready" || return 1
)

test_doctor_raw_fallback_rejects_missing_install_prerequisite() (
    # shellcheck source=../install.sh
    source "${INSTALLER}"
    local status output doctor_output
    doctor_output='{"readiness":{"can_register_mcp_tools":true,"can_build_accessibility_tree":true,"can_query_windows":false,"can_focus_apps":false,"can_focus_windows":false,"can_send_development_input":false,"recommended_next_step":"Start a keyboard-capable input backend.","blockers":["Development keyboard input is unavailable."]}}'

    status=0
    output="$(run_fake_doctor "${doctor_output}" 1)" || status=$?

    assert_eq "${status}" "1" || return 1
    assert_contains "${output}" "doctor did not report ready" || return 1
)

test_enabled_gnome_extension_is_upgraded() (
    # Exercise the upgrade path without touching the live GNOME session.
    export XDG_CURRENT_DESKTOP=GNOME XDG_SESSION_TYPE=wayland
    export XDG_DATA_HOME="$(mktemp -d "${XDG_CACHE_HOME:-$HOME/.cache}/cul-gnome-upgrade.XXXXXX")"
    trap 'rm -rf -- "${XDG_DATA_HOME}"' EXIT
    source "${INSTALLER}"
    local installed="${XDG_DATA_HOME}/gnome-shell/extensions/${EXT_UUID}"
    mkdir -p "${installed}"
    printf '%s\n' '// old extension' >"${installed}/extension.js"
    printf '%s\n' '{"version":2}' >"${installed}/metadata.json"
    gnome-extensions() {
        case "$1" in
            list) printf '%s\n' "${EXT_UUID}" ;;
            pack) : >"bridge.shell-extension.zip" ;;
            install)
                cp "${EXT_SRC_DIR}/extension.js" "${installed}/extension.js"
                cp "${EXT_SRC_DIR}/metadata.json" "${installed}/metadata.json" ;;
            enable) return 1 ;;
        esac
    }
    local output
    output="$(install_gnome_extension)" || return 1
    cmp -s "${EXT_SRC_DIR}/extension.js" "${installed}/extension.js" || return 1
    cmp -s "${EXT_SRC_DIR}/metadata.json" "${installed}/metadata.json" || return 1
    assert_contains "${output}" 'log out and back in' || return 1
)

run_test() {
    local name="$1" test_fn="$2"
    if "${test_fn}"; then
        printf 'ok - %s\n' "${name}"
    else
        printf 'not ok - %s\n' "${name}" >&2
        return 1
    fi
}

run_test "Artix selects pacman" test_artix_selects_pacman
run_test "unknown distro selects its only supported manager" test_unknown_distro_selects_only_available_manager
run_test "--skip-system-deps needs no package manager" test_skip_system_deps_needs_no_package_manager
run_test "--skip-system-deps allows a missing override" test_skip_system_deps_allows_missing_override
run_test "startx system dependencies include xdotool" test_startx_system_deps_include_xdotool
run_test "Wayland system dependencies exclude xdotool" test_wayland_system_deps_exclude_xdotool
run_test "non-systemd host gets manual ydotoold guidance" test_non_systemd_host_gets_manual_guidance
run_test "non-systemd host requires uinput access" test_non_systemd_host_requires_uinput_access
run_test "systemd ydotoold unit forces a private socket" test_systemd_ydotoold_unit_forces_private_socket
run_test "ydotoold rejects unsafe socket metadata" test_ydotoold_rejects_unsafe_socket_metadata
run_test "ydotoold rejects a symlink socket path" test_ydotoold_rejects_symlink_socket_path
run_test "doctor accepts platform capability blockers" test_doctor_accepts_platform_capability_blockers
run_test "doctor rejects missing install prerequisites" test_doctor_rejects_missing_install_prerequisite
run_test "doctor raw fallback accepts platform capability blockers" test_doctor_raw_fallback_accepts_platform_capability_blockers
run_test "doctor raw fallback rejects missing install prerequisites" test_doctor_raw_fallback_rejects_missing_install_prerequisite

run_test "enabled GNOME extension assets are upgraded" test_enabled_gnome_extension_is_upgraded

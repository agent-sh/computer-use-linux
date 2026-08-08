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

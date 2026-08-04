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

test_artix_selects_pacman() (
    export COMPUTER_USE_LINUX_OS_RELEASE_FILE="${FIXTURE_DIR}/os-release.artix"
    export XDG_SESSION_TYPE=x11
    export XDG_CURRENT_DESKTOP=unknown

    # shellcheck source=../install.sh
    source "${INSTALLER}"
    package_manager_available() { [[ "$1" == "pacman" ]]; }

    detect_distro >/dev/null
    assert_eq "${PKG_MANAGER}" "pacman"
    assert_eq "${DISTRO_FAMILY}" "arch"
)

test_unknown_distro_selects_only_available_manager() (
    export COMPUTER_USE_LINUX_OS_RELEASE_FILE="${FIXTURE_DIR}/os-release.unknown"
    export XDG_SESSION_TYPE=x11
    export XDG_CURRENT_DESKTOP=unknown

    # shellcheck source=../install.sh
    source "${INSTALLER}"
    FORCE_UNKNOWN_DISTRO=1
    package_manager_available() { [[ "$1" == "dnf" ]]; }

    detect_distro >/dev/null
    assert_eq "${PKG_MANAGER}" "dnf"
    assert_eq "${DISTRO_FAMILY}" "fedora"
)

test_non_systemd_host_gets_manual_guidance() (
    export XDG_RUNTIME_DIR="/run/user/test"

    # shellcheck source=../install.sh
    source "${INSTALLER}"
    systemd_user_manager_available() { return 1; }
    ydotoold() { :; }

    local output
    output="$(setup_ydotoold)"
    assert_contains "${output}" "systemd --user is unavailable"
    assert_contains "${output}" "configure your per-user supervisor to run"
    assert_contains "${output}" "ydotoold --socket-path=/run/user/test/.ydotool_socket"
    assert_contains "${output}" "do not run ydotoold as root"
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
run_test "non-systemd host gets manual ydotoold guidance" test_non_systemd_host_gets_manual_guidance

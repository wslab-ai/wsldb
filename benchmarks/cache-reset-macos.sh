#!/usr/bin/env bash

set -euo pipefail

fail() {
    printf 'macOS cache reset error: %s\n' "$*" >&2
    exit 1
}

check_host() {
    [[ "$(uname -s)" == Darwin ]] || fail "this helper requires macOS"
    [[ -x /bin/sync ]] || fail "/bin/sync is unavailable"
    [[ -x /usr/sbin/purge ]] || fail "/usr/sbin/purge is unavailable"
    if (( EUID != 0 )); then
        [[ -x /usr/bin/sudo ]] || fail "/usr/bin/sudo is unavailable"
    fi
}

if [[ $# -eq 1 && "$1" == --check-host ]]; then
    check_host
    printf 'macOS cache reset host support is available\n'
    exit 0
fi
if [[ $# -eq 1 && "$1" == --check ]]; then
    check_host
    if (( EUID != 0 )); then
        /usr/bin/sudo -n -v || fail "non-interactive sudo authorization is not ready"
    fi
    printf 'macOS cache reset helper is ready\n'
    exit 0
fi
[[ $# -eq 2 ]] || fail "usage: $0 --check-host | --check | DATABASE LABEL"

readonly DATABASE=$1
readonly LABEL=$2
check_host
[[ -e "$DATABASE" ]] || fail "database path does not exist: $DATABASE"

printf 'cache_reset_begin platform=macos label=%s database=%s\n' "$LABEL" "$DATABASE"
/bin/sync
if (( EUID == 0 )); then
    /usr/sbin/purge
else
    /usr/bin/sudo -n /usr/sbin/purge
fi
printf 'cache_reset_complete platform=macos label=%s database=%s\n' "$LABEL" "$DATABASE"

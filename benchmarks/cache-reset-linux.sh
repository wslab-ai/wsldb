#!/usr/bin/env bash

set -euo pipefail

readonly DROP_CACHES=/proc/sys/vm/drop_caches

fail() {
    printf 'Linux cache reset error: %s\n' "$*" >&2
    exit 1
}

check_host() {
    [[ "$(uname -s)" == Linux ]] || fail "this helper requires Linux"
    [[ -e "$DROP_CACHES" ]] || fail "$DROP_CACHES is unavailable"
    command -v sync >/dev/null 2>&1 || fail "sync is unavailable"
    if [[ ! -w "$DROP_CACHES" ]]; then
        command -v sudo >/dev/null 2>&1 || fail "sudo is unavailable"
        command -v sh >/dev/null 2>&1 || fail "sh is unavailable"
    fi
}

if [[ $# -eq 1 && "$1" == --check-host ]]; then
    check_host
    printf 'Linux cache reset host support is available\n'
    exit 0
fi
if [[ $# -eq 1 && "$1" == --check ]]; then
    check_host
    if [[ ! -w "$DROP_CACHES" ]]; then
        sudo_command="$(command -v sudo)"
        readonly sudo_command
        "$sudo_command" -n -v || fail "non-interactive sudo authorization is not ready"
    fi
    printf 'Linux cache reset helper is ready\n'
    exit 0
fi
[[ $# -eq 2 ]] || fail "usage: $0 --check-host | --check | DATABASE LABEL"

readonly DATABASE=$1
readonly LABEL=$2
check_host
[[ -e "$DATABASE" ]] || fail "database path does not exist: $DATABASE"

printf 'cache_reset_begin platform=linux label=%s database=%s\n' "$LABEL" "$DATABASE"
sync_command="$(command -v sync)"
readonly sync_command
"$sync_command"
if [[ -w "$DROP_CACHES" ]]; then
    printf '3' >"$DROP_CACHES"
else
    sudo_command="$(command -v sudo)"
    shell_command="$(command -v sh)"
    readonly sudo_command shell_command
    "$sudo_command" -n "$shell_command" -c 'printf 3 > /proc/sys/vm/drop_caches'
fi
printf 'cache_reset_complete platform=linux label=%s database=%s\n' "$LABEL" "$DATABASE"

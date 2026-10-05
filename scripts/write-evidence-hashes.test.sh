#!/usr/bin/env bash

set -euo pipefail

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
readonly WRITER="$SCRIPT_DIR/write-evidence-hashes.sh"

fail() {
    printf 'evidence hash test failed: %s\n' "$*" >&2
    exit 1
}

expect_failure() {
    if "$@" >/dev/null 2>&1; then
        fail "command unexpectedly succeeded: $*"
    fi
}

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{ print $1 }'
    else
        shasum -a 256 "$1" | awk '{ print $1 }'
    fi
}

expect_hash() {
    local directory=$1
    local relative=$2
    local expected
    local recorded
    expected="$(sha256_file "$directory/$relative")"
    recorded="$(awk -v relative="$relative" '
        $2 == relative { matches += 1; digest = $1 }
        END {
            if (matches != 1) exit 1
            print digest
        }
    ' "$directory/evidence-sha256.txt")" || fail "missing hash: $relative"
    [[ "$recorded" == "$expected" ]] || fail "incorrect hash: $relative"
}

scratch="$(mktemp -d "${TMPDIR:-/tmp}/wsldb-evidence-hash-test.XXXXXX")"
cleanup() {
    if [[ -d "$scratch" ]]; then
        find "$scratch" -depth -delete
    fi
}
trap cleanup EXIT INT TERM

complete="$scratch/complete"
mkdir -p "$complete/raw"
printf 'manifest\n' >"$complete/manifest.txt"
printf 'log\n' >"$complete/raw/run.log"
"$WRITER" "$complete"
[[ "$(wc -l <"$complete/evidence-sha256.txt" | tr -d ' ')" == 2 ]] || \
    fail "hash manifest has the wrong number of records"
expect_hash "$complete" manifest.txt
expect_hash "$complete" raw/run.log
before="$(sha256_file "$complete/evidence-sha256.txt")"
expect_failure "$WRITER" "$complete"
[[ "$(sha256_file "$complete/evidence-sha256.txt")" == "$before" ]] || \
    fail "a repeated invocation changed the hash manifest"

empty="$scratch/empty"
mkdir "$empty"
expect_failure "$WRITER" "$empty"
[[ ! -e "$empty/evidence-sha256.txt" ]] || fail "empty evidence was published"

invalid="$scratch/invalid"
mkdir "$invalid"
printf 'invalid\n' >"$invalid/"$'line\nbreak'
expect_failure "$WRITER" "$invalid"
[[ ! -e "$invalid/evidence-sha256.txt" ]] || fail "invalid evidence was published"
expect_failure "$WRITER" "$scratch/missing"

printf 'evidence hash tests passed\n'

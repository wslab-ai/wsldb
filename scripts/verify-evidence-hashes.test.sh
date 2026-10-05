#!/usr/bin/env bash

set -euo pipefail

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
readonly WRITER="$SCRIPT_DIR/write-evidence-hashes.sh"
readonly VERIFIER="$SCRIPT_DIR/verify-evidence-hashes.sh"

fail() {
    printf 'evidence hash verifier test failed: %s\n' "$*" >&2
    exit 1
}

expect_failure() {
    if "$@" >/dev/null 2>&1; then
        fail "command unexpectedly succeeded: $*"
    fi
}

scratch="$(mktemp -d "${TMPDIR:-/tmp}/wsldb-evidence-verify-test.XXXXXX")"
cleanup() {
    if [[ -d "$scratch" ]]; then
        find "$scratch" -depth -delete
    fi
}
trap cleanup EXIT INT TERM

new_evidence() {
    local name=$1
    local directory="$scratch/$name"
    mkdir -p "$directory/raw"
    printf 'manifest\n' >"$directory/manifest.txt"
    printf 'log\n' >"$directory/raw/run.log"
    printf 'spaced\n' >"$directory/raw/file with spaces.log"
    "$WRITER" "$directory"
    printf '%s\n' "$directory"
}

complete="$(new_evidence complete)"
"$VERIFIER" "$complete" >/dev/null

tampered="$(new_evidence tampered)"
printf 'changed\n' >>"$tampered/raw/run.log"
expect_failure "$VERIFIER" "$tampered"

missing="$(new_evidence missing)"
rm -- "$missing/raw/run.log"
expect_failure "$VERIFIER" "$missing"

extra="$(new_evidence extra)"
printf 'extra\n' >"$extra/extra.txt"
expect_failure "$VERIFIER" "$extra"

duplicate="$(new_evidence duplicate)"
head -n 1 "$duplicate/evidence-sha256.txt" >>"$duplicate/evidence-sha256.txt"
expect_failure "$VERIFIER" "$duplicate"

malformed="$scratch/malformed"
mkdir "$malformed"
printf '%064d  ../outside\n' 0 >"$malformed/evidence-sha256.txt"
expect_failure "$VERIFIER" "$malformed"

linked="$(new_evidence linked)"
ln -s manifest.txt "$linked/manifest-link"
expect_failure "$VERIFIER" "$linked"

nonregular="$(new_evidence nonregular)"
mkfifo "$nonregular/unlisted-pipe"
expect_failure "$VERIFIER" "$nonregular"

empty="$scratch/empty"
mkdir "$empty"
: >"$empty/evidence-sha256.txt"
expect_failure "$VERIFIER" "$empty"

expect_failure "$VERIFIER" "$scratch/absent"
printf 'evidence hash verifier tests passed\n'

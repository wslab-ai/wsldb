#!/usr/bin/env bash

set -euo pipefail

fail() {
    printf 'evidence hash verification error: %s\n' "$*" >&2
    exit 1
}

for required in awk cmp find mktemp sort uniq; do
    command -v "$required" >/dev/null 2>&1 || \
        fail "required command is unavailable: $required"
done

if [[ $# -ne 1 ]]; then
    fail "usage: $0 /sealed/evidence/directory"
fi

[[ -d "$1" ]] || fail "evidence directory is missing: $1"
readonly DIRECTORY="$(cd "$1" && pwd -P)"
readonly MANIFEST="$DIRECTORY/evidence-sha256.txt"
[[ -f "$MANIFEST" && ! -L "$MANIFEST" ]] || \
    fail "hash manifest is missing or is not a regular file: $MANIFEST"

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{ print $1 }'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{ print $1 }'
    else
        fail "sha256sum or shasum is required"
    fi
}

scratch="$(mktemp -d "${TMPDIR:-/tmp}/wsldb-evidence-verify.XXXXXX")"
cleanup() {
    if [[ -d "$scratch" ]]; then
        find "$scratch" -depth -delete
    fi
}
trap cleanup EXIT INT TERM

recorded="$scratch/recorded"
actual="$scratch/actual"
: >"$recorded"
: >"$actual"
line_number=0
while IFS= read -r line || [[ -n "$line" ]]; do
    line_number=$((line_number + 1))
    digest=${line%%  *}
    [[ "$line" == "$digest  "* ]] || \
        fail "malformed record at line $line_number"
    relative=${line#"$digest  "}
    [[ "$digest" =~ ^[0-9a-f]{64}$ ]] || \
        fail "invalid SHA-256 at line $line_number"
    [[ -n "$relative" && "$relative" != /* && "$relative" != *$'\r'* ]] || \
        fail "invalid path at line $line_number"
    case "/$relative/" in
        */../* | */./*) fail "unsafe path at line $line_number: $relative" ;;
    esac
    [[ "$relative" != "evidence-sha256.txt" ]] || \
        fail "hash manifest must not contain itself"
    path="$DIRECTORY/$relative"
    [[ -f "$path" && ! -L "$path" ]] || \
        fail "listed evidence file is missing or is not regular: $relative"
    [[ "$(sha256_file "$path")" == "$digest" ]] || \
        fail "SHA-256 mismatch: $relative"
    printf '%s\n' "$relative" >>"$recorded"
done <"$MANIFEST"
(( line_number > 0 )) || fail "hash manifest is empty"

if [[ -n "$(find "$DIRECTORY" -type l -print -quit)" ]]; then
    fail "evidence directory contains a symbolic link"
fi
if [[ -n "$(find "$DIRECTORY" ! -type d ! -type f -print -quit)" ]]; then
    fail "evidence directory contains a non-regular entry"
fi

while IFS= read -r -d '' file; do
    [[ "$file" != "$MANIFEST" ]] || continue
    relative=${file#"$DIRECTORY"/}
    [[ "$relative" != "$file" && "$relative" != *$'\n'* && "$relative" != *$'\r'* ]] || \
        fail "invalid evidence path: $file"
    printf '%s\n' "$relative" >>"$actual"
done < <(find "$DIRECTORY" -type f -print0)

LC_ALL=C sort "$recorded" -o "$recorded.sorted"
LC_ALL=C sort "$actual" -o "$actual.sorted"
if [[ -n "$(uniq -d "$recorded.sorted")" ]]; then
    fail "hash manifest contains duplicate paths"
fi
cmp -s "$recorded.sorted" "$actual.sorted" || \
    fail "hash manifest does not cover the exact evidence file set"

printf 'evidence hashes verified\n'

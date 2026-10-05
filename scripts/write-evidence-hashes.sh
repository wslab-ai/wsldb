#!/usr/bin/env bash

set -euo pipefail

fail() {
    printf 'evidence hash error: %s\n' "$*" >&2
    exit 1
}

if [[ $# -ne 1 ]]; then
    fail "usage: $0 /completed/evidence/directory"
fi

[[ -d "$1" ]] || fail "evidence directory is missing: $1"
readonly DIRECTORY="$(cd "$1" && pwd -P)"
readonly DESTINATION="$DIRECTORY/evidence-sha256.txt"
[[ ! -e "$DESTINATION" ]] || fail "hash manifest already exists: $DESTINATION"

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{ print $1 }'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{ print $1 }'
    else
        fail "sha256sum or shasum is required"
    fi
}

temporary="$(mktemp "$DIRECTORY/.evidence-sha256.XXXXXX")"
cleanup() {
    if [[ -n "$temporary" && -f "$temporary" ]]; then
        rm -f -- "$temporary"
    fi
}
trap cleanup EXIT INT TERM

while IFS= read -r file; do
    if [[ "$file" == "$temporary" || "$file" == "$DESTINATION" ]]; then
        continue
    fi
    relative=${file#"$DIRECTORY"/}
    if [[ "$relative" == "$file" || "$relative" == *$'\n'* ]]; then
        fail "invalid evidence path: $file"
    fi
    printf '%s  %s\n' "$(sha256_file "$file")" "$relative" >>"$temporary"
done < <(find "$DIRECTORY" -type f -print | LC_ALL=C sort)

[[ -s "$temporary" ]] || fail "evidence directory contains no regular files"
mv -- "$temporary" "$DESTINATION"
temporary=

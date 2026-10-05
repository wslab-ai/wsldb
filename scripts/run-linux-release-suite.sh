#!/usr/bin/env bash

set -euo pipefail

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
readonly REPOSITORY="$(cd "$SCRIPT_DIR/.." && pwd -P)"
readonly MINIMUM_KIB="${WSLDB_LINUX_RELEASE_MINIMUM_KIB:-5242880}"
readonly BUILD_JOBS="${WSLDB_LINUX_RELEASE_BUILD_JOBS:-2}"

fail() {
    printf 'Linux release suite error: %s\n' "$*" >&2
    exit 1
}

if [[ $# -ne 2 ]]; then
    fail "usage: $0 /ext4/scratch-parent /new/evidence-directory"
fi

[[ "$(uname -s)" == Linux ]] || fail "this suite must run on Linux"
for required in cargo rustc git rg findmnt /usr/bin/time; do
    if [[ "$required" == /* ]]; then
        [[ -x "$required" ]] || fail "required executable is unavailable: $required"
    else
        command -v "$required" >/dev/null 2>&1 || \
            fail "required command is unavailable: $required"
    fi
done
[[ "$MINIMUM_KIB" =~ ^[1-9][0-9]*$ ]] || \
    fail "WSLDB_LINUX_RELEASE_MINIMUM_KIB must be positive"
[[ "$BUILD_JOBS" =~ ^[1-9][0-9]*$ ]] || \
    fail "WSLDB_LINUX_RELEASE_BUILD_JOBS must be positive"
[[ -z "$(git -C "$REPOSITORY" status --porcelain=v1)" ]] || \
    fail "WSLDB checkout must be clean"

readonly REQUESTED_SCRATCH=$1
readonly REQUESTED_OUTPUT=$2
[[ -d "$REQUESTED_SCRATCH" ]] || fail "scratch parent does not exist"
readonly SCRATCH_PARENT="$(cd "$REQUESTED_SCRATCH" && pwd -P)"
readonly FILESYSTEM="$(findmnt -T "$SCRATCH_PARENT" -n -o FSTYPE)"
[[ "$FILESYSTEM" == ext4 ]] || \
    fail "scratch parent must be ext4, found $FILESYSTEM"

output_parent="$(dirname -- "$REQUESTED_OUTPUT")"
[[ -d "$output_parent" ]] || fail "evidence parent does not exist: $output_parent"
readonly OUTPUT_PARENT="$(cd "$output_parent" && pwd -P)"
readonly OUTPUT="$OUTPUT_PARENT/$(basename -- "$REQUESTED_OUTPUT")"
[[ ! -e "$OUTPUT" ]] || fail "evidence directory already exists: $OUTPUT"
case "$OUTPUT" in
    "$REPOSITORY" | "$REPOSITORY"/*) fail "evidence directory must be outside the checkout" ;;
esac

available_kib="$(df -Pk "$SCRATCH_PARENT" | awk 'NR == 2 { print $4 }')"
[[ "$available_kib" =~ ^[0-9]+$ ]] || fail "could not determine scratch free space"
(( available_kib >= MINIMUM_KIB )) || \
    fail "scratch filesystem has ${available_kib} KiB free; ${MINIMUM_KIB} KiB required"

mkdir -- "$OUTPUT"
mkdir -- "$OUTPUT/raw"
WORK="$(mktemp -d "$SCRATCH_PARENT/wsldb-linux-release.XXXXXX")"
readonly WORK
mkdir -- "$WORK/tmp"

cleanup() {
    case "$WORK" in
        "$SCRATCH_PARENT"/wsldb-linux-release.*)
            if [[ -d "$WORK" ]]; then
                find "$WORK" -depth -delete
            fi
            ;;
        *) printf 'refusing to clean unexpected work path: %s\n' "$WORK" >&2 ;;
    esac
}
trap cleanup EXIT INT TERM

export CARGO_BUILD_JOBS="$BUILD_JOBS"
export CARGO_INCREMENTAL=0
export CARGO_TARGET_DIR="$WORK/target"
export TMPDIR="$WORK/tmp"

: >"$OUTPUT/commands.log"
: >"$OUTPUT/steps.tsv"

record_command() {
    local label=$1
    shift
    printf '# %s\n' "$label" >>"$OUTPUT/commands.log"
    printf '%q ' "$@" >>"$OUTPUT/commands.log"
    printf '\n' >>"$OUTPUT/commands.log"
}

run_step() {
    local label=$1
    shift
    record_command "$label" "$@"
    printf 'running %s\n' "$label"
    set +e
    /usr/bin/time -v -o "$OUTPUT/raw/$label.time" \
        "$@" >"$OUTPUT/raw/$label.stdout" 2>"$OUTPUT/raw/$label.stderr"
    local status=$?
    set -e
    printf '%s\t%s\n' "$label" "$status" >>"$OUTPUT/steps.tsv"
    if (( status != 0 )); then
        printf 'suite_complete=no\nfailed_step=%s\n' "$label" >>"$OUTPUT/manifest.txt"
        fail "$label failed; evidence retained at $OUTPUT"
    fi
}

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{ print $1 }'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{ print $1 }'
    else
        fail "sha256sum or shasum is required"
    fi
}

readonly SOURCE_COMMIT="$(git -C "$REPOSITORY" rev-parse HEAD)"
readonly RELEASE_RUNNER_SHA256="$(sha256_file "$SCRIPT_DIR/run-linux-release-suite.sh")"
readonly EVIDENCE_HASH_WRITER_SHA256="$(
    sha256_file "$SCRIPT_DIR/write-evidence-hashes.sh"
)"
readonly EVIDENCE_HASH_VERIFIER_SHA256="$(
    sha256_file "$SCRIPT_DIR/verify-evidence-hashes.sh"
)"
readonly SOURCE_SNAPSHOT="$WORK/source"

require_unchanged_file() {
    local path=$1
    local expected=$2
    local label=$3
    [[ -f "$path" ]] || fail "$label is no longer a regular file: $path"
    [[ "$(sha256_file "$path")" == "$expected" ]] || \
        fail "$label changed during the release suite"
}

require_unchanged_source() {
    [[ "$(git -C "$REPOSITORY" rev-parse HEAD)" == "$SOURCE_COMMIT" ]] || \
        fail "WSLDB commit changed during the release suite"
    [[ -z "$(git -C "$REPOSITORY" status --porcelain=v1)" ]] || \
        fail "WSLDB checkout changed during the release suite"
}

{
    printf 'wsldb_commit=%s\n' "$SOURCE_COMMIT"
    printf 'release_runner_sha256=%s\n' "$RELEASE_RUNNER_SHA256"
    printf 'evidence_hash_writer_sha256=%s\n' "$EVIDENCE_HASH_WRITER_SHA256"
    printf 'evidence_hash_verifier_sha256=%s\n' "$EVIDENCE_HASH_VERIFIER_SHA256"
    printf 'wsldb_status_begin\n'
    git -C "$REPOSITORY" status --porcelain=v1
    printf 'wsldb_status_end\n'
    printf 'filesystem=%s\n' "$FILESYSTEM"
    printf 'scratch_parent=%s\n' "$SCRATCH_PARENT"
    printf 'source_snapshot=isolated-commit\n'
    printf 'build_jobs=%s\n' "$BUILD_JOBS"
    printf 'minimum_kib=%s\navailable_kib=%s\n' "$MINIMUM_KIB" "$available_kib"
    printf 'mount_begin\n'
    findmnt -T "$SCRATCH_PARENT"
    printf 'mount_end\n'
    printf 'system_begin\n'
    uname -a
    rustc -Vv
    cargo -V
    command -v lscpu >/dev/null 2>&1 && lscpu
    command -v free >/dev/null 2>&1 && free -b
    command -v lsblk >/dev/null 2>&1 && lsblk
    printf 'system_end\n'
} >"$OUTPUT/manifest.txt" 2>&1

run_step source-clone git clone --quiet --no-local --no-checkout \
    "$REPOSITORY" "$SOURCE_SNAPSHOT"
run_step source-checkout git -C "$SOURCE_SNAPSHOT" checkout --quiet --detach \
    "$SOURCE_COMMIT"
[[ "$(git -C "$SOURCE_SNAPSHOT" rev-parse HEAD)" == "$SOURCE_COMMIT" ]] || \
    fail "isolated source snapshot has the wrong commit"
[[ -z "$(git -C "$SOURCE_SNAPSHOT" status --porcelain=v1)" ]] || \
    fail "isolated source snapshot is dirty"

cd "$SOURCE_SNAPSHOT"
run_step release-audit scripts/release-audit.sh
run_step format cargo fmt --all -- --check
run_step check cargo check --locked --all-targets --all-features
run_step clippy cargo clippy --locked --all-targets --all-features -- -D warnings
run_step test-debug cargo test --locked --all-targets --all-features
run_step test-release cargo test --locked --release --all-targets --all-features
run_step rustdoc env RUSTDOCFLAGS=-D warnings \
    cargo doc --locked --no-deps --all-features
run_step package cargo package --locked

require_unchanged_source
[[ "$(git -C "$SOURCE_SNAPSHOT" rev-parse HEAD)" == "$SOURCE_COMMIT" ]] || \
    fail "isolated source snapshot commit changed"
[[ -z "$(git -C "$SOURCE_SNAPSHOT" status --porcelain=v1)" ]] || \
    fail "isolated source snapshot changed during the release suite"
require_unchanged_file "$SCRIPT_DIR/run-linux-release-suite.sh" \
    "$RELEASE_RUNNER_SHA256" "release runner"
require_unchanged_file "$SCRIPT_DIR/write-evidence-hashes.sh" \
    "$EVIDENCE_HASH_WRITER_SHA256" "evidence hash writer"
require_unchanged_file "$SCRIPT_DIR/verify-evidence-hashes.sh" \
    "$EVIDENCE_HASH_VERIFIER_SHA256" "evidence hash verifier"
printf 'runner_inputs_revalidated=yes\n' >>"$OUTPUT/manifest.txt"

readonly PACKAGE="$CARGO_TARGET_DIR/package/wsldb-1.0.0.crate"
[[ -f "$PACKAGE" ]] || fail "Cargo package archive is missing: $PACKAGE"
cp -- "$PACKAGE" "$OUTPUT/wsldb-1.0.0.crate"
printf 'package_sha256=%s\n' "$(sha256_file "$OUTPUT/wsldb-1.0.0.crate")" \
    >>"$OUTPUT/manifest.txt"
printf 'suite_complete=yes\nevidence_hashes_file=evidence-sha256.txt\n' \
    >>"$OUTPUT/manifest.txt"
"$SCRIPT_DIR/write-evidence-hashes.sh" "$OUTPUT"
if ! "$SCRIPT_DIR/verify-evidence-hashes.sh" "$OUTPUT"; then
    rm -f -- "$OUTPUT/evidence-sha256.txt"
    fail "sealed evidence failed self-verification; seal removed"
fi
printf 'Linux ext4 release suite passed; evidence: %s\n' "$OUTPUT"

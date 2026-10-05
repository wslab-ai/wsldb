#!/usr/bin/env bash
set -euo pipefail

repository=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)

fail() {
    echo "power-loss runner: $*" >&2
    exit 1
}

usage() {
    cat >&2 <<'EOF'
usage:
  scripts/run-power-loss-cycle.sh --check-host
  scripts/run-power-loss-cycle.sh prepare direct|buffered DATABASE ORACLE NEW_EVIDENCE_DIR
  scripts/run-power-loss-cycle.sh run EVIDENCE_DIR
  scripts/run-power-loss-cycle.sh verify EVIDENCE_DIR
  scripts/run-power-loss-cycle.sh seal EVIDENCE_DIR

prepare creates a pinned probe and initialized database. Fill hardware.txt,
then invoke run and physically remove power while it is writing. After reboot,
fill the cycle cut-attestation.txt and invoke verify before another run.
EOF
    exit 2
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || fail "required command is unavailable: $1"
}

sha256_file() {
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    elif command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        fail "shasum or sha256sum is required"
    fi
}

host_os() {
    uname -s
}

check_host() {
    local system
    system=$(host_os)
    case "$system" in
        Darwin)
            for command_name in awk basename cargo chmod cp date df dirname find git grep mkdir sed stat sync sysctl tee tr uname wc; do
                require_command "$command_name"
            done
            ;;
        Linux)
            for command_name in awk basename cargo chmod cp date dirname find findmnt git grep mkdir sed stat sync tee tr uname wc; do
                require_command "$command_name"
            done
            ;;
        *) fail "unsupported host: $system" ;;
    esac
    sha256_file "$0" >/dev/null
    boot_token >/dev/null
    echo "power-loss runner host support is available for $system"
}

canonical_new_file() {
    local candidate=$1
    local parent name
    parent=$(dirname "$candidate")
    name=$(basename "$candidate")
    [[ -d "$parent" ]] || fail "parent directory does not exist: $parent"
    [[ "$name" != "." && "$name" != ".." && "$name" != "/" ]] || fail "invalid file path"
    parent=$(cd "$parent" && pwd -P)
    printf '%s/%s\n' "$parent" "$name"
}

canonical_directory() {
    [[ -d "$1" ]] || fail "directory does not exist: $1"
    (cd "$1" && pwd -P)
}

mount_type() {
    local location=$1
    case "$(host_os)" in
        Darwin) stat -f '%T' "$location" ;;
        Linux) findmnt -n -o FSTYPE --target "$location" ;;
        *) fail "unsupported host" ;;
    esac
}

mount_source() {
    local location=$1
    case "$(host_os)" in
        Darwin) df -P "$location" | awk 'NR == 2 {print $1}' ;;
        Linux) findmnt -n -o SOURCE --target "$location" ;;
        *) fail "unsupported host" ;;
    esac
}

require_supported_database_filesystem() {
    local filesystem=$1
    case "$filesystem" in
        apfs | ext4 | xfs) ;;
        *) fail "database filesystem must be APFS, ext4, or XFS, got: $filesystem" ;;
    esac
}

boot_token() {
    case "$(host_os)" in
        Darwin) sysctl -n kern.boottime | sha256_stream ;;
        Linux) sed -n '1p' /proc/sys/kernel/random/boot_id ;;
        *) fail "unsupported host" ;;
    esac
}

sha256_stream() {
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 | awk '{print $1}'
    elif command -v sha256sum >/dev/null 2>&1; then
        sha256sum | awk '{print $1}'
    else
        fail "shasum or sha256sum is required"
    fi
}

write_value() {
    local destination=$1
    local value=$2
    [[ "$value" != *$'\n'* ]] || fail "state values cannot contain newlines"
    printf '%s\n' "$value" >"$destination"
}

read_value() {
    local source=$1
    local lines value
    [[ -f "$source" ]] || fail "missing state file: $source"
    lines=$(wc -l <"$source" | tr -d ' ')
    [[ "$lines" == "1" ]] || fail "state file must contain exactly one line: $source"
    value=$(sed -n '1p' "$source")
    [[ -n "$value" ]] || fail "state file is empty: $source"
    printf '%s\n' "$value"
}

require_note() {
    local notes=$1
    local field=$2
    grep -Eq "^${field}=.+$" "$notes" || fail "fill ${field}=... in $notes"
}

verify_probe_identity() {
    local evidence_dir=$1
    local probe expected actual
    probe="$evidence_dir/power_loss_probe"
    [[ -x "$probe" ]] || fail "pinned probe is missing or not executable"
    expected=$(read_value "$evidence_dir/probe.sha256")
    actual=$(sha256_file "$probe")
    [[ "$actual" == "$expected" ]] || fail "pinned probe hash changed"
}

verify_runner_identity() {
    local evidence_dir=$1
    local expected actual
    expected=$(read_value "$evidence_dir/runner.sha256")
    actual=$(sha256_file "$0")
    [[ "$actual" == "$expected" ]] || fail "power-loss runner hash changed"
}

record_platform() {
    local destination=$1
    {
        printf 'os=%s\n' "$(uname -s)"
        printf 'kernel=%s\n' "$(uname -r)"
        printf 'architecture=%s\n' "$(uname -m)"
        if [[ "$(host_os)" == "Darwin" ]] && command -v sw_vers >/dev/null 2>&1; then
            printf 'os_product=%s\n' "$(sw_vers -productName)"
            printf 'os_version=%s\n' "$(sw_vers -productVersion)"
            printf 'os_build=%s\n' "$(sw_vers -buildVersion)"
        elif [[ -f /etc/os-release ]]; then
            grep -E '^(ID|VERSION_ID)=' /etc/os-release || true
        fi
    } >"$destination"
}

file_size() {
    case "$(host_os)" in
        Darwin) stat -f '%z' "$1" ;;
        Linux) stat -c '%s' "$1" ;;
        *) fail "unsupported host" ;;
    esac
}

prepare() {
    [[ $# -eq 4 ]] || usage
    local mode=$1
    local database_file oracle_file evidence_candidate evidence_parent evidence_dir
    local database_parent oracle_parent database_fs database_mount oracle_mount evidence_mount
    local build_dir probe build_jobs
    case "$mode" in direct | buffered) ;; *) usage ;; esac

    check_host >/dev/null
    database_file=$(canonical_new_file "$2")
    oracle_file=$(canonical_new_file "$3")
    evidence_candidate=$4
    [[ ! -e "$database_file" ]] || fail "database path already exists: $database_file"
    [[ ! -e "${database_file}.wal" ]] || fail "database WAL path already exists"
    [[ ! -e "$oracle_file" ]] || fail "oracle path already exists: $oracle_file"
    [[ ! -e "$evidence_candidate" ]] || fail "evidence directory already exists"
    evidence_parent=$(canonical_directory "$(dirname "$evidence_candidate")")
    evidence_dir="$evidence_parent/$(basename "$evidence_candidate")"

    database_parent=$(dirname "$database_file")
    oracle_parent=$(dirname "$oracle_file")
    database_fs=$(mount_type "$database_parent")
    database_mount=$(mount_source "$database_parent")
    oracle_mount=$(mount_source "$oracle_parent")
    evidence_mount=$(mount_source "$evidence_parent")
    require_supported_database_filesystem "$database_fs"
    [[ "$database_mount" != "$oracle_mount" ]] || fail "oracle must use a different mount source"
    [[ "$database_mount" != "$evidence_mount" ]] || fail "evidence must survive on a different mount source"

    cd "$repository"
    [[ -z "$(git status --porcelain --untracked-files=normal)" ]] || fail "source checkout must be clean"
    mkdir "$evidence_dir"
    mkdir "$evidence_dir/cycles"
    write_value "$evidence_dir/mode" "$mode"
    write_value "$evidence_dir/database-path" "$database_file"
    write_value "$evidence_dir/oracle-path" "$oracle_file"
    write_value "$evidence_dir/source-commit" "$(git rev-parse HEAD)"
    write_value "$evidence_dir/runner.sha256" "$(sha256_file "$0")"
    write_value "$evidence_dir/evidence-hash-writer.sha256" \
        "$(sha256_file "$repository/scripts/write-evidence-hashes.sh")"
    write_value "$evidence_dir/evidence-hash-verifier.sha256" \
        "$(sha256_file "$repository/scripts/verify-evidence-hashes.sh")"
    write_value "$evidence_dir/database-filesystem" "$database_fs"
    write_value "$evidence_dir/database-mount-source" "$database_mount"
    write_value "$evidence_dir/oracle-mount-source" "$oracle_mount"
    write_value "$evidence_dir/evidence-mount-source" "$evidence_mount"
    write_value "$evidence_dir/prepared-boot-token" "$(boot_token)"
    write_value "$evidence_dir/next-cycle" "1"
    record_platform "$evidence_dir/platform-prepare.txt"
    printf '%s\n' \
        'device_model=' \
        'device_firmware=' \
        'controller=' \
        'write_cache=' \
        'power_protection=' \
        'oracle_independence=' \
        >"$evidence_dir/hardware.txt"

    build_dir="$evidence_dir/build-target"
    build_jobs=${WSLDB_POWER_LOSS_BUILD_JOBS:-2}
    case "$build_jobs" in *[!0-9]* | 0 | '') fail "WSLDB_POWER_LOSS_BUILD_JOBS must be positive" ;; esac
    CARGO_TARGET_DIR="$build_dir" CARGO_BUILD_JOBS="$build_jobs" \
        cargo build --locked --release --example power_loss_probe \
        >"$evidence_dir/build.stdout" 2>"$evidence_dir/build.stderr"
    probe="$build_dir/release/examples/power_loss_probe"
    [[ -x "$probe" ]] || fail "release probe was not built"
    cp "$probe" "$evidence_dir/power_loss_probe"
    chmod 0555 "$evidence_dir/power_loss_probe"
    write_value "$evidence_dir/probe.sha256" "$(sha256_file "$evidence_dir/power_loss_probe")"
    find "$build_dir" -depth -delete

    "$evidence_dir/power_loss_probe" init "$mode" "$database_file" "$oracle_file" \
        >"$evidence_dir/init.stdout" 2>"$evidence_dir/init.stderr"
    sync
    echo "prepared $evidence_dir"
    echo "fill $evidence_dir/hardware.txt before run"
}

load_state() {
    local evidence_dir=$1
    [[ ! -e "$evidence_dir/evidence-sha256.txt" ]] || \
        fail "evidence is sealed and cannot accept another cycle"
    MODE=$(read_value "$evidence_dir/mode")
    DATABASE_FILE=$(read_value "$evidence_dir/database-path")
    ORACLE_FILE=$(read_value "$evidence_dir/oracle-path")
    NEXT_CYCLE=$(read_value "$evidence_dir/next-cycle")
    case "$MODE" in direct | buffered) ;; *) fail "invalid recorded mode" ;; esac
    case "$NEXT_CYCLE" in *[!0-9]* | 0 | '') fail "invalid next-cycle state" ;; esac
    [[ -f "$DATABASE_FILE" && -f "$ORACLE_FILE" ]] || fail "probe database or oracle is missing"
    verify_runner_identity "$evidence_dir"
    verify_probe_identity "$evidence_dir"
}

run_cycle() {
    [[ $# -eq 1 ]] || usage
    local evidence_dir cycle_name cycle_dir status
    evidence_dir=$(canonical_directory "$1")
    load_state "$evidence_dir"
    for field in device_model device_firmware controller write_cache power_protection oracle_independence; do
        require_note "$evidence_dir/hardware.txt" "$field"
    done
    cycle_name=$(printf '%06d' "$NEXT_CYCLE")
    cycle_dir="$evidence_dir/cycles/$cycle_name"
    [[ ! -e "$cycle_dir" ]] || fail "cycle already exists: $cycle_dir"
    mkdir "$cycle_dir"
    write_value "$cycle_dir/run-boot-token" "$(boot_token)"
    write_value "$cycle_dir/run-started-utc" "$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
    printf '%s\n' \
        'physical_cut_method=' \
        'physical_cut_time_utc=' \
        'filesystem_recovery=' \
        >"$cycle_dir/cut-attestation.txt"
    record_platform "$cycle_dir/platform-before.txt"
    sync

    set +e
    "$evidence_dir/power_loss_probe" run "$MODE" "$DATABASE_FILE" "$ORACLE_FILE" \
        > >(tee "$cycle_dir/ack.stdout") \
        2> >(tee "$cycle_dir/run.stderr" >&2)
    status=$?
    set -e
    write_value "$cycle_dir/orderly-exit" "$status"
    sync
    fail "writer exited without physical power loss (status $status); retain this failed cycle"
}

record_artifact() {
    local output=$1
    local label=$2
    local artifact=$3
    if [[ -f "$artifact" ]]; then
        printf '%s_size=%s\n' "$label" "$(file_size "$artifact")" >>"$output"
        printf '%s_sha256=%s\n' "$label" "$(sha256_file "$artifact")" >>"$output"
    else
        printf '%s=absent\n' "$label" >>"$output"
    fi
}

verify_cycle() {
    [[ $# -eq 1 ]] || usage
    local evidence_dir cycle_name cycle_dir before_boot after_boot status
    local current_fs current_mount recorded_fs recorded_mount artifacts
    evidence_dir=$(canonical_directory "$1")
    load_state "$evidence_dir"
    cycle_name=$(printf '%06d' "$NEXT_CYCLE")
    cycle_dir="$evidence_dir/cycles/$cycle_name"
    [[ -d "$cycle_dir" ]] || fail "cycle has not been armed: $cycle_dir"
    [[ ! -e "$cycle_dir/orderly-exit" ]] || fail "cycle ended orderly and is not power-loss evidence"
    [[ ! -e "$cycle_dir/verify-exit" ]] || fail "cycle verification was already attempted"
    before_boot=$(read_value "$cycle_dir/run-boot-token")
    after_boot=$(boot_token)
    [[ "$before_boot" != "$after_boot" ]] || fail "boot token did not change; reboot is unproven"

    current_fs=$(mount_type "$(dirname "$DATABASE_FILE")")
    current_mount=$(mount_source "$(dirname "$DATABASE_FILE")")
    recorded_fs=$(read_value "$evidence_dir/database-filesystem")
    recorded_mount=$(read_value "$evidence_dir/database-mount-source")
    [[ "$current_fs" == "$recorded_fs" ]] || fail "database filesystem changed after reboot"
    [[ "$current_mount" == "$recorded_mount" ]] || fail "database mount source changed after reboot"

    for field in physical_cut_method physical_cut_time_utc filesystem_recovery; do
        require_note "$cycle_dir/cut-attestation.txt" "$field"
    done

    set +e
    "$evidence_dir/power_loss_probe" verify "$MODE" "$DATABASE_FILE" "$ORACLE_FILE" \
        > >(tee "$cycle_dir/verify.stdout") \
        2> >(tee "$cycle_dir/verify.stderr" >&2)
    status=$?
    set -e
    write_value "$cycle_dir/verify-exit" "$status"
    record_platform "$cycle_dir/platform-after.txt"
    [[ "$status" -eq 0 ]] || {
        write_value "$cycle_dir/technical-status" "failed"
        sync
        fail "probe verification failed; failed evidence was retained"
    }

    artifacts="$cycle_dir/artifacts.txt"
    : >"$artifacts"
    record_artifact "$artifacts" database "$DATABASE_FILE"
    record_artifact "$artifacts" wal "${DATABASE_FILE}.wal"
    record_artifact "$artifacts" oracle "$ORACLE_FILE"
    record_artifact "$artifacts" probe "$evidence_dir/power_loss_probe"
    write_value "$cycle_dir/after-boot-token" "$after_boot"
    write_value "$cycle_dir/hardware.sha256" "$(sha256_file "$evidence_dir/hardware.txt")"
    write_value "$cycle_dir/cut-attestation.sha256" "$(sha256_file "$cycle_dir/cut-attestation.txt")"
    write_value "$cycle_dir/technical-status" "passed"
    write_value "$cycle_dir/manual-review-required" "yes"
    write_value "$cycle_dir/release-gate-complete" "no"
    write_value "$evidence_dir/next-cycle" "$((NEXT_CYCLE + 1))"
    sync
    echo "cycle $cycle_name passed technical verification"
    echo "manual review is required; this cycle alone does not close the release gate"
}

seal_evidence() {
    [[ $# -eq 1 ]] || usage
    local evidence_dir cycle_dir cycle_name expected_name value
    local expected_cycle=1
    local cycle_count=0
    local writer="$repository/scripts/write-evidence-hashes.sh"
    local verifier="$repository/scripts/verify-evidence-hashes.sh"
    evidence_dir=$(canonical_directory "$1")
    load_state "$evidence_dir"
    [[ -x "$writer" ]] || fail "evidence hash writer is missing or not executable"
    [[ -x "$verifier" ]] || fail "evidence hash verifier is missing or not executable"
    for field in device_model device_firmware controller write_cache power_protection oracle_independence; do
        require_note "$evidence_dir/hardware.txt" "$field"
    done
    [[ "$(sha256_file "$writer")" == \
        "$(read_value "$evidence_dir/evidence-hash-writer.sha256")" ]] || \
        fail "evidence hash writer changed"
    [[ "$(sha256_file "$verifier")" == \
        "$(read_value "$evidence_dir/evidence-hash-verifier.sha256")" ]] || \
        fail "evidence hash verifier changed"

    shopt -s nullglob
    local cycle_directories=("$evidence_dir"/cycles/*)
    shopt -u nullglob
    ((${#cycle_directories[@]} > 0)) || fail "no power-loss cycles were recorded"
    for cycle_dir in "${cycle_directories[@]}"; do
        [[ -d "$cycle_dir" ]] || fail "unexpected non-directory in cycles: $cycle_dir"
        cycle_name=$(basename "$cycle_dir")
        expected_name=$(printf '%06d' "$expected_cycle")
        [[ "$cycle_name" == "$expected_name" ]] || \
            fail "power-loss cycle sequence is not contiguous"
        for required in ack.stdout run.stderr run-boot-token run-started-utc \
            cut-attestation.txt verify.stdout verify.stderr verify-exit \
            platform-before.txt platform-after.txt artifacts.txt after-boot-token \
            hardware.sha256 cut-attestation.sha256 technical-status \
            manual-review-required release-gate-complete; do
            [[ -f "$cycle_dir/$required" ]] || \
                fail "cycle $cycle_name is missing $required"
        done
        for field in physical_cut_method physical_cut_time_utc filesystem_recovery; do
            require_note "$cycle_dir/cut-attestation.txt" "$field"
        done
        value=$(read_value "$cycle_dir/verify-exit")
        [[ "$value" == 0 ]] || fail "cycle $cycle_name verification did not succeed"
        [[ "$(read_value "$cycle_dir/technical-status")" == passed ]] || \
            fail "cycle $cycle_name technical status is not passed"
        [[ "$(read_value "$cycle_dir/manual-review-required")" == yes ]] || \
            fail "cycle $cycle_name lost its manual-review requirement"
        [[ "$(read_value "$cycle_dir/release-gate-complete")" == no ]] || \
            fail "cycle $cycle_name makes an unsupported release-gate claim"
        [[ "$(read_value "$cycle_dir/hardware.sha256")" == \
            "$(sha256_file "$evidence_dir/hardware.txt")" ]] || \
            fail "cycle $cycle_name hardware record changed"
        [[ "$(read_value "$cycle_dir/cut-attestation.sha256")" == \
            "$(sha256_file "$cycle_dir/cut-attestation.txt")" ]] || \
            fail "cycle $cycle_name cut attestation changed"
        expected_cycle=$((expected_cycle + 1))
        cycle_count=$((cycle_count + 1))
    done
    [[ "$NEXT_CYCLE" == "$expected_cycle" ]] || \
        fail "next-cycle state does not follow the verified cycle sequence"

    write_value "$evidence_dir/sealed-utc" "$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
    write_value "$evidence_dir/sealed-cycle-count" "$cycle_count"
    write_value "$evidence_dir/release-gate-complete" "no"
    "$writer" "$evidence_dir"
    if ! "$verifier" "$evidence_dir"; then
        rm -f -- "$evidence_dir/evidence-sha256.txt"
        fail "sealed evidence failed self-verification; seal removed"
    fi
    sync
    echo "sealed $cycle_count technically verified cycle(s) in $evidence_dir"
    echo "manual matrix review is still required; the release gate remains open"
}

case "${1:-}" in
    --check-host)
        [[ $# -eq 1 ]] || usage
        check_host
        ;;
    prepare)
        shift
        prepare "$@"
        ;;
    run)
        shift
        run_cycle "$@"
        ;;
    verify)
        shift
        verify_cycle "$@"
        ;;
    seal)
        shift
        seal_evidence "$@"
        ;;
    *) usage ;;
esac

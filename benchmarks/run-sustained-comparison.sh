#!/usr/bin/env bash

set -euo pipefail

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
readonly REPOSITORY="$(cd "$SCRIPT_DIR/.." && pwd -P)"
source "$SCRIPT_DIR/benchmark-contract.sh"

fail() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

if [[ $# -ne 3 ]]; then
    fail "usage: $0 /path/to/rocksdb_sustained_bench /path/to/rocksdb/source /new/output/directory"
fi

readonly ROCKSDB_BENCH="$1"
readonly ROCKSDB_SOURCE="$2"
readonly REQUESTED_OUTPUT="$3"
[[ -x "$ROCKSDB_BENCH" ]] || fail "RocksDB sustained benchmark is not executable: $ROCKSDB_BENCH"
git -C "$ROCKSDB_SOURCE" rev-parse --is-inside-work-tree >/dev/null 2>&1 || \
    fail "RocksDB source is not a Git checkout: $ROCKSDB_SOURCE"
[[ ! -e "$REQUESTED_OUTPUT" ]] || fail "output already exists: $REQUESTED_OUTPUT"

readonly ENTRIES="${WSLDB_SUSTAINED_ENTRIES:-1000000}"
readonly VALUE_BYTES="${WSLDB_SUSTAINED_VALUE_BYTES:-100}"
readonly DURATION_SECONDS="${WSLDB_SUSTAINED_DURATION_SECONDS:-1800}"
readonly CACHE_BYTES="${WSLDB_SUSTAINED_CACHE_BYTES:-268435456}"
readonly COMPACT_BYTES="${WSLDB_SUSTAINED_COMPACT_BYTES:-536870912}"
readonly MAINTENANCE_SECONDS="${WSLDB_SUSTAINED_MAINTENANCE_SECONDS:-60}"
readonly ZIPF_EXPONENT="${WSLDB_SUSTAINED_ZIPF_EXPONENT:-1.2}"
readonly DISTRIBUTIONS="${WSLDB_SUSTAINED_DISTRIBUTIONS:-uniform zipf}"
readonly MEASURED_RUNS="${WSLDB_SUSTAINED_MEASURED_RUNS:-5}"
readonly FROZEN_WRITE_BUFFER_BYTES=67108864
readonly WRITE_BUFFER_BYTES="${WSLDB_SUSTAINED_WRITE_BUFFER_BYTES:-$FROZEN_WRITE_BUFFER_BYTES}"
readonly OPERATION_LIMIT="${WSLDB_SUSTAINED_OPERATION_LIMIT:-0}"
readonly SEED="0x4D595DF4D0F33173"

for number in "$ENTRIES" "$VALUE_BYTES" "$DURATION_SECONDS" "$CACHE_BYTES" \
    "$COMPACT_BYTES" "$MAINTENANCE_SECONDS" "$MEASURED_RUNS" \
    "$WRITE_BUFFER_BYTES"; do
    [[ "$number" =~ ^[1-9][0-9]*$ ]] || fail "numeric settings must be positive integers"
done
[[ "$OPERATION_LIMIT" =~ ^[0-9]+$ ]] || fail "operation limit must be a non-negative integer"
readonly CACHE_PAGES=$((CACHE_BYTES / 4096))
(( CACHE_PAGES > 0 )) || fail "cache must hold at least one page"
case "${WSLDB_ROCKSDB_WAL_STATS:-}" in
    "") readonly ROCKSDB_WAL_STATS_ENABLED=0 ;;
    1) readonly ROCKSDB_WAL_STATS_ENABLED=1 ;;
    *) fail "WSLDB_ROCKSDB_WAL_STATS must be unset or 1" ;;
esac
seen_uniform=0
seen_zipf=0
for distribution in $DISTRIBUTIONS; do
    case "$distribution" in
        uniform)
            ((seen_uniform == 0)) || fail "uniform distribution is duplicated"
            seen_uniform=1
            ;;
        zipf)
            ((seen_zipf == 0)) || fail "zipf distribution is duplicated"
            seen_zipf=1
            ;;
        *) fail "distributions must contain uniform and/or zipf" ;;
    esac
done
((seen_uniform + seen_zipf > 0)) || fail "at least one sustained distribution is required"

release_mode="$(benchmark_contract_mode)" || fail "invalid benchmark contract mode"
readonly RELEASE_MODE="$release_mode"
readonly WSLDB_SOURCE_COMMIT="$(git -C "$REPOSITORY" rev-parse HEAD)"
if [[ "$RELEASE_MODE" == 1 && -n "${WSLDB_BENCH_BIN:-}" ]]; then
    fail "release mode builds WSLDB from the recorded clean checkout"
fi
if [[ "$RELEASE_MODE" == 1 && "$WRITE_BUFFER_BYTES" != "$FROZEN_WRITE_BUFFER_BYTES" ]]; then
    fail "release mode requires a $FROZEN_WRITE_BUFFER_BYTES-byte write buffer"
fi
benchmark_validate_release_inputs "$RELEASE_MODE" "$REPOSITORY" \
    "$ROCKSDB_SOURCE" "$ROCKSDB_BENCH" "$BENCHMARK_ROCKSDB_COMMIT" \
    "rocksdb_sustained_bench version $BENCHMARK_ROCKSDB_VERSION" || \
    fail "release benchmark inputs do not satisfy the frozen contract"

mkdir -p "$REQUESTED_OUTPUT/raw"
readonly OUTPUT="$(cd "$REQUESTED_OUTPUT" && pwd -P)"

readonly SCRATCH_PARENT="${TMPDIR:-/tmp}"
SCRATCH="$(mktemp -d "$SCRATCH_PARENT/wsldb-sustained.XXXXXX")"
readonly SCRATCH

remove_scratch_path() {
    local target="$1"
    case "$target" in
        "$SCRATCH"/*) rm -rf -- "$target" ;;
        *) fail "refusing to remove a path outside benchmark scratch space: $target" ;;
    esac
}

cleanup() {
    case "$SCRATCH" in
        "$SCRATCH_PARENT"/wsldb-sustained.*) rm -rf -- "$SCRATCH" ;;
        *) printf 'refusing to remove unexpected scratch path: %s\n' "$SCRATCH" >&2 ;;
    esac
}
trap cleanup EXIT INT TERM

WSLDB_BUILD_SOURCE="$REPOSITORY"
WSLDB_BUILD_SNAPSHOT=working-tree
if [[ "$RELEASE_MODE" == 1 ]]; then
    WSLDB_BUILD_SOURCE="$SCRATCH/wsldb-source"
    WSLDB_BUILD_SNAPSHOT=isolated-commit
    benchmark_create_source_snapshot "$REPOSITORY" "$WSLDB_SOURCE_COMMIT" \
        "$WSLDB_BUILD_SOURCE" || fail "could not isolate WSLDB source"
fi
readonly WSLDB_BUILD_SOURCE
readonly WSLDB_BUILD_SNAPSHOT

record_command() {
    local label="$1"
    shift
    printf '# %s\n' "$label" >>"$OUTPUT/commands.log"
    printf '%q ' "$@" >>"$OUTPUT/commands.log"
    printf '\n' >>"$OUTPUT/commands.log"
}

run_logged() {
    local label="$1"
    shift
    record_command "$label" "$@"
    printf 'running %s\n' "$label"
    if [[ "$(uname -s)" == Darwin ]]; then
        /usr/bin/time -l -o "$OUTPUT/raw/$label.time" \
            "$@" >"$OUTPUT/raw/$label.stdout" 2>"$OUTPUT/raw/$label.stderr"
    else
        /usr/bin/time -v -o "$OUTPUT/raw/$label.time" \
            "$@" >"$OUTPUT/raw/$label.stdout" 2>"$OUTPUT/raw/$label.stderr"
    fi
}

logical_bytes() {
    local path="$1"
    if [[ "$(uname -s)" == Darwin ]]; then
        find "$path" -type f -exec stat -f '%z' {} \; | \
            awk '{ total += $1 } END { print total + 0 }'
    else
        find "$path" -type f -exec stat -c '%s' {} \; | \
            awk '{ total += $1 } END { print total + 0 }'
    fi
}

path_allocated_kib() {
    du -sk "$1" | awk '{ print $1 }'
}

record_size() {
    local engine="$1"
    local phase="$2"
    local path="$3"
    local logical
    local allocated
    logical="$(logical_bytes "$path")"
    allocated="$(path_allocated_kib "$path")"
    if [[ "$engine" == wsldb && -e "$path.wal" ]]; then
        logical=$((logical + $(logical_bytes "$path.wal")))
        allocated=$((allocated + $(path_allocated_kib "$path.wal")))
    fi
    printf '%s,%s,%s,%s\n' "$engine" "$phase" "$logical" \
        "$allocated" >>"$OUTPUT/sizes.csv"
}

readonly SUSTAINED_RUNNER_SHA256="$(
    benchmark_sha256_file "$SCRIPT_DIR/run-sustained-comparison.sh"
)"
readonly BENCHMARK_CONTRACT_SHA256="$(
    benchmark_sha256_file "$SCRIPT_DIR/benchmark-contract.sh"
)"
readonly CHECKPOINT_WINDOW_REPORT_SHA256="$(
    benchmark_sha256_file "$SCRIPT_DIR/checkpoint-window-report.sh"
)"
readonly EVIDENCE_HASH_WRITER_SHA256="$(
    benchmark_sha256_file "$REPOSITORY/scripts/write-evidence-hashes.sh"
)"
readonly EVIDENCE_HASH_VERIFIER_SHA256="$(
    benchmark_sha256_file "$REPOSITORY/scripts/verify-evidence-hashes.sh"
)"
readonly ROCKSDB_BENCH_SHA256="$(benchmark_sha256_file "$ROCKSDB_BENCH")"

{
    printf 'wsldb_commit=%s\n' "$WSLDB_SOURCE_COMMIT"
    printf 'wsldb_build_snapshot=%s\n' "$WSLDB_BUILD_SNAPSHOT"
    printf 'sustained_runner_sha256=%s\n' "$SUSTAINED_RUNNER_SHA256"
    printf 'benchmark_contract_sha256=%s\n' "$BENCHMARK_CONTRACT_SHA256"
    printf 'checkpoint_window_report_sha256=%s\n' "$CHECKPOINT_WINDOW_REPORT_SHA256"
    printf 'evidence_hash_writer_sha256=%s\n' "$EVIDENCE_HASH_WRITER_SHA256"
    printf 'evidence_hash_verifier_sha256=%s\n' "$EVIDENCE_HASH_VERIFIER_SHA256"
    printf 'wsldb_status_begin\n'
    git -C "$REPOSITORY" status --porcelain=v1
    printf 'wsldb_status_end\n'
    printf 'benchmark_release_mode=%s\n' "$RELEASE_MODE"
    if [[ "$RELEASE_MODE" == 1 ]]; then
        printf 'source_contract_satisfied=yes\n'
    else
        printf 'source_contract_satisfied=no\n'
    fi
    printf 'release_gate_complete=no\n'
    printf 'rocksdb_expected_version=%s\n' "$BENCHMARK_ROCKSDB_VERSION"
    printf 'rocksdb_expected_commit=%s\n' "$BENCHMARK_ROCKSDB_COMMIT"
    printf 'rocksdb_commit=%s\n' "$(git -C "$ROCKSDB_SOURCE" rev-parse HEAD)"
    printf 'rocksdb_status_begin\n'
    git -C "$ROCKSDB_SOURCE" status --porcelain=v1
    printf 'rocksdb_status_end\n'
    printf 'rocksdb_benchmark=%s\n' "$ROCKSDB_BENCH"
    printf 'rocksdb_benchmark_sha256=%s\n' "$ROCKSDB_BENCH_SHA256"
    "$ROCKSDB_BENCH" --version
    printf 'entries=%s\nvalue_bytes=%s\nduration_seconds=%s\n' \
        "$ENTRIES" "$VALUE_BYTES" "$DURATION_SECONDS"
    printf 'cache_bytes=%s\ncompact_bytes=%s\nmaintenance_seconds=%s\n' \
        "$CACHE_BYTES" "$COMPACT_BYTES" "$MAINTENANCE_SECONDS"
    printf 'write_buffer_bytes=%s\nzipf_exponent=%s\ndistributions=%s\n' \
        "$WRITE_BUFFER_BYTES" "$ZIPF_EXPONENT" "$DISTRIBUTIONS"
    printf 'rocksdb_write_buffer_bytes=%s\nrocksdb_compression=none\n' \
        "$WRITE_BUFFER_BYTES"
    printf 'operation_limit=%s\n' "$OPERATION_LIMIT"
    printf 'seed=%s\nrocksdb_wal_stats_enabled=%s\n' \
        "$SEED" "$ROCKSDB_WAL_STATS_ENABLED"
    printf 'scratch_parent=%s\n' "$SCRATCH_PARENT"
    printf 'warmup_runs=1\nmeasured_runs=%s\n' "$MEASURED_RUNS"
    printf 'system_begin\n'
    uname -a
    rustc -Vv
    cargo -V
    df -h "$SCRATCH"
    mount
    if [[ "$(uname -s)" == Darwin ]]; then
        sw_vers
        sysctl -n machdep.cpu.brand_string hw.memsize
    else
        command -v lscpu >/dev/null && lscpu
        command -v free >/dev/null && free -b
        command -v lsblk >/dev/null && lsblk
    fi
    printf 'system_end\n'
} >"$OUTPUT/manifest.txt" 2>&1

printf 'engine,phase,logical_bytes,allocated_kib\n' >"$OUTPUT/sizes.csv"
printf '%s\n' \
    'workload,run,window,checkpoint_activity,checkpoint_starts,checkpoint_operations,checkpoint_failures,checkpoint_bytes,checkpoints_in_progress,overwrite_operations,overwrite_p99_ns,delete_operations,delete_p99_ns,total_operations,throughput_ops_per_second' \
    >"$OUTPUT/checkpoint-windows.csv"
: >"$OUTPUT/commands.log"

if [[ -n "${WSLDB_BENCH_BIN:-}" ]]; then
    WSLDB_BENCH_BIN="$WSLDB_BENCH_BIN"
else
    record_command build-wsldb cargo build --locked \
        --manifest-path "$WSLDB_BUILD_SOURCE/Cargo.toml" --release --bench wsldb \
        --message-format=json-render-diagnostics
    cargo build --locked --manifest-path "$WSLDB_BUILD_SOURCE/Cargo.toml" \
        --release --bench wsldb \
        --message-format=json-render-diagnostics >"$OUTPUT/build.json" \
        2>"$OUTPUT/build.stderr"
    WSLDB_BENCH_BIN="$(
        sed -n '/"kind":\["bench"\]/s/.*"executable":"\([^"]*\)".*/\1/p' \
            "$OUTPUT/build.json" | tail -n 1
    )"
fi
readonly WSLDB_BENCH_BIN
[[ -x "$WSLDB_BENCH_BIN" ]] || fail "could not resolve WSLDB benchmark executable"
readonly WSLDB_BENCH_BIN_SHA256="$(benchmark_sha256_file "$WSLDB_BENCH_BIN")"
printf 'wsldb_benchmark=%s\n' "$WSLDB_BENCH_BIN" >>"$OUTPUT/manifest.txt"
printf 'wsldb_benchmark_sha256=%s\n' \
    "$WSLDB_BENCH_BIN_SHA256" >>"$OUTPUT/manifest.txt"

run_wsldb_load() {
    local label="$1"
    local database="$2"
    run_logged "$label" env -u WSLDB_BENCH_DRAIN -u WSLDB_BENCH_OPERATION_LIMIT \
        WSLDB_RUN_BENCHMARK=1 WSLDB_BENCH_WORKLOAD=load \
        WSLDB_BENCH_DATABASE="$database" WSLDB_BENCH_VALUE_BYTES="$VALUE_BYTES" \
        WSLDB_BENCH_WRITE_BUFFER_BYTES=0 \
        "$WSLDB_BENCH_BIN" "$ENTRIES" "$ENTRIES" 10000 "$CACHE_PAGES"
}

run_wsldb_sustained() {
    local label="$1"
    local mode="$2"
    local database="$3"
    local environment=(
        env -u WSLDB_BENCH_DRAIN -u WSLDB_BENCH_OPERATION_LIMIT
        WSLDB_RUN_BENCHMARK=1 WSLDB_BENCH_WORKLOAD="$mode"
        WSLDB_BENCH_DATABASE="$database" WSLDB_BENCH_VALUE_BYTES="$VALUE_BYTES"
        WSLDB_BENCH_DURATION_SECONDS="$DURATION_SECONDS"
        WSLDB_BENCH_COMPACT_BYTES="$COMPACT_BYTES"
        WSLDB_BENCH_MAINTENANCE_SECONDS="$MAINTENANCE_SECONDS"
        WSLDB_BENCH_ZIPF_EXPONENT="$ZIPF_EXPONENT"
        WSLDB_BENCH_WRITE_BUFFER_BYTES="$WRITE_BUFFER_BYTES"
    )
    if (( OPERATION_LIMIT > 0 )); then
        environment+=(WSLDB_BENCH_OPERATION_LIMIT="$OPERATION_LIMIT")
    fi
    run_logged "$label" "${environment[@]}" \
        "$WSLDB_BENCH_BIN" "$ENTRIES" "$ENTRIES" 10000 "$CACHE_PAGES"
}

run_rocks() {
    local label="$1"
    local mode="$2"
    local database="$3"
    local distribution="$4"
    local environment=(
        env -u WSLDB_BENCH_OPERATION_LIMIT -u WSLDB_BENCH_WRITE_BUFFER_BYTES
        -u WSLDB_ROCKSDB_WAL_STATS
        WSLDB_BENCH_WRITE_BUFFER_BYTES="$WRITE_BUFFER_BYTES"
    )
    if (( ROCKSDB_WAL_STATS_ENABLED == 1 )); then
        environment+=(WSLDB_ROCKSDB_WAL_STATS=1)
    fi
    if (( OPERATION_LIMIT > 0 )) && [[ "$mode" == run ]]; then
        environment+=(WSLDB_BENCH_OPERATION_LIMIT="$OPERATION_LIMIT")
    fi
    run_logged "$label" "${environment[@]}" \
        "$ROCKSDB_BENCH" "$mode" "$database" "$ENTRIES" \
        "$VALUE_BYTES" "$DURATION_SECONDS" "$CACHE_BYTES" \
        "$MAINTENANCE_SECONDS" "$distribution" "$ZIPF_EXPONENT"
}

record_field() {
    local file="$1"
    local record="$2"
    local field="$3"
    awk -v record="$record" -v prefix="$field=" '
        $1 == record {
            records += 1
            for (field_index = 1; field_index <= NF; field_index++) {
                if (index($field_index, prefix) == 1) {
                    fields += 1
                    value = substr($field_index, length(prefix) + 1)
                }
            }
        }
        END {
            if (records != 1 || fields != 1 || value == "") {
                exit 1
            }
            print value
        }
    ' "$file"
}

verify_load_result() {
    local workload="$1"
    local run="$2"
    local wsldb_log="$OUTPUT/raw/setup-wsldb-$workload-r$run.stderr"
    local rocksdb_log="$OUTPUT/raw/setup-rocksdb-$workload-r$run.stderr"
    local wsldb_entries
    local rocksdb_entries
    local wsldb_hash
    local rocksdb_hash
    wsldb_entries="$(record_field "$wsldb_log" load_complete entries)" || \
        fail "WSLDB load completion record is missing or ambiguous"
    rocksdb_entries="$(record_field "$rocksdb_log" load_complete entries)" || \
        fail "RocksDB load completion record is missing or ambiguous"
    wsldb_hash="$(record_field "$wsldb_log" load_complete logical_hash)" || \
        fail "WSLDB load hash is missing or ambiguous"
    rocksdb_hash="$(record_field "$rocksdb_log" load_complete logical_hash)" || \
        fail "RocksDB load hash is missing or ambiguous"
    [[ "$wsldb_entries" == "$ENTRIES" ]] || fail "WSLDB load entry count is incorrect"
    [[ "$rocksdb_entries" == "$ENTRIES" ]] || fail "RocksDB load entry count is incorrect"
    [[ "$wsldb_hash" =~ ^[0-9a-f]{64}$ ]] || fail "WSLDB load hash is malformed"
    [[ "$rocksdb_hash" =~ ^[0-9a-f]{64}$ ]] || fail "RocksDB load hash is malformed"
    [[ "$wsldb_hash" == "$rocksdb_hash" ]] || fail "loaded logical datasets disagree"
}

verify_sustained_result() {
    local workload="$1"
    local run="$2"
    local wsldb_log="$OUTPUT/raw/$workload-wsldb-r$run.stderr"
    local rocksdb_log="$OUTPUT/raw/$workload-rocksdb-r$run.stderr"
    local wsldb_operations
    local rocksdb_operations
    local wsldb_entries
    local rocksdb_entries
    local wsldb_hash
    local rocksdb_hash
    wsldb_operations="$(record_field "$wsldb_log" sustained_complete operations)" || \
        fail "WSLDB sustained completion record is missing or ambiguous"
    rocksdb_operations="$(record_field "$rocksdb_log" sustained_complete operations)" || \
        fail "RocksDB sustained completion record is missing or ambiguous"
    wsldb_entries="$(record_field "$wsldb_log" sustained_complete entries)" || \
        fail "WSLDB sustained entry count is missing or ambiguous"
    rocksdb_entries="$(record_field "$rocksdb_log" sustained_complete entries)" || \
        fail "RocksDB sustained entry count is missing or ambiguous"
    wsldb_hash="$(record_field "$wsldb_log" sustained_complete logical_hash)" || \
        fail "WSLDB sustained hash is missing or ambiguous"
    rocksdb_hash="$(record_field "$rocksdb_log" sustained_complete logical_hash)" || \
        fail "RocksDB sustained hash is missing or ambiguous"
    [[ "$wsldb_operations" =~ ^[1-9][0-9]*$ ]] || fail "WSLDB operation count is malformed"
    [[ "$rocksdb_operations" =~ ^[1-9][0-9]*$ ]] || fail "RocksDB operation count is malformed"
    [[ "$wsldb_entries" == "$ENTRIES" ]] || fail "WSLDB final entry count is incorrect"
    [[ "$rocksdb_entries" == "$ENTRIES" ]] || fail "RocksDB final entry count is incorrect"
    [[ "$wsldb_hash" =~ ^[0-9a-f]{64}$ ]] || fail "WSLDB final hash is malformed"
    [[ "$rocksdb_hash" =~ ^[0-9a-f]{64}$ ]] || fail "RocksDB final hash is malformed"
    if (( OPERATION_LIMIT > 0 )); then
        [[ "$wsldb_operations" == "$OPERATION_LIMIT" ]] || fail "WSLDB stopped before the operation limit"
        [[ "$rocksdb_operations" == "$OPERATION_LIMIT" ]] || fail "RocksDB stopped before the operation limit"
        [[ "$wsldb_hash" == "$rocksdb_hash" ]] || fail "final logical hashes disagree"
    fi
}

record_checkpoint_windows() {
    local workload="$1"
    local run="$2"
    "$SCRIPT_DIR/checkpoint-window-report.sh" \
        "$OUTPUT/raw/$workload-wsldb-r$run.stdout" \
        "$OUTPUT/raw/$workload-wsldb-r$run.stderr" \
        "$workload" "$run" >>"$OUTPUT/checkpoint-windows.csv" || \
        fail "WSLDB checkpoint window records are missing or malformed"
}

distribution_index=0
for distribution in $DISTRIBUTIONS; do
    if [[ "$distribution" == uniform ]]; then
        workload=w5
        wsldb_mode=sustained
    else
        workload=w6
        wsldb_mode=skewed
    fi
    for ((run = 0; run <= MEASURED_RUNS; run++)); do
        wsldb_file="$SCRATCH/$workload-r$run.wsldb"
        rocksdb_dir="$SCRATCH/$workload-r$run.rocksdb"
        run_wsldb_load "setup-wsldb-$workload-r$run" "$wsldb_file"
        run_rocks "setup-rocksdb-$workload-r$run" load "$rocksdb_dir" "$distribution"
        verify_load_result "$workload" "$run"
        record_size wsldb "setup-$workload-r$run" "$wsldb_file"
        record_size rocksdb "setup-$workload-r$run" "$rocksdb_dir"
        if (( (run + distribution_index) % 2 == 0 )); then
            run_wsldb_sustained "$workload-wsldb-r$run" "$wsldb_mode" "$wsldb_file"
            run_rocks "$workload-rocksdb-r$run" run "$rocksdb_dir" "$distribution"
        else
            run_rocks "$workload-rocksdb-r$run" run "$rocksdb_dir" "$distribution"
            run_wsldb_sustained "$workload-wsldb-r$run" "$wsldb_mode" "$wsldb_file"
        fi
        record_size wsldb "final-$workload-r$run" "$wsldb_file"
        record_size rocksdb "final-$workload-r$run" "$rocksdb_dir"
        verify_sustained_result "$workload" "$run"
        record_checkpoint_windows "$workload" "$run"
        remove_scratch_path "$wsldb_file"
        remove_scratch_path "$wsldb_file.lock"
        if [[ -e "$wsldb_file.wal" ]]; then
            remove_scratch_path "$wsldb_file.wal"
        fi
        remove_scratch_path "$rocksdb_dir"
    done
    distribution_index=$((distribution_index + 1))
done

benchmark_validate_source_snapshot "$RELEASE_MODE" "$REPOSITORY" \
    "$WSLDB_SOURCE_COMMIT" WSLDB || fail "WSLDB source snapshot changed"
benchmark_validate_source_snapshot "$RELEASE_MODE" "$WSLDB_BUILD_SOURCE" \
    "$WSLDB_SOURCE_COMMIT" "isolated WSLDB build" || \
    fail "isolated WSLDB build snapshot changed"
benchmark_validate_source_snapshot "$RELEASE_MODE" "$ROCKSDB_SOURCE" \
    "$BENCHMARK_ROCKSDB_COMMIT" RocksDB || fail "RocksDB source snapshot changed"
benchmark_validate_file_snapshot "$SCRIPT_DIR/run-sustained-comparison.sh" \
    "$SUSTAINED_RUNNER_SHA256" "sustained runner" || fail "runner changed"
benchmark_validate_file_snapshot "$SCRIPT_DIR/benchmark-contract.sh" \
    "$BENCHMARK_CONTRACT_SHA256" "benchmark contract" || fail "contract changed"
benchmark_validate_file_snapshot "$SCRIPT_DIR/checkpoint-window-report.sh" \
    "$CHECKPOINT_WINDOW_REPORT_SHA256" "checkpoint window report" || \
    fail "checkpoint window report changed"
benchmark_validate_file_snapshot "$REPOSITORY/scripts/write-evidence-hashes.sh" \
    "$EVIDENCE_HASH_WRITER_SHA256" "evidence hash writer" || fail "writer changed"
benchmark_validate_file_snapshot "$REPOSITORY/scripts/verify-evidence-hashes.sh" \
    "$EVIDENCE_HASH_VERIFIER_SHA256" "evidence hash verifier" || fail "verifier changed"
benchmark_validate_file_snapshot "$ROCKSDB_BENCH" "$ROCKSDB_BENCH_SHA256" \
    "RocksDB benchmark" || fail "RocksDB benchmark changed"
benchmark_validate_file_snapshot "$WSLDB_BENCH_BIN" "$WSLDB_BENCH_BIN_SHA256" \
    "WSLDB benchmark" || fail "WSLDB benchmark changed"
printf 'runner_inputs_revalidated=yes\n' >>"$OUTPUT/manifest.txt"
printf 'runner_complete=yes\nevidence_hashes_file=evidence-sha256.txt\n' \
    >>"$OUTPUT/manifest.txt"
"$REPOSITORY/scripts/write-evidence-hashes.sh" "$OUTPUT"
if ! "$REPOSITORY/scripts/verify-evidence-hashes.sh" "$OUTPUT"; then
    rm -f -- "$OUTPUT/evidence-sha256.txt"
    fail "sealed evidence failed self-verification; seal removed"
fi
printf 'sustained comparison completed; raw evidence: %s\n' "$OUTPUT"

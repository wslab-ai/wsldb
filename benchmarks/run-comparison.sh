#!/usr/bin/env bash

set -euo pipefail

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
readonly REPOSITORY="$(cd "$SCRIPT_DIR/.." && pwd -P)"
source "$SCRIPT_DIR/benchmark-contract.sh"
readonly SEED=5573576672346087795
readonly MEASURED_RUNS=5
readonly ESTIMATED_ENTRIES_PER_LEAF=32
readonly MAX_RANGE_WARMUP_OPERATIONS=1000000

fail() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

if [[ $# -ne 2 ]]; then
    fail "usage: $0 /path/to/rocksdb/db_bench /new/output/directory"
fi

readonly ROCKSDB_BENCH="$1"
readonly REQUESTED_OUTPUT="$2"
[[ -x "$ROCKSDB_BENCH" ]] || fail "db_bench is not executable: $ROCKSDB_BENCH"
readonly ROCKSDB_OPEN_BENCH="${WSLDB_ROCKSDB_OPEN_BENCH:-}"
if [[ -n "$ROCKSDB_OPEN_BENCH" ]]; then
    [[ -x "$ROCKSDB_OPEN_BENCH" ]] || \
        fail "RocksDB open benchmark is not executable: $ROCKSDB_OPEN_BENCH"
fi
[[ ! -e "$REQUESTED_OUTPUT" ]] || fail "output already exists: $REQUESTED_OUTPUT"

readonly ENTRIES="${WSLDB_COMPARISON_ENTRIES:-1000000}"
readonly OPERATIONS="${WSLDB_COMPARISON_OPERATIONS:-1000000}"
readonly RANGE_OPERATIONS="${WSLDB_COMPARISON_RANGE_OPERATIONS:-10000}"
readonly CACHE_BYTES="${WSLDB_COMPARISON_CACHE_BYTES:-268435456}"
readonly LOADS="${WSLDB_COMPARISON_LOADS:-sequential random}"
readonly BATCHES="${WSLDB_COMPARISON_BATCHES:-1 100 10000}"
readonly THREADS="${WSLDB_COMPARISON_THREADS:-1 4 16 64}"
readonly RANGES="${WSLDB_COMPARISON_RANGES:-10 100 10000}"
readonly VALUES="${WSLDB_COMPARISON_VALUES:-100 4096}"
readonly PHASES="${WSLDB_COMPARISON_PHASES:-loads reads ranges opens}"
readonly WRITE_BUFFER_BYTES="${WSLDB_COMPARISON_WRITE_BUFFER_BYTES:-0}"
readonly CACHE_STATE="${WSLDB_COMPARISON_CACHE_STATE:-warm}"
readonly COLD_CACHE_COMMAND="${WSLDB_COLD_CACHE_COMMAND:-}"
readonly CACHE_PAGES=$((CACHE_BYTES / 4096))

phase_enabled() {
    local requested="$1"
    local phase
    for phase in $PHASES; do
        if [[ "$phase" == "$requested" ]]; then
            return 0
        fi
    done
    return 1
}

for number in "$ENTRIES" "$OPERATIONS" "$RANGE_OPERATIONS" "$CACHE_BYTES"; do
    [[ "$number" =~ ^[1-9][0-9]*$ ]] || fail "numeric settings must be positive integers"
done
[[ "$WRITE_BUFFER_BYTES" =~ ^[0-9]+$ ]] || \
    fail "write-buffer bytes must be a non-negative integer"
[[ "$CACHE_STATE" == warm || "$CACHE_STATE" == cold ]] || \
    fail "cache state must be warm or cold"
if [[ "$CACHE_STATE" == cold ]]; then
    [[ -x "$COLD_CACHE_COMMAND" ]] || \
        fail "cold-cache runs require executable WSLDB_COLD_CACHE_COMMAND"
fi
(( CACHE_PAGES > 0 )) || fail "cache must hold at least one 4096-byte page"
for number in $BATCHES $RANGES $VALUES; do
    [[ "$number" =~ ^[1-9][0-9]*$ ]] || fail "workload lists must contain positive integers"
done
if phase_enabled reads; then
    for threads in $THREADS; do
        [[ "$threads" =~ ^[1-9][0-9]*$ ]] || fail "thread counts must be positive integers"
        (( OPERATIONS % threads == 0 )) || \
            fail "operations must be divisible by every thread count"
    done
fi
for load in $LOADS; do
    [[ "$load" == sequential || "$load" == random ]] || \
        fail "load list must contain sequential and/or random"
done
for phase in $PHASES; do
    case "$phase" in
        loads | reads | ranges | opens) ;;
        *) fail "phases must contain loads, reads, ranges, and/or opens" ;;
    esac
done
if [[ "$CACHE_STATE" == cold ]] && \
    ! phase_enabled reads && ! phase_enabled ranges && ! phase_enabled opens; then
    fail "cold-cache mode requires reads, ranges, or opens"
fi
readonly SETUP_BATCH=$((ENTRIES < 10000 ? ENTRIES : 10000))

release_mode="$(benchmark_contract_mode)" || fail "invalid benchmark contract mode"
readonly RELEASE_MODE="$release_mode"
if [[ "$RELEASE_MODE" == 1 && -n "${WSLDB_BENCH_BIN:-}" ]]; then
    fail "release mode builds WSLDB from the recorded clean checkout"
fi
if [[ "$RELEASE_MODE" == 1 && "$CACHE_STATE" == cold && "$COLD_CACHE_COMMAND" != /* ]]; then
    fail "release-mode cold-cache command must be an absolute path"
fi
rocksdb_source="$(benchmark_resolve_rocksdb_source \
    "$ROCKSDB_BENCH" "${WSLDB_ROCKSDB_SOURCE:-}")"
readonly ROCKSDB_SOURCE="$rocksdb_source"
readonly WSLDB_SOURCE_COMMIT="$(git -C "$REPOSITORY" rev-parse HEAD)"
benchmark_validate_release_inputs "$RELEASE_MODE" "$REPOSITORY" \
    "$ROCKSDB_SOURCE" "$ROCKSDB_BENCH" "$BENCHMARK_ROCKSDB_COMMIT" \
    "db_bench version $BENCHMARK_ROCKSDB_VERSION" || \
    fail "release benchmark inputs do not satisfy the frozen contract"
if [[ "$RELEASE_MODE" == 1 ]] && phase_enabled opens; then
    [[ -n "$ROCKSDB_OPEN_BENCH" ]] || \
        fail "release-mode W7 requires WSLDB_ROCKSDB_OPEN_BENCH"
    benchmark_validate_binary_version "$ROCKSDB_OPEN_BENCH" \
        "rocksdb_open_bench version $BENCHMARK_ROCKSDB_VERSION" || \
        fail "RocksDB open adapter does not use the pinned version"
fi

mkdir -p "$REQUESTED_OUTPUT"
readonly OUTPUT="$(cd "$REQUESTED_OUTPUT" && pwd -P)"
mkdir -p "$OUTPUT/raw"

readonly SCRATCH_PARENT="${TMPDIR:-/tmp}"
SCRATCH="$(mktemp -d "$SCRATCH_PARENT/wsldb-comparison.XXXXXX")"
readonly SCRATCH

remove_scratch_path() {
    local target="$1"
    case "$target" in
        "$SCRATCH"/*) rm -rf -- "$target" ;;
        *) fail "refusing to remove a path outside benchmark scratch space: $target" ;;
    esac
}

remove_wsldb_database() {
    local database="$1"
    remove_scratch_path "$database"
    remove_scratch_path "$database.lock"
    if [[ -e "$database.wal" ]]; then
        remove_scratch_path "$database.wal"
    fi
}

cleanup() {
    case "$SCRATCH" in
        "$SCRATCH_PARENT"/wsldb-comparison.*) rm -rf -- "$SCRATCH" ;;
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
    printf '# %s\n' "$label" >> "$OUTPUT/commands.log"
    printf '%q ' "$@" >> "$OUTPUT/commands.log"
    printf '\n' >> "$OUTPUT/commands.log"
}

run_logged() {
    local label="$1"
    shift
    record_command "$label" "$@"
    printf 'running %s\n' "$label"
    if [[ "$(uname -s)" == Darwin ]]; then
        /usr/bin/time -l -o "$OUTPUT/raw/$label.time" \
            "$@" > "$OUTPUT/raw/$label.stdout" 2> "$OUTPUT/raw/$label.stderr"
    else
        /usr/bin/time -v -o "$OUTPUT/raw/$label.time" \
            "$@" > "$OUTPUT/raw/$label.stdout" 2> "$OUTPUT/raw/$label.stderr"
    fi
}

reset_cache() {
    local label="$1"
    local database="$2"
    [[ "$CACHE_STATE" == cold ]] || return 0
    record_command "cache-reset-$label" "$COLD_CACHE_COMMAND" "$database" "$label"
    "$COLD_CACHE_COMMAND" "$database" "$label" \
        >"$OUTPUT/raw/cache-reset-$label.stdout" \
        2>"$OUTPUT/raw/cache-reset-$label.stderr"
}

logical_bytes() {
    local path="$1"
    if [[ "$(uname -s)" == Darwin ]]; then
        find "$path" -type f -exec stat -f '%z' {} \; | awk '{ total += $1 } END { print total + 0 }'
    else
        find "$path" -type f -exec stat -c '%s' {} \; | awk '{ total += $1 } END { print total + 0 }'
    fi
}

record_size() {
    local engine="$1"
    local workload="$2"
    local path="$3"
    local logical
    local allocated_kib
    logical="$(logical_bytes "$path")"
    allocated_kib="$(du -sk "$path" | awk '{ print $1 }')"
    if [[ "$engine" == wsldb && -e "$path.wal" ]]; then
        logical=$((logical + $(logical_bytes "$path.wal")))
        allocated_kib=$((allocated_kib + $(du -sk "$path.wal" | awk '{ print $1 }')))
    fi
    printf '%s,%s,%s,%s\n' \
        "$engine" "$workload" "$logical" "$allocated_kib" >> "$OUTPUT/sizes.csv"
}

repeat_benchmark() {
    local name="$1"
    local repeated="$name"
    local run
    for ((run = 0; run < MEASURED_RUNS; run++)); do
        repeated="$repeated,$name"
    done
    printf '%s' "$repeated"
}

repeat_measured_benchmark() {
    local name="$1"
    local repeated="$name"
    local run
    for ((run = 1; run < MEASURED_RUNS; run++)); do
        repeated="$repeated,$name"
    done
    printf '%s' "$repeated"
}

range_warmup_operations() {
    local range_size="$1"
    local touched_entries=$((CACHE_PAGES * 5 * ESTIMATED_ENTRIES_PER_LEAF))
    local warmup=$(((touched_entries + range_size - 1) / range_size))
    (( warmup > MAX_RANGE_WARMUP_OPERATIONS )) && warmup=$MAX_RANGE_WARMUP_OPERATIONS
    (( warmup < RANGE_OPERATIONS )) && warmup=$RANGE_OPERATIONS
    printf '%s' "$warmup"
}

readonly COMPARISON_RUNNER_SHA256="$(
    benchmark_sha256_file "$SCRIPT_DIR/run-comparison.sh"
)"
readonly BENCHMARK_CONTRACT_SHA256="$(
    benchmark_sha256_file "$SCRIPT_DIR/benchmark-contract.sh"
)"
readonly EVIDENCE_HASH_WRITER_SHA256="$(
    benchmark_sha256_file "$REPOSITORY/scripts/write-evidence-hashes.sh"
)"
readonly EVIDENCE_HASH_VERIFIER_SHA256="$(
    benchmark_sha256_file "$REPOSITORY/scripts/verify-evidence-hashes.sh"
)"
readonly ROCKSDB_BENCH_SHA256="$(benchmark_sha256_file "$ROCKSDB_BENCH")"
ROCKSDB_OPEN_BENCH_SHA256=
if [[ -n "$ROCKSDB_OPEN_BENCH" ]]; then
    ROCKSDB_OPEN_BENCH_SHA256="$(benchmark_sha256_file "$ROCKSDB_OPEN_BENCH")"
fi
readonly ROCKSDB_OPEN_BENCH_SHA256
COLD_CACHE_COMMAND_SHA256=
if [[ -n "$COLD_CACHE_COMMAND" ]]; then
    COLD_CACHE_COMMAND_SHA256="$(benchmark_sha256_file "$COLD_CACHE_COMMAND")"
fi
readonly COLD_CACHE_COMMAND_SHA256

{
    printf 'wsldb_commit=%s\n' "$WSLDB_SOURCE_COMMIT"
    printf 'wsldb_build_snapshot=%s\n' "$WSLDB_BUILD_SNAPSHOT"
    printf 'comparison_runner_sha256=%s\n' "$COMPARISON_RUNNER_SHA256"
    printf 'benchmark_contract_sha256=%s\n' "$BENCHMARK_CONTRACT_SHA256"
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
    printf 'rocksdb_source=%s\n' "${ROCKSDB_SOURCE:-unavailable}"
    printf 'rocksdb_binary=%s\n' "$ROCKSDB_BENCH"
    printf 'rocksdb_binary_sha256=%s\n' "$ROCKSDB_BENCH_SHA256"
    printf 'rocksdb_open_benchmark=%s\n' "${ROCKSDB_OPEN_BENCH:-unavailable}"
    if [[ -n "$ROCKSDB_OPEN_BENCH" ]]; then
        printf 'rocksdb_open_benchmark_sha256=%s\n' "$ROCKSDB_OPEN_BENCH_SHA256"
        "$ROCKSDB_OPEN_BENCH" --version
    fi
    "$ROCKSDB_BENCH" --version
    if [[ -n "$ROCKSDB_SOURCE" ]]; then
        printf 'rocksdb_commit=%s\n' "$(git -C "$ROCKSDB_SOURCE" rev-parse HEAD)"
        printf 'rocksdb_status_begin\n'
        git -C "$ROCKSDB_SOURCE" status --porcelain=v1
        printf 'rocksdb_status_end\n'
    fi
    printf 'entries=%s\noperations=%s\nrange_operations=%s\nsetup_batch=%s\n' \
        "$ENTRIES" "$OPERATIONS" "$RANGE_OPERATIONS" "$SETUP_BATCH"
    printf 'cache_bytes=%s\nloads=%s\nbatches=%s\nthreads=%s\nranges=%s\nvalues=%s\n' \
        "$CACHE_BYTES" "$LOADS" "$BATCHES" "$THREADS" "$RANGES" "$VALUES"
    printf 'phases=%s\nwrite_buffer_bytes=%s\n' "$PHASES" "$WRITE_BUFFER_BYTES"
    printf 'cache_state=%s\n' "$CACHE_STATE"
    printf 'cold_cache_command=%s\n' "${COLD_CACHE_COMMAND:-unavailable}"
    if [[ -n "$COLD_CACHE_COMMAND" ]]; then
        printf 'cold_cache_command_sha256=%s\n' "$COLD_CACHE_COMMAND_SHA256"
    fi
    printf 'seed=%s\nwarmups=1\nmeasured_runs=%s\n' "$SEED" "$MEASURED_RUNS"
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
} > "$OUTPUT/manifest.txt" 2>&1

printf 'engine,workload,logical_bytes,allocated_kib\n' > "$OUTPUT/sizes.csv"
: > "$OUTPUT/commands.log"

if [[ -n "${WSLDB_BENCH_BIN:-}" ]]; then
    WSLDB_BENCH_BIN="$WSLDB_BENCH_BIN"
else
    record_command build-wsldb cargo build --locked \
        --manifest-path "$WSLDB_BUILD_SOURCE/Cargo.toml" --release --bench wsldb \
        --message-format=json-render-diagnostics
    cargo build --locked --manifest-path "$WSLDB_BUILD_SOURCE/Cargo.toml" \
        --release --bench wsldb --message-format=json-render-diagnostics \
        > "$OUTPUT/build.json" 2> "$OUTPUT/build.stderr"
    WSLDB_BENCH_BIN="$(
        sed -n '/"kind":\["bench"\]/s/.*"executable":"\([^"]*\)".*/\1/p' "$OUTPUT/build.json" | tail -n 1
    )"
fi
readonly WSLDB_BENCH_BIN
[[ -x "$WSLDB_BENCH_BIN" ]] || fail "could not resolve the WSLDB benchmark executable"
readonly WSLDB_BENCH_BIN_SHA256="$(benchmark_sha256_file "$WSLDB_BENCH_BIN")"
printf 'wsldb_benchmark_binary=%s\n' "$WSLDB_BENCH_BIN" >> "$OUTPUT/manifest.txt"
printf 'wsldb_benchmark_binary_sha256=%s\n' \
    "$WSLDB_BENCH_BIN_SHA256" >> "$OUTPUT/manifest.txt"

run_wsldb() {
    local label="$1"
    local workload="$2"
    local database="$3"
    local entries="$4"
    local operations="$5"
    local batch="$6"
    local value_bytes="$7"
    shift 7
    run_logged "$label" env \
        WSLDB_RUN_BENCHMARK=1 \
        WSLDB_BENCH_WORKLOAD="$workload" \
        WSLDB_BENCH_DATABASE="$database" \
        WSLDB_BENCH_VALUE_BYTES="$value_bytes" \
        WSLDB_BENCH_WRITE_BUFFER_BYTES="$WRITE_BUFFER_BYTES" \
        "$@" \
        "$WSLDB_BENCH_BIN" "$entries" "$operations" "$batch" "$CACHE_PAGES"
}

run_rocks_load() {
    local label="$1"
    local database="$2"
    local entries="$3"
    local batch="$4"
    local value_bytes="$5"
    local benchmark="$6"
    run_logged "$label" "$ROCKSDB_BENCH" \
        --db="$database" \
        --benchmarks="$benchmark,flush,waitforcompaction" \
        --num="$entries" \
        --batch_size="$batch" \
        --key_size=16 \
        --value_size="$value_bytes" \
        --sync=true \
        --disable_wal=false \
        --compression_type=none \
        --compression_ratio=1.0 \
        --cache_size="$CACHE_BYTES" \
        --seed="$SEED" \
        --histogram=true
}

run_rocks_reads() {
    local label="$1"
    local database="$2"
    local value_bytes="$3"
    local threads="$4"
    local seed="$5"
    local benchmarks="$6"
    run_logged "$label" "$ROCKSDB_BENCH" \
        --db="$database" \
        --use_existing_db=true \
        --benchmarks="$benchmarks" \
        --num="$((ENTRIES * 2))" \
        --reads="$((OPERATIONS / threads))" \
        --threads="$threads" \
        --key_size=16 \
        --value_size="$value_bytes" \
        --compression_type=none \
        --compression_ratio=1.0 \
        --cache_size="$CACHE_BYTES" \
        --seed="$seed" \
        --histogram=true
}

run_rocks_ranges() {
    local label="$1"
    local database="$2"
    local value_bytes="$3"
    local range_size="$4"
    local reads="$5"
    local seed="$6"
    local benchmarks="$7"
    run_logged "$label" "$ROCKSDB_BENCH" \
        --db="$database" \
        --use_existing_db=true \
        --benchmarks="$benchmarks" \
        --num="$ENTRIES" \
        --reads="$reads" \
        --seek_nexts="$((range_size - 1))" \
        --threads=1 \
        --key_size=16 \
        --value_size="$value_bytes" \
        --compression_type=none \
        --compression_ratio=1.0 \
        --cache_size="$CACHE_BYTES" \
        --seed="$seed" \
        --histogram=true
}

run_rocks_open() {
    local label="$1"
    local database="$2"
    local value_bytes="$3"
    if [[ -n "$ROCKSDB_OPEN_BENCH" ]]; then
        run_logged "$label" "$ROCKSDB_OPEN_BENCH" "$database" 16 "$CACHE_BYTES"
    else
        run_logged "$label" "$ROCKSDB_BENCH" \
            --db="$database" \
            --use_existing_db=true \
            --benchmarks=readseq \
            --num="$ENTRIES" \
            --reads=1 \
            --threads=1 \
            --key_size=16 \
            --value_size="$value_bytes" \
            --compression_type=none \
            --compression_ratio=1.0 \
            --cache_size="$CACHE_BYTES" \
            --seed="$SEED" \
            --histogram=true
    fi
}

for value_bytes in $VALUES; do
    if phase_enabled loads; then
        for load in $LOADS; do
            if [[ "$load" == sequential ]]; then
                workload=w1
                wsldb_load=load
                rocksdb_load=fillseq
            else
                workload=w2
                wsldb_load=random-load
                rocksdb_load=filluniquerandom
            fi
            for batch in $BATCHES; do
                for ((run = 0; run <= MEASURED_RUNS; run++)); do
                    wsldb_file="$SCRATCH/${workload}-v${value_bytes}-b${batch}-r${run}.wsldb"
                    rocksdb_dir="$SCRATCH/${workload}-v${value_bytes}-b${batch}-r${run}.rocksdb"
                    wsldb_label="${workload}-wsldb-v${value_bytes}-b${batch}-r${run}"
                    rocksdb_label="${workload}-rocksdb-v${value_bytes}-b${batch}-r${run}"
                    if (( run % 2 == 0 )); then
                        run_wsldb "$wsldb_label" "$wsldb_load" "$wsldb_file" \
                            "$ENTRIES" "$OPERATIONS" "$batch" "$value_bytes"
                        record_size wsldb "$wsldb_label" "$wsldb_file"
                        run_rocks_load "$rocksdb_label" "$rocksdb_dir" \
                            "$ENTRIES" "$batch" "$value_bytes" "$rocksdb_load"
                        record_size rocksdb "$rocksdb_label" "$rocksdb_dir"
                    else
                        run_rocks_load "$rocksdb_label" "$rocksdb_dir" \
                            "$ENTRIES" "$batch" "$value_bytes" "$rocksdb_load"
                        record_size rocksdb "$rocksdb_label" "$rocksdb_dir"
                        run_wsldb "$wsldb_label" "$wsldb_load" "$wsldb_file" \
                            "$ENTRIES" "$OPERATIONS" "$batch" "$value_bytes"
                        record_size wsldb "$wsldb_label" "$wsldb_file"
                    fi
                    remove_wsldb_database "$wsldb_file"
                    remove_scratch_path "$rocksdb_dir"
                done
            done
        done
    fi

    if phase_enabled reads || phase_enabled ranges || phase_enabled opens; then
        wsldb_file="$SCRATCH/read-v${value_bytes}.wsldb"
        rocksdb_dir="$SCRATCH/read-v${value_bytes}.rocksdb"
        run_wsldb "setup-wsldb-v${value_bytes}" load "$wsldb_file" \
            "$ENTRIES" "$OPERATIONS" "$SETUP_BATCH" "$value_bytes"
        run_rocks_load "setup-rocksdb-v${value_bytes}" "$rocksdb_dir" \
            "$ENTRIES" "$SETUP_BATCH" "$value_bytes" fillseq
        record_size wsldb "shared-v${value_bytes}" "$wsldb_file"
        record_size rocksdb "shared-v${value_bytes}" "$rocksdb_dir"

        if phase_enabled reads; then
            for threads in $THREADS; do
                if [[ "$CACHE_STATE" == warm ]]; then
                    run_wsldb "w3-wsldb-v${value_bytes}-t${threads}" reads "$wsldb_file" \
                        "$ENTRIES" "$OPERATIONS" "$SETUP_BATCH" "$value_bytes" \
                        WSLDB_BENCH_THREADS="$threads"
                    run_rocks_reads "w3-rocksdb-v${value_bytes}-t${threads}" \
                        "$rocksdb_dir" "$value_bytes" "$threads" "$SEED" \
                        "$(repeat_benchmark readrandom)"
                else
                    for ((run = 0; run <= MEASURED_RUNS; run++)); do
                        wsldb_label="w3-wsldb-v${value_bytes}-t${threads}-r${run}"
                        rocksdb_label="w3-rocksdb-v${value_bytes}-t${threads}-r${run}"
                        if (( run % 2 == 0 )); then
                            reset_cache "$wsldb_label" "$wsldb_file"
                            run_wsldb "$wsldb_label" reads "$wsldb_file" \
                                "$ENTRIES" "$OPERATIONS" "$SETUP_BATCH" "$value_bytes" \
                                WSLDB_BENCH_THREADS="$threads" WSLDB_BENCH_WARMUP=0 \
                                WSLDB_BENCH_MEASURED_RUNS=1 WSLDB_BENCH_RUN_START="$run"
                            reset_cache "$rocksdb_label" "$rocksdb_dir"
                            run_rocks_reads "$rocksdb_label" "$rocksdb_dir" \
                                "$value_bytes" "$threads" "$((SEED + run))" readrandom
                        else
                            reset_cache "$rocksdb_label" "$rocksdb_dir"
                            run_rocks_reads "$rocksdb_label" "$rocksdb_dir" \
                                "$value_bytes" "$threads" "$((SEED + run))" readrandom
                            reset_cache "$wsldb_label" "$wsldb_file"
                            run_wsldb "$wsldb_label" reads "$wsldb_file" \
                                "$ENTRIES" "$OPERATIONS" "$SETUP_BATCH" "$value_bytes" \
                                WSLDB_BENCH_THREADS="$threads" WSLDB_BENCH_WARMUP=0 \
                                WSLDB_BENCH_MEASURED_RUNS=1 WSLDB_BENCH_RUN_START="$run"
                        fi
                    done
                fi
            done
        fi

        if phase_enabled ranges; then
            for range_size in $RANGES; do
                if [[ "$CACHE_STATE" == warm ]]; then
                    run_wsldb "w4-wsldb-v${value_bytes}-n${range_size}" ranges "$wsldb_file" \
                        "$ENTRIES" "$RANGE_OPERATIONS" "$SETUP_BATCH" "$value_bytes" \
                        WSLDB_BENCH_RANGE="$range_size"
                    range_warmup="$(range_warmup_operations "$range_size")"
                    run_rocks_ranges "w4-rocksdb-warmup-v${value_bytes}-n${range_size}" \
                        "$rocksdb_dir" "$value_bytes" "$range_size" "$range_warmup" \
                        "$SEED" seekrandom
                    run_rocks_ranges "w4-rocksdb-v${value_bytes}-n${range_size}" \
                        "$rocksdb_dir" "$value_bytes" "$range_size" "$RANGE_OPERATIONS" \
                        "$SEED" "$(repeat_measured_benchmark seekrandom)"
                else
                    for ((run = 0; run <= MEASURED_RUNS; run++)); do
                        wsldb_label="w4-wsldb-v${value_bytes}-n${range_size}-r${run}"
                        rocksdb_label="w4-rocksdb-v${value_bytes}-n${range_size}-r${run}"
                        if (( run % 2 == 0 )); then
                            reset_cache "$wsldb_label" "$wsldb_file"
                            run_wsldb "$wsldb_label" ranges "$wsldb_file" \
                                "$ENTRIES" "$RANGE_OPERATIONS" "$SETUP_BATCH" "$value_bytes" \
                                WSLDB_BENCH_RANGE="$range_size" WSLDB_BENCH_WARMUP=0 \
                                WSLDB_BENCH_MEASURED_RUNS=1 WSLDB_BENCH_RUN_START="$run"
                            reset_cache "$rocksdb_label" "$rocksdb_dir"
                            run_rocks_ranges "$rocksdb_label" "$rocksdb_dir" \
                                "$value_bytes" "$range_size" "$RANGE_OPERATIONS" \
                                "$((SEED + run))" seekrandom
                        else
                            reset_cache "$rocksdb_label" "$rocksdb_dir"
                            run_rocks_ranges "$rocksdb_label" "$rocksdb_dir" \
                                "$value_bytes" "$range_size" "$RANGE_OPERATIONS" \
                                "$((SEED + run))" seekrandom
                            reset_cache "$wsldb_label" "$wsldb_file"
                            run_wsldb "$wsldb_label" ranges "$wsldb_file" \
                                "$ENTRIES" "$RANGE_OPERATIONS" "$SETUP_BATCH" "$value_bytes" \
                                WSLDB_BENCH_RANGE="$range_size" WSLDB_BENCH_WARMUP=0 \
                                WSLDB_BENCH_MEASURED_RUNS=1 WSLDB_BENCH_RUN_START="$run"
                        fi
                    done
                fi
            done
        fi

        if phase_enabled opens; then
            for ((run = 0; run <= MEASURED_RUNS; run++)); do
                wsldb_label="w7-wsldb-v${value_bytes}-r${run}"
                rocksdb_label="w7-rocksdb-v${value_bytes}-r${run}"
                if [[ "$CACHE_STATE" == warm || $((run % 2)) == 0 ]]; then
                    reset_cache "$wsldb_label" "$wsldb_file"
                    run_wsldb "$wsldb_label" open "$wsldb_file" \
                        "$ENTRIES" 1 "$SETUP_BATCH" "$value_bytes" \
                        WSLDB_BENCH_RUN_START="$run"
                    reset_cache "$rocksdb_label" "$rocksdb_dir"
                    run_rocks_open "$rocksdb_label" "$rocksdb_dir" "$value_bytes"
                else
                    reset_cache "$rocksdb_label" "$rocksdb_dir"
                    run_rocks_open "$rocksdb_label" "$rocksdb_dir" "$value_bytes"
                    reset_cache "$wsldb_label" "$wsldb_file"
                    run_wsldb "$wsldb_label" open "$wsldb_file" \
                        "$ENTRIES" 1 "$SETUP_BATCH" "$value_bytes" \
                        WSLDB_BENCH_RUN_START="$run"
                fi
            done
        fi

        remove_wsldb_database "$wsldb_file"
        remove_scratch_path "$rocksdb_dir"
    fi
done

benchmark_validate_source_snapshot "$RELEASE_MODE" "$REPOSITORY" \
    "$WSLDB_SOURCE_COMMIT" WSLDB || fail "WSLDB source snapshot changed"
benchmark_validate_source_snapshot "$RELEASE_MODE" "$WSLDB_BUILD_SOURCE" \
    "$WSLDB_SOURCE_COMMIT" "isolated WSLDB build" || \
    fail "isolated WSLDB build snapshot changed"
benchmark_validate_source_snapshot "$RELEASE_MODE" "$ROCKSDB_SOURCE" \
    "$BENCHMARK_ROCKSDB_COMMIT" RocksDB || fail "RocksDB source snapshot changed"
benchmark_validate_file_snapshot "$SCRIPT_DIR/run-comparison.sh" \
    "$COMPARISON_RUNNER_SHA256" "comparison runner" || fail "runner changed"
benchmark_validate_file_snapshot "$SCRIPT_DIR/benchmark-contract.sh" \
    "$BENCHMARK_CONTRACT_SHA256" "benchmark contract" || fail "contract changed"
benchmark_validate_file_snapshot "$REPOSITORY/scripts/write-evidence-hashes.sh" \
    "$EVIDENCE_HASH_WRITER_SHA256" "evidence hash writer" || fail "writer changed"
benchmark_validate_file_snapshot "$REPOSITORY/scripts/verify-evidence-hashes.sh" \
    "$EVIDENCE_HASH_VERIFIER_SHA256" "evidence hash verifier" || fail "verifier changed"
benchmark_validate_file_snapshot "$ROCKSDB_BENCH" "$ROCKSDB_BENCH_SHA256" \
    "RocksDB benchmark" || fail "RocksDB benchmark changed"
benchmark_validate_file_snapshot "$WSLDB_BENCH_BIN" "$WSLDB_BENCH_BIN_SHA256" \
    "WSLDB benchmark" || fail "WSLDB benchmark changed"
if [[ -n "$ROCKSDB_OPEN_BENCH" ]]; then
    benchmark_validate_file_snapshot "$ROCKSDB_OPEN_BENCH" \
        "$ROCKSDB_OPEN_BENCH_SHA256" "RocksDB open benchmark" || \
        fail "RocksDB open benchmark changed"
fi
if [[ -n "$COLD_CACHE_COMMAND" ]]; then
    benchmark_validate_file_snapshot "$COLD_CACHE_COMMAND" \
        "$COLD_CACHE_COMMAND_SHA256" "cold-cache command" || \
        fail "cold-cache command changed"
fi
printf 'runner_inputs_revalidated=yes\n' >>"$OUTPUT/manifest.txt"
printf 'runner_complete=yes\nevidence_hashes_file=evidence-sha256.txt\n' \
    >>"$OUTPUT/manifest.txt"
"$REPOSITORY/scripts/write-evidence-hashes.sh" "$OUTPUT"
if ! "$REPOSITORY/scripts/verify-evidence-hashes.sh" "$OUTPUT"; then
    rm -f -- "$OUTPUT/evidence-sha256.txt"
    fail "sealed evidence failed self-verification; seal removed"
fi
printf 'comparison completed; raw evidence: %s\n' "$OUTPUT"

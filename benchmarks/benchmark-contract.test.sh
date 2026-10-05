#!/usr/bin/env bash

set -euo pipefail

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
source "$SCRIPT_DIR/benchmark-contract.sh"

fail() {
    printf 'benchmark contract test failed: %s\n' "$*" >&2
    exit 1
}

expect_failure() {
    if "$@" >/dev/null 2>&1; then
        fail "command unexpectedly succeeded: $*"
    fi
}

expect_evidence_hash() {
    local directory="$1"
    local relative="$2"
    local expected
    local recorded
    expected="$(benchmark_sha256_file "$directory/$relative")"
    recorded="$(awk -v relative="$relative" '
        $2 == relative { matches += 1; digest = $1 }
        END {
            if (matches != 1) exit 1
            print digest
        }
    ' "$directory/evidence-sha256.txt")" || \
        fail "evidence hash is missing or duplicated: $relative"
    [[ "$recorded" == "$expected" ]] || fail "evidence hash disagrees: $relative"
}

SCRATCH="$(mktemp -d "${TMPDIR:-/tmp}/wsldb-benchmark-contract.XXXXXX")"
readonly SCRATCH
trap 'rm -rf -- "$SCRATCH"' EXIT INT TERM

make_checkout() {
    local path=$1
    mkdir -p "$path"
    git -C "$path" init -q
    git -C "$path" config user.name "WSLDB benchmark test"
    git -C "$path" config user.email "benchmark-test@invalid.example"
    printf 'fixture\n' >"$path/fixture"
    git -C "$path" add fixture
    git -C "$path" commit -q -m fixture
}

make_checkout "$SCRATCH/wsldb"
make_checkout "$SCRATCH/rocksdb"
printf 'stable input\n' >"$SCRATCH/snapshotted-input"
snapshotted_hash="$(benchmark_sha256_file "$SCRATCH/snapshotted-input")"
benchmark_validate_file_snapshot "$SCRATCH/snapshotted-input" \
    "$snapshotted_hash" "test input"
printf 'changed input\n' >"$SCRATCH/snapshotted-input"
expect_failure benchmark_validate_file_snapshot "$SCRATCH/snapshotted-input" \
    "$snapshotted_hash" "test input"
expect_failure benchmark_validate_file_snapshot "$SCRATCH/missing-input" \
    "$snapshotted_hash" "test input"
wsldb_snapshot_commit="$(git -C "$SCRATCH/wsldb" rev-parse HEAD)"
benchmark_validate_source_snapshot 1 "$SCRATCH/wsldb" \
    "$wsldb_snapshot_commit" "test source"
printf 'dirty\n' >"$SCRATCH/wsldb/dirty"
expect_failure benchmark_validate_source_snapshot 1 "$SCRATCH/wsldb" \
    "$wsldb_snapshot_commit" "test source"
rm "$SCRATCH/wsldb/dirty"
printf 'second commit\n' >"$SCRATCH/wsldb/fixture"
git -C "$SCRATCH/wsldb" add fixture
git -C "$SCRATCH/wsldb" commit -q -m second
expect_failure benchmark_validate_source_snapshot 1 "$SCRATCH/wsldb" \
    "$wsldb_snapshot_commit" "test source"
benchmark_validate_source_snapshot 0 /missing ignored "diagnostic source"
benchmark_create_source_snapshot "$SCRATCH/wsldb" "$wsldb_snapshot_commit" \
    "$SCRATCH/source-snapshot"
benchmark_validate_source_snapshot 1 "$SCRATCH/source-snapshot" \
    "$wsldb_snapshot_commit" "isolated test source"
printf '%s\n' \
    '#!/usr/bin/env bash' \
    'if [[ "${1:-}" == --version ]]; then' \
    '    printf "db_bench version 11.8.1\\n"' \
    '    exit 0' \
    'fi' \
    'database=' \
    'for argument in "$@"; do' \
    '    case "$argument" in --db=*) database=${argument#--db=} ;; esac' \
    'done' \
    '[[ -n "$database" ]] || exit 1' \
    'if [[ " $* " == *fillseq* ]]; then' \
    '    mkdir -p "$database"' \
    '    printf "fixture\\n" >"$database/data"' \
    'fi' \
    'printf "stub metric\\n"' >"$SCRATCH/db_bench"
chmod +x "$SCRATCH/db_bench"
printf '%s\n' \
    '#!/usr/bin/env bash' \
    'database=${WSLDB_BENCH_DATABASE:?}' \
    'workload=${WSLDB_BENCH_WORKLOAD:?}' \
    'if [[ "${WSLDB_TEST_SUSTAINED_CONTRACT:-0}" == 1 ]]; then' \
    '    [[ -z "${WSLDB_BENCH_DRAIN+x}" ]] || exit 21' \
    '    if [[ "$workload" == load ]]; then' \
    '        [[ "${WSLDB_BENCH_WRITE_BUFFER_BYTES:?}" == 0 ]] || exit 22' \
    '    else' \
    '        [[ "${WSLDB_BENCH_WRITE_BUFFER_BYTES:?}" == "${WSLDB_TEST_EXPECTED_WRITE_BUFFER:?}" ]] || exit 22' \
    '    fi' \
    'fi' \
    'if [[ "$workload" == load ]]; then' \
    '    if [[ "${WSLDB_TEST_SUSTAINED_CONTRACT:-0}" == 1 ]]; then' \
    '        [[ -z "${WSLDB_BENCH_OPERATION_LIMIT+x}" ]] || exit 23' \
    '    fi' \
    '    mkdir -p "$(dirname "$database")"' \
    '    printf "fixture\\n" >"$database"' \
    '    load_hash=${WSLDB_TEST_LOAD_HASH:-}' \
    '    [[ -n "$load_hash" ]] || printf -v load_hash "%064d" 0' \
    '    printf "load_complete entries=10 logical_hash=%s\\n" "$load_hash" >&2' \
    'elif [[ "${WSLDB_TEST_SUSTAINED_CONTRACT:-0}" == 1 ]]; then' \
    '    [[ "${WSLDB_BENCH_OPERATION_LIMIT:?}" == "${WSLDB_TEST_EXPECTED_OPERATION_LIMIT:?}" ]] || exit 24' \
    '    printf "w5_all,1,1,1000,1000.000,,,,\\n"' \
    '    printf "w5_overwrite,1,1,1000,1000.000,100,200,300,400\\n"' \
    '    printf "sustained_window=1 elapsed_seconds=1.000 total_operations=1 checkpoint_activity=1 checkpoint_starts=1 checkpoint_operations=0 checkpoint_failures=0 checkpoint_bytes=4096 checkpoints_in_progress=1 checkpoint_total_starts=1 checkpoint_total_operations=0 checkpoint_total_failures=0 checkpoint_total_bytes=4096\\n" >&2' \
    '    case "${WSLDB_TEST_COMPLETION_MODE:-valid}" in' \
    '        missing) ;;' \
    '        duplicate)' \
    '            printf "sustained_complete operations=%s entries=10 logical_hash=%064d\\n" "$WSLDB_BENCH_OPERATION_LIMIT" 0 >&2' \
    '            printf "sustained_complete operations=%s entries=10 logical_hash=%064d\\n" "$WSLDB_BENCH_OPERATION_LIMIT" 0 >&2' \
    '            ;;' \
    '        malformed)' \
    '            printf "sustained_complete operations=%s entries=10 logical_hash=broken\\n" "$WSLDB_BENCH_OPERATION_LIMIT" >&2' \
    '            ;;' \
    '        valid)' \
    '            printf "sustained_complete operations=%s entries=10 logical_hash=%064d\\n" "$WSLDB_BENCH_OPERATION_LIMIT" 0 >&2' \
    '            ;;' \
    '        *) exit 25 ;;' \
    '    esac' \
    'fi' \
    '[[ -f "$database" ]] || exit 1' \
    'printf "stub metric\\n"' >"$SCRATCH/wsldb_bench"
chmod +x "$SCRATCH/wsldb_bench"
printf '%s\n' \
    '#!/usr/bin/env bash' \
    'printf "%s|%s\\n" "$1" "$2" >>"${WSLDB_TEST_CACHE_LOG:?}"' \
    >"$SCRATCH/reset-cache"
chmod +x "$SCRATCH/reset-cache"
printf '%s\n' \
    '#!/usr/bin/env bash' \
    'if [[ "${1:-}" == --version ]]; then' \
    '    printf "rocksdb_sustained_bench version 11.8.1\\n"' \
    '    exit 0' \
    'fi' \
    'mode=${1:?}' \
    'database=${2:?}' \
    '[[ "${WSLDB_BENCH_WRITE_BUFFER_BYTES:?}" == "${WSLDB_TEST_EXPECTED_WRITE_BUFFER:?}" ]] || exit 30' \
    'if [[ "${WSLDB_TEST_EXPECTED_ROCKS_WAL_STATS:-0}" == 1 ]]; then' \
    '    [[ "${WSLDB_ROCKSDB_WAL_STATS:?}" == 1 ]] || exit 31' \
    'else' \
    '    [[ -z "${WSLDB_ROCKSDB_WAL_STATS+x}" ]] || exit 32' \
    'fi' \
    'if [[ "$mode" == load ]]; then' \
    '    [[ -z "${WSLDB_BENCH_OPERATION_LIMIT+x}" ]] || exit 33' \
    '    mkdir -p "$database"' \
    '    printf "fixture\\n" >"$database/data"' \
    '    printf "load_complete entries=10 logical_hash=%064d\\n" 0 >&2' \
    'else' \
    '    [[ "${WSLDB_BENCH_OPERATION_LIMIT:?}" == "${WSLDB_TEST_EXPECTED_OPERATION_LIMIT:?}" ]] || exit 34' \
    '    printf "sustained_complete operations=%s entries=10 logical_hash=%064d\\n" "$WSLDB_BENCH_OPERATION_LIMIT" 0 >&2' \
    'fi' \
    'printf "stub metric\\n"' >"$SCRATCH/rocksdb_sustained_bench"
chmod +x "$SCRATCH/rocksdb_sustained_bench"

existence_line="$(rg -n -F '"delete existence check"' \
    "$SCRIPT_DIR/rocksdb_sustained_bench.cc" | cut -d: -f1)"
delete_line="$(rg -n -F '"durable delete"' \
    "$SCRIPT_DIR/rocksdb_sustained_bench.cc" | cut -d: -f1)"
[[ "$existence_line" =~ ^[0-9]+$ && "$delete_line" =~ ^[0-9]+$ ]] || \
    fail "sustained RocksDB delete contract is missing"
(( existence_line < delete_line && delete_line - existence_line <= 3 )) || \
    fail "sustained RocksDB delete does not follow its exact existence check"

rust_key_line="$(rg -n -F 'let operation_key = key(index);' \
    "$SCRIPT_DIR/wsldb/sustained.rs" | cut -d: -f1)"
rust_timer_line="$(rg -n -F 'let operation_started = Instant::now();' \
    "$SCRIPT_DIR/wsldb/sustained.rs" | cut -d: -f1)"
[[ "$rust_key_line" =~ ^[0-9]+$ && "$rust_timer_line" =~ ^[0-9]+$ ]] || \
    fail "sustained WSLDB timer boundary is missing"
(( rust_key_line < rust_timer_line && rust_timer_line - rust_key_line == 1 )) || \
    fail "sustained WSLDB key encoding is inside the operation timer"
[[ "$(rg -c -F '&operation_key' "$SCRIPT_DIR/wsldb/sustained.rs")" == 4 ]] || \
    fail "sustained WSLDB operations do not share the pre-encoded key"
rocks_key_line="$(rg -n -F 'const auto key = Key(index);' \
    "$SCRIPT_DIR/rocksdb_sustained_bench.cc" | tail -n 1 | cut -d: -f1)"
rocks_timer_line="$(rg -n -F 'const auto operation_started = Clock::now();' \
    "$SCRIPT_DIR/rocksdb_sustained_bench.cc" | cut -d: -f1)"
[[ "$rocks_key_line" =~ ^[0-9]+$ && "$rocks_timer_line" =~ ^[0-9]+$ ]] || \
    fail "sustained RocksDB timer boundary is missing"
(( rocks_key_line < rocks_timer_line && rocks_timer_line - rocks_key_line == 2 )) || \
    fail "sustained RocksDB key encoding is inside the operation timer"
[[ "$(rg -c -F 'window_started =' "$SCRIPT_DIR/wsldb/sustained.rs")" == 2 ]] || \
    fail "sustained WSLDB instrumentation is inside an active window"
[[ "$(rg -c -F 'window_started =' "$SCRIPT_DIR/rocksdb_sustained_bench.cc")" == 2 ]] || \
    fail "sustained RocksDB instrumentation is inside an active window"

rocksdb_commit="$(git -C "$SCRATCH/rocksdb" rev-parse HEAD)"
benchmark_validate_release_inputs 1 "$SCRATCH/wsldb" "$SCRATCH/rocksdb" \
    "$SCRATCH/db_bench" "$rocksdb_commit" "db_bench version 11.8.1"

printf 'dirty\n' >"$SCRATCH/wsldb/untracked"
expect_failure benchmark_validate_release_inputs 1 "$SCRATCH/wsldb" \
    "$SCRATCH/rocksdb" "$SCRATCH/db_bench" "$rocksdb_commit" \
    "db_bench version 11.8.1"
rm "$SCRATCH/wsldb/untracked"

printf 'dirty\n' >"$SCRATCH/rocksdb/untracked"
expect_failure benchmark_validate_release_inputs 1 "$SCRATCH/wsldb" \
    "$SCRATCH/rocksdb" "$SCRATCH/db_bench" "$rocksdb_commit" \
    "db_bench version 11.8.1"
rm "$SCRATCH/rocksdb/untracked"

expect_failure benchmark_validate_release_inputs 1 "$SCRATCH/wsldb" \
    "$SCRATCH/rocksdb" "$SCRATCH/db_bench" \
    0000000000000000000000000000000000000000 "db_bench version 11.8.1"
expect_failure benchmark_validate_release_inputs 1 "$SCRATCH/wsldb" \
    "$SCRATCH/rocksdb" "$SCRATCH/db_bench" "$rocksdb_commit" \
    "db_bench version 10.10.1"

benchmark_validate_release_inputs 0 /missing/wsldb /missing/rocksdb \
    /missing/db_bench ignored ignored
expect_failure benchmark_validate_release_inputs 1 /missing/wsldb \
    "$SCRATCH/rocksdb" "$SCRATCH/db_bench" "$rocksdb_commit" \
    "db_bench version 11.8.1"
expect_failure benchmark_validate_release_inputs 1 "$SCRATCH/wsldb" \
    /missing/rocksdb "$SCRATCH/db_bench" "$rocksdb_commit" \
    "db_bench version 11.8.1"
[[ "$(WSLDB_BENCHMARK_RELEASE_MODE=0 benchmark_contract_mode)" == 0 ]] || \
    fail "diagnostic mode was not preserved"
expect_failure env WSLDB_BENCHMARK_RELEASE_MODE=invalid bash -c \
    'source "$1"; benchmark_contract_mode' _ "$SCRIPT_DIR/benchmark-contract.sh"

env WSLDB_BENCHMARK_RELEASE_MODE=0 \
    WSLDB_BENCH_BIN="$SCRATCH/wsldb_bench" \
    WSLDB_COMPARISON_PHASES=' ' \
    "$SCRIPT_DIR/run-comparison.sh" "$SCRATCH/db_bench" \
    "$SCRATCH/comparison-output" >/dev/null
rg -q '^source_contract_satisfied=no$' \
    "$SCRATCH/comparison-output/manifest.txt" || fail "comparison contract was not recorded"
rg -q '^wsldb_build_snapshot=working-tree$' \
    "$SCRATCH/comparison-output/manifest.txt" || fail "comparison build snapshot is missing"
rg -q '^release_gate_complete=no$' \
    "$SCRATCH/comparison-output/manifest.txt" || fail "comparison gate scope was not recorded"
rg -q '^wsldb_benchmark_binary_sha256=[0-9a-f]{64}$' \
    "$SCRATCH/comparison-output/manifest.txt" || fail "comparison binary hash is missing"
rg -q '^comparison_runner_sha256=[0-9a-f]{64}$' \
    "$SCRATCH/comparison-output/manifest.txt" || fail "comparison runner hash is missing"
rg -q '^benchmark_contract_sha256=[0-9a-f]{64}$' \
    "$SCRATCH/comparison-output/manifest.txt" || fail "comparison contract hash is missing"
rg -q '^evidence_hash_writer_sha256=[0-9a-f]{64}$' \
    "$SCRATCH/comparison-output/manifest.txt" || fail "comparison hash writer is missing"
rg -q '^evidence_hash_verifier_sha256=[0-9a-f]{64}$' \
    "$SCRATCH/comparison-output/manifest.txt" || fail "comparison hash verifier is missing"
rg -q '^runner_complete=yes$' "$SCRATCH/comparison-output/manifest.txt" || \
    fail "comparison completion is missing"
rg -q '^runner_inputs_revalidated=yes$' \
    "$SCRATCH/comparison-output/manifest.txt" || fail "comparison inputs were not revalidated"
expect_evidence_hash "$SCRATCH/comparison-output" manifest.txt
expect_evidence_hash "$SCRATCH/comparison-output" commands.log
"$SCRIPT_DIR/../scripts/verify-evidence-hashes.sh" \
    "$SCRATCH/comparison-output" >/dev/null

expect_failure env WSLDB_BENCHMARK_RELEASE_MODE=0 \
    WSLDB_BENCH_BIN="$SCRATCH/wsldb_bench" \
    WSLDB_SUSTAINED_DISTRIBUTIONS=' ' \
    "$SCRIPT_DIR/run-sustained-comparison.sh" \
    "$SCRATCH/rocksdb_sustained_bench" "$SCRATCH/rocksdb" \
    "$SCRATCH/sustained-empty"
expect_failure env WSLDB_BENCHMARK_RELEASE_MODE=0 \
    WSLDB_BENCH_BIN="$SCRATCH/wsldb_bench" \
    WSLDB_SUSTAINED_DISTRIBUTIONS='uniform uniform' \
    "$SCRIPT_DIR/run-sustained-comparison.sh" \
    "$SCRATCH/rocksdb_sustained_bench" "$SCRATCH/rocksdb" \
    "$SCRATCH/sustained-duplicate"
expect_failure env WSLDB_BENCHMARK_RELEASE_MODE=0 \
    WSLDB_BENCH_BIN="$SCRATCH/wsldb_bench" \
    WSLDB_ROCKSDB_WAL_STATS=0 \
    "$SCRIPT_DIR/run-sustained-comparison.sh" \
    "$SCRATCH/rocksdb_sustained_bench" "$SCRATCH/rocksdb" \
    "$SCRATCH/sustained-invalid-wal-stats"
expect_failure env WSLDB_BENCHMARK_RELEASE_MODE=0 \
    WSLDB_BENCH_BIN="$SCRATCH/wsldb_bench" \
    WSLDB_SUSTAINED_WRITE_BUFFER_BYTES=0 \
    "$SCRIPT_DIR/run-sustained-comparison.sh" \
    "$SCRATCH/rocksdb_sustained_bench" "$SCRATCH/rocksdb" \
    "$SCRATCH/sustained-zero-write-buffer"
if env WSLDB_BENCHMARK_RELEASE_MODE=1 \
    WSLDB_SUSTAINED_WRITE_BUFFER_BYTES=33554432 \
    "$SCRIPT_DIR/run-sustained-comparison.sh" \
    "$SCRATCH/rocksdb_sustained_bench" "$SCRATCH/rocksdb" \
    "$SCRATCH/sustained-nonfrozen-write-buffer" \
    >"$SCRATCH/nonfrozen.stdout" 2>"$SCRATCH/nonfrozen.stderr"; then
    fail "release sustained runner accepted a non-frozen write buffer"
fi
rg -q 'release mode requires a 67108864-byte write buffer' \
    "$SCRATCH/nonfrozen.stderr" || fail "release write-buffer failure was ambiguous"

run_sustained_fixture() {
    local output="$1"
    local completion_mode="${2:-valid}"
    local load_hash="${3:-}"
    env WSLDB_BENCHMARK_RELEASE_MODE=0 \
        WSLDB_BENCH_BIN="$SCRATCH/wsldb_bench" \
        WSLDB_BENCH_DRAIN=1 \
        WSLDB_BENCH_OPERATION_LIMIT=9 \
        WSLDB_BENCH_WRITE_BUFFER_BYTES=999 \
        WSLDB_ROCKSDB_WAL_STATS=1 \
        WSLDB_TEST_SUSTAINED_CONTRACT=1 \
        WSLDB_TEST_EXPECTED_OPERATION_LIMIT=1 \
        WSLDB_TEST_EXPECTED_WRITE_BUFFER=67108864 \
        WSLDB_TEST_EXPECTED_ROCKS_WAL_STATS=1 \
        WSLDB_TEST_COMPLETION_MODE="$completion_mode" \
        WSLDB_TEST_LOAD_HASH="$load_hash" \
        WSLDB_SUSTAINED_ENTRIES=10 \
        WSLDB_SUSTAINED_VALUE_BYTES=10 \
        WSLDB_SUSTAINED_DURATION_SECONDS=1 \
        WSLDB_SUSTAINED_CACHE_BYTES=4096 \
        WSLDB_SUSTAINED_COMPACT_BYTES=4096 \
        WSLDB_SUSTAINED_MAINTENANCE_SECONDS=1 \
        WSLDB_SUSTAINED_DISTRIBUTIONS=uniform \
        WSLDB_SUSTAINED_MEASURED_RUNS=1 \
        WSLDB_SUSTAINED_WRITE_BUFFER_BYTES=67108864 \
        WSLDB_SUSTAINED_OPERATION_LIMIT=1 \
        "$SCRIPT_DIR/run-sustained-comparison.sh" \
        "$SCRATCH/rocksdb_sustained_bench" "$SCRATCH/rocksdb" "$output"
}

env WSLDB_BENCHMARK_RELEASE_MODE=0 \
    WSLDB_BENCH_BIN="$SCRATCH/wsldb_bench" \
    WSLDB_COMPARISON_CACHE_STATE=cold \
    WSLDB_COLD_CACHE_COMMAND="$SCRATCH/reset-cache" \
    WSLDB_TEST_CACHE_LOG="$SCRATCH/cache-resets" \
    WSLDB_COMPARISON_ENTRIES=10 \
    WSLDB_COMPARISON_OPERATIONS=2 \
    WSLDB_COMPARISON_RANGE_OPERATIONS=2 \
    WSLDB_COMPARISON_THREADS=1 \
    WSLDB_COMPARISON_RANGES=2 \
    WSLDB_COMPARISON_VALUES=10 \
    WSLDB_COMPARISON_PHASES='reads ranges opens' \
    "$SCRIPT_DIR/run-comparison.sh" "$SCRATCH/db_bench" \
    "$SCRATCH/cold-output" >/dev/null
[[ "$(wc -l <"$SCRATCH/cache-resets" | tr -d ' ')" == 36 ]] || \
    fail "cold comparison did not reset before all 36 isolated engine runs"
expected_order=$'# cache-reset-w3-wsldb-v10-t1-r0\n# w3-wsldb-v10-t1-r0\n# cache-reset-w3-rocksdb-v10-t1-r0\n# w3-rocksdb-v10-t1-r0\n# cache-reset-w3-rocksdb-v10-t1-r1\n# w3-rocksdb-v10-t1-r1\n# cache-reset-w3-wsldb-v10-t1-r1\n# w3-wsldb-v10-t1-r1'
actual_order="$(awk '/^# .*w3.*-r[01]$/ { print; count += 1; if (count == 8) exit }' \
    "$SCRATCH/cold-output/commands.log")"
[[ "$actual_order" == "$expected_order" ]] || \
    fail "cold comparison did not alternate the first two engine pairs"
rg -q 'WSLDB_BENCH_WARMUP=0 .*WSLDB_BENCH_MEASURED_RUNS=1 .*WSLDB_BENCH_RUN_START=0' \
    "$SCRATCH/cold-output/commands.log" || fail "cold WSLDB process was not isolated"
rg -q '^cache_state=cold$' "$SCRATCH/cold-output/manifest.txt" || \
    fail "cold cache state was not recorded"
rg -q '^cold_cache_command_sha256=[0-9a-f]{64}$' \
    "$SCRATCH/cold-output/manifest.txt" || fail "cold reset hash is missing"

run_sustained_fixture "$SCRATCH/sustained-output" >/dev/null
rg -q '^source_contract_satisfied=no$' \
    "$SCRATCH/sustained-output/manifest.txt" || fail "sustained contract was not recorded"
rg -q '^wsldb_build_snapshot=working-tree$' \
    "$SCRATCH/sustained-output/manifest.txt" || fail "sustained build snapshot is missing"
rg -q '^release_gate_complete=no$' \
    "$SCRATCH/sustained-output/manifest.txt" || fail "sustained gate scope was not recorded"
rg -q '^wsldb_benchmark_sha256=[0-9a-f]{64}$' \
    "$SCRATCH/sustained-output/manifest.txt" || fail "sustained binary hash is missing"
rg -q '^sustained_runner_sha256=[0-9a-f]{64}$' \
    "$SCRATCH/sustained-output/manifest.txt" || fail "sustained runner hash is missing"
rg -q '^benchmark_contract_sha256=[0-9a-f]{64}$' \
    "$SCRATCH/sustained-output/manifest.txt" || fail "sustained contract hash is missing"
rg -q '^checkpoint_window_report_sha256=[0-9a-f]{64}$' \
    "$SCRATCH/sustained-output/manifest.txt" || fail "window report hash is missing"
rg -q '^evidence_hash_writer_sha256=[0-9a-f]{64}$' \
    "$SCRATCH/sustained-output/manifest.txt" || fail "sustained hash writer is missing"
rg -q '^evidence_hash_verifier_sha256=[0-9a-f]{64}$' \
    "$SCRATCH/sustained-output/manifest.txt" || fail "sustained hash verifier is missing"
rg -q '^runner_complete=yes$' "$SCRATCH/sustained-output/manifest.txt" || \
    fail "sustained completion is missing"
rg -q '^runner_inputs_revalidated=yes$' \
    "$SCRATCH/sustained-output/manifest.txt" || fail "sustained inputs were not revalidated"
rg -q '^rocksdb_wal_stats_enabled=1$' \
    "$SCRATCH/sustained-output/manifest.txt" || fail "sustained WAL statistics mode is missing"
rg -q '^write_buffer_bytes=67108864$' \
    "$SCRATCH/sustained-output/manifest.txt" || fail "WSLDB write buffer is missing"
rg -q '^rocksdb_write_buffer_bytes=67108864$' \
    "$SCRATCH/sustained-output/manifest.txt" || fail "RocksDB write buffer is missing"
rg -q 'env -u WSLDB_BENCH_OPERATION_LIMIT -u WSLDB_BENCH_WRITE_BUFFER_BYTES -u WSLDB_ROCKSDB_WAL_STATS WSLDB_BENCH_WRITE_BUFFER_BYTES=67108864' \
    "$SCRATCH/sustained-output/commands.log" || fail "RocksDB write buffer environment was not isolated"
rg -q '^system_begin$' "$SCRATCH/sustained-output/manifest.txt" || \
    fail "sustained system manifest start is missing"
rg -q '^system_end$' "$SCRATCH/sustained-output/manifest.txt" || \
    fail "sustained system manifest end is missing"
[[ "$(wc -l <"$SCRATCH/sustained-output/checkpoint-windows.csv" | tr -d ' ')" == 3 ]] || \
    fail "sustained checkpoint report did not contain both runs"
awk -F, '
    NR == 1 { next }
    $1 != "w5" || $3 != 1 || $4 != 1 || $5 != 1 || $8 != 4096 ||
        $10 != 1 || $11 != 300 || $12 != 0 || $13 != "" ||
        $14 != 1 || $15 != "1000.000" { exit 1 }
    END { exit NR == 3 ? 0 : 1 }
' "$SCRATCH/sustained-output/checkpoint-windows.csv" || \
    fail "sustained checkpoint report did not preserve joined window fields"
expect_evidence_hash "$SCRATCH/sustained-output" manifest.txt
expect_evidence_hash "$SCRATCH/sustained-output" checkpoint-windows.csv
"$SCRIPT_DIR/../scripts/verify-evidence-hashes.sh" \
    "$SCRATCH/sustained-output" >/dev/null

printf 'metric,run,operations,elapsed_ns,ops_per_second,p50_ns,p95_ns,p99_ns,p999_ns\nw5_all,1,1,1000,1000.000,,,,\n' \
    >"$SCRATCH/checkpoint-report.stdout"
printf 'sustained_window=1 checkpoint_activity=1 checkpoint_starts=1 checkpoint_operations=0 checkpoint_failures=0 checkpoint_bytes=4096\n' \
    >"$SCRATCH/checkpoint-report.stderr"
expect_failure "$SCRIPT_DIR/checkpoint-window-report.sh" \
    "$SCRATCH/checkpoint-report.stdout" "$SCRATCH/checkpoint-report.stderr" w5 0
printf 'sustained_window=1 checkpoint_activity=0 checkpoint_starts=1 checkpoint_operations=0 checkpoint_failures=0 checkpoint_bytes=0 checkpoints_in_progress=0\n' \
    >"$SCRATCH/checkpoint-report.stderr"
expect_failure "$SCRIPT_DIR/checkpoint-window-report.sh" \
    "$SCRATCH/checkpoint-report.stdout" "$SCRATCH/checkpoint-report.stderr" w5 0

for completion_mode in missing duplicate malformed; do
    expect_failure run_sustained_fixture \
        "$SCRATCH/sustained-$completion_mode" "$completion_mode"
done
[[ ! -e "$SCRATCH/sustained-missing/evidence-sha256.txt" ]] || \
    fail "failed sustained run published a final evidence hash manifest"
readonly DIFFERENT_LOAD_HASH="$(printf '%063d1' 0)"
expect_failure run_sustained_fixture \
    "$SCRATCH/sustained-load-mismatch" valid "$DIFFERENT_LOAD_HASH"

printf 'benchmark contract tests passed\n'

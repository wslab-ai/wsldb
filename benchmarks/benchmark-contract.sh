#!/usr/bin/env bash

readonly BENCHMARK_ROCKSDB_VERSION="11.8.1"
readonly BENCHMARK_ROCKSDB_COMMIT="abeebd9630f11bd08c28b7bd43c7bdfc62050654"

benchmark_contract_error() {
    printf 'benchmark contract error: %s\n' "$*" >&2
    return 1
}

benchmark_contract_mode() {
    local mode="${WSLDB_BENCHMARK_RELEASE_MODE:-1}"
    case "$mode" in
        0 | 1) printf '%s' "$mode" ;;
        *) benchmark_contract_error "WSLDB_BENCHMARK_RELEASE_MODE must be 0 or 1" ;;
    esac
}

benchmark_resolve_rocksdb_source() {
    local binary=$1
    local configured=${2:-}
    if [[ -n "$configured" ]]; then
        printf '%s' "$configured"
        return
    fi
    git -C "$(dirname "$binary")" rev-parse --show-toplevel 2>/dev/null || true
}

benchmark_sha256_file() {
    local file=$1
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$file" | awk '{ print $1 }'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$file" | awk '{ print $1 }'
    else
        benchmark_contract_error "sha256sum or shasum is required"
    fi
}

benchmark_validate_file_snapshot() {
    local file=$1
    local expected=$2
    local label=$3
    local actual
    if [[ ! -f "$file" ]]; then
        benchmark_contract_error "$label is no longer a regular file: $file"
        return 1
    fi
    actual=$(benchmark_sha256_file "$file") || return
    if [[ "$actual" != "$expected" ]]; then
        benchmark_contract_error "$label changed during the run: $file"
        return 1
    fi
}

benchmark_validate_source_snapshot() {
    local mode=$1
    local source=$2
    local expected_commit=$3
    local label=$4
    local actual_commit

    [[ "$mode" == 1 ]] || return 0
    if ! git -C "$source" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
        benchmark_contract_error "$label source is not a Git checkout: $source"
        return 1
    fi
    actual_commit=$(git -C "$source" rev-parse HEAD) || return
    if [[ "$actual_commit" != "$expected_commit" ]]; then
        benchmark_contract_error \
            "$label commit changed during the run: $actual_commit (expected $expected_commit)"
        return 1
    fi
    if [[ -n "$(git -C "$source" status --porcelain=v1)" ]]; then
        benchmark_contract_error "$label checkout changed during the run"
        return 1
    fi
}

benchmark_create_source_snapshot() {
    local source=$1
    local commit=$2
    local destination=$3
    if ! git clone --quiet --no-local --no-checkout "$source" "$destination"; then
        benchmark_contract_error "could not clone source snapshot: $source"
        return 1
    fi
    if ! git -C "$destination" checkout --quiet --detach "$commit"; then
        benchmark_contract_error "could not check out source snapshot: $commit"
        return 1
    fi
    benchmark_validate_source_snapshot 1 "$destination" "$commit" \
        "isolated WSLDB build"
}

benchmark_validate_binary_version() {
    local binary=$1
    local expected=$2
    local actual
    if ! actual=$("$binary" --version 2>&1); then
        benchmark_contract_error "could not query benchmark version: $binary"
        return
    fi
    if [[ "$actual" != "$expected" ]]; then
        benchmark_contract_error \
            "unexpected benchmark version from $binary: $actual (expected $expected)"
    fi
}

benchmark_validate_release_inputs() {
    local mode=$1
    local wsldb_source=$2
    local rocksdb_source=$3
    local rocksdb_binary=$4
    local expected_commit=$5
    local expected_version=$6

    [[ "$mode" == 1 ]] || return 0
    if ! git -C "$wsldb_source" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
        benchmark_contract_error "WSLDB source is not a Git checkout: $wsldb_source"
        return 1
    fi
    if [[ -n "$(git -C "$wsldb_source" status --porcelain=v1)" ]]; then
        benchmark_contract_error "WSLDB checkout must be clean in release mode"
        return 1
    fi
    if [[ -z "$rocksdb_source" ]]; then
        benchmark_contract_error "set WSLDB_ROCKSDB_SOURCE to the pinned checkout"
        return 1
    fi
    if ! git -C "$rocksdb_source" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
        benchmark_contract_error "RocksDB source is not a Git checkout: $rocksdb_source"
        return 1
    fi

    local actual_commit
    actual_commit=$(git -C "$rocksdb_source" rev-parse HEAD)
    if [[ "$actual_commit" != "$expected_commit" ]]; then
        benchmark_contract_error \
            "RocksDB commit is $actual_commit; expected $expected_commit"
        return 1
    fi
    if [[ -n "$(git -C "$rocksdb_source" status --porcelain=v1)" ]]; then
        benchmark_contract_error "RocksDB checkout must be clean in release mode"
        return 1
    fi
    benchmark_validate_binary_version "$rocksdb_binary" "$expected_version"
}

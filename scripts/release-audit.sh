#!/usr/bin/env bash
set -euo pipefail

repository=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repository"

if ! command -v rg >/dev/null 2>&1; then
    echo "release audit requires ripgrep (rg)" >&2
    exit 1
fi

while IFS= read -r path; do
    case "$path" in
        .github/* | \
            .DS_Store | \
            */.DS_Store | \
            target/* | \
            */target/* | \
            NOTICE | \
            NOTICE.* | \
            */NOTICE | \
            */NOTICE.* | \
            *file-format-v[0-9]*)
            echo "forbidden release artifact: $path" >&2
            exit 1
            ;;
    esac
done < <(git ls-files)

if git grep -n -i \
    -e 'l[m]db' \
    -e 'mdb[.]master3' \
    -e 'f[e]rrodb' \
    -- . ':(exclude)scripts/release-audit.sh'
then
    echo "legacy project attribution remains in tracked files" >&2
    exit 1
fi

sha256_file() {
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    elif command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        echo "release audit requires shasum or sha256sum" >&2
        exit 1
    fi
}

check_sha256() {
    local path=$1
    local expected=$2
    local contract=$3
    local actual
    actual=$(sha256_file "$path")
    if [[ "$actual" != "$expected" ]]; then
        echo "$contract hash changed: $path ($actual)" >&2
        exit 1
    fi
}

check_sha256 tests/fixtures/database.wsldb \
    eb64a848d5c543a71725ad680df1827a322ee9e425adc44190d4db9dda3d9e52 \
    "format fixture"
check_sha256 tests/fixtures/buffered.wsldb \
    c030112bcfd0786e69d8792546648dfdebe49795bec6ff3ec8280fa0b953cfc2 \
    "format fixture"
check_sha256 tests/fixtures/buffered.wsldb.wal \
    e5967eb6e30ff9ab4b06a830f56a96eb08672f70d1fee71ae6a2a9312af8a64a \
    "format fixture"
check_sha256 tests/public_api.rs \
    2ae6abb818ee92d5b6af81c9169f6c24c7eb551ee64d5df0cec3a7d12cf81829 \
    "public API compile contract"

bash -n \
    benchmarks/benchmark-contract.sh \
    benchmarks/benchmark-contract.test.sh \
    benchmarks/cache-reset-linux.sh \
    benchmarks/cache-reset-macos.sh \
    benchmarks/checkpoint-window-report.sh \
    benchmarks/run-comparison.sh \
    benchmarks/run-sustained-comparison.sh \
    scripts/run-linux-release-suite.sh \
    scripts/run-power-loss-cycle.sh \
    scripts/run-power-loss-cycle.test.sh \
    scripts/verify-evidence-hashes.sh \
    scripts/verify-evidence-hashes.test.sh \
    scripts/write-evidence-hashes.sh \
    scripts/write-evidence-hashes.test.sh
benchmarks/benchmark-contract.test.sh
scripts/run-power-loss-cycle.sh --check-host
scripts/run-power-loss-cycle.test.sh
scripts/verify-evidence-hashes.test.sh
scripts/write-evidence-hashes.test.sh
case "$(uname -s)" in
    Darwin) benchmarks/cache-reset-macos.sh --check-host ;;
    Linux) benchmarks/cache-reset-linux.sh --check-host ;;
esac

metadata=$(cargo metadata --locked --no-deps --format-version 1)
if ! rg -q '"name":"wsldb","version":"1[.]0[.]0"' <<<"$metadata"; then
    echo "Cargo package must be wsldb 1.0.0" >&2
    exit 1
fi
if ! rg -q '"license":"MIT"' <<<"$metadata"; then
    echo "Cargo package license must be MIT" >&2
    exit 1
fi
if ! rg -q '"publish":\[\]' <<<"$metadata"; then
    echo "Cargo publishing must remain disabled before release approval" >&2
    exit 1
fi

while IFS= read -r path; do
    lines=$(wc -l <"$path")
    if ((lines > 700)); then
        echo "Rust source exceeds 700 lines: $path ($lines)" >&2
        exit 1
    fi
done < <(git ls-files '*.rs')

while IFS= read -r path; do
    lines=$(wc -l <"$path")
    if ((lines > 1000)); then
        echo "Markdown file exceeds 1000 lines: $path ($lines)" >&2
        exit 1
    fi
done < <(git ls-files '*.md')

package_list=$(cargo package --locked --list --allow-dirty)
while IFS= read -r path; do
    case "$path" in
        .DS_Store | */.DS_Store | target/* | */target/*)
            echo "forbidden packaged build or host artifact: $path" >&2
            exit 1
            ;;
    esac
done <<<"$package_list"

for required in \
    Cargo.toml \
    LICENSE \
    README.md \
    benchmarks/benchmark-contract.sh \
    benchmarks/benchmark-contract.test.sh \
    benchmarks/cache-reset-linux.sh \
    benchmarks/cache-reset-macos.sh \
    benchmarks/checkpoint-window-report.sh \
    benchmarks/rocksdb_open_bench.cc \
    benchmarks/rocksdb_sustained_bench.cc \
    benchmarks/verification.rs \
    benchmarks/wsldb/measurement.rs \
    docs/file-format.md \
    scripts/run-linux-release-suite.sh \
    scripts/run-power-loss-cycle.sh \
    scripts/run-power-loss-cycle.test.sh \
    scripts/verify-evidence-hashes.sh \
    scripts/verify-evidence-hashes.test.sh \
    scripts/write-evidence-hashes.sh \
    scripts/write-evidence-hashes.test.sh \
    src/lib.rs \
    tests/benchmark_measurement.rs \
    tests/injection_contract.rs \
    tests/path_identity.rs \
    tests/public_api.rs \
    tests/fixtures/database.wsldb \
    tests/fixtures/buffered.wsldb \
    tests/fixtures/buffered.wsldb.wal
do
    if ! rg -q "^${required//./[.]}$" <<<"$package_list"; then
        echo "Cargo package is missing $required" >&2
        exit 1
    fi
done

git diff --check
echo "release hygiene audit passed"

#!/usr/bin/env bash
set -euo pipefail

repository=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)
runner="$repository/scripts/run-power-loss-cycle.sh"
scratch=$(mktemp -d)

cleanup() {
    find "$scratch" -depth -delete
}
trap cleanup EXIT

sha256_file() {
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        sha256sum "$1" | awk '{print $1}'
    fi
}

boot_token() {
    case "$(uname -s)" in
        Darwin) sysctl -n kern.boottime | shasum -a 256 | awk '{print $1}' ;;
        Linux) sed -n '1p' /proc/sys/kernel/random/boot_id ;;
        *) exit 77 ;;
    esac
}

mount_type() {
    case "$(uname -s)" in
        Darwin) stat -f '%T' "$1" ;;
        Linux) findmnt -n -o FSTYPE --target "$1" ;;
        *) exit 77 ;;
    esac
}

mount_source() {
    case "$(uname -s)" in
        Darwin) df -P "$1" | awk 'NR == 2 {print $1}' ;;
        Linux) findmnt -n -o SOURCE --target "$1" ;;
        *) exit 77 ;;
    esac
}

expect_failure() {
    local label=$1
    shift
    if "$@" >"$scratch/${label}.stdout" 2>"$scratch/${label}.stderr"; then
        echo "expected failure: $label" >&2
        exit 1
    fi
}

write_value() {
    printf '%s\n' "$2" >"$1"
}

"$runner" --check-host >"$scratch/check-host.stdout"

same_mount_database="$scratch/same-mount.wsldb"
same_mount_oracle="$scratch/same-mount.oracle"
same_mount_evidence="$scratch/same-mount-evidence"
expect_failure same-mount \
    "$runner" prepare direct "$same_mount_database" "$same_mount_oracle" "$same_mount_evidence"
[[ ! -e "$same_mount_evidence" ]] || {
    echo "same-mount rejection created evidence" >&2
    exit 1
}

evidence="$scratch/evidence"
mkdir -p "$evidence/cycles/000001"
database="$scratch/existing.wsldb"
oracle="$scratch/existing.oracle"
: >"$database"
: >"$oracle"
printf '#!/usr/bin/env bash\nexit 99\n' >"$evidence/power_loss_probe"
chmod 0555 "$evidence/power_loss_probe"
write_value "$evidence/mode" direct
write_value "$evidence/database-path" "$database"
write_value "$evidence/oracle-path" "$oracle"
write_value "$evidence/next-cycle" 1
write_value "$evidence/runner.sha256" "$(sha256_file "$runner")"
write_value "$evidence/probe.sha256" "$(sha256_file "$evidence/power_loss_probe")"
printf '%s\n' \
    'device_model=' \
    'device_firmware=' \
    'controller=' \
    'write_cache=' \
    'power_protection=' \
    'oracle_independence=' \
    >"$evidence/hardware.txt"

find "$evidence/cycles/000001" -depth -delete
expect_failure incomplete-hardware "$runner" run "$evidence"
[[ ! -e "$evidence/cycles/000001" ]] || {
    echo "incomplete hardware metadata armed a cycle" >&2
    exit 1
}

cycle="$evidence/cycles/000001"
mkdir "$cycle"
write_value "$cycle/run-boot-token" "$(boot_token)"
expect_failure unchanged-boot "$runner" verify "$evidence"
[[ ! -e "$cycle/verify-exit" ]] || {
    echo "unchanged boot token reached probe verification" >&2
    exit 1
}

write_value "$cycle/orderly-exit" 0
expect_failure orderly-exit "$runner" verify "$evidence"
[[ ! -e "$cycle/verify-exit" ]] || {
    echo "orderly cycle reached probe verification" >&2
    exit 1
}

find "$cycle/orderly-exit" -delete
write_value "$cycle/run-boot-token" not-the-current-boot-token
write_value "$evidence/database-filesystem" "$(mount_type "$database")"
write_value "$evidence/database-mount-source" "$(mount_source "$database")"
printf '%s\n' \
    'physical_cut_method=' \
    'physical_cut_time_utc=' \
    'filesystem_recovery=' \
    >"$cycle/cut-attestation.txt"
expect_failure incomplete-attestation "$runner" verify "$evidence"
[[ ! -e "$cycle/verify-exit" ]] || {
    echo "incomplete attestation reached probe verification" >&2
    exit 1
}

sealed_evidence="$scratch/sealed-evidence"
sealed_cycle="$sealed_evidence/cycles/000001"
sealed_database="$scratch/sealed.wsldb"
sealed_oracle="$scratch/sealed.oracle"
mkdir -p "$sealed_cycle"
: >"$sealed_database"
: >"$sealed_oracle"
printf '#!/usr/bin/env bash\nexit 99\n' >"$sealed_evidence/power_loss_probe"
chmod 0555 "$sealed_evidence/power_loss_probe"
write_value "$sealed_evidence/mode" direct
write_value "$sealed_evidence/database-path" "$sealed_database"
write_value "$sealed_evidence/oracle-path" "$sealed_oracle"
write_value "$sealed_evidence/next-cycle" 2
write_value "$sealed_evidence/runner.sha256" "$(sha256_file "$runner")"
write_value "$sealed_evidence/evidence-hash-writer.sha256" \
    "$(sha256_file "$repository/scripts/write-evidence-hashes.sh")"
write_value "$sealed_evidence/evidence-hash-verifier.sha256" \
    "$(sha256_file "$repository/scripts/verify-evidence-hashes.sh")"
write_value "$sealed_evidence/probe.sha256" \
    "$(sha256_file "$sealed_evidence/power_loss_probe")"
printf '%s\n' \
    'device_model=test-device' \
    'device_firmware=test-firmware' \
    'controller=test-controller' \
    'write_cache=enabled' \
    'power_protection=none' \
    'oracle_independence=separate-test-device' \
    >"$sealed_evidence/hardware.txt"

: >"$sealed_cycle/ack.stdout"
: >"$sealed_cycle/run.stderr"
: >"$sealed_cycle/verify.stdout"
: >"$sealed_cycle/verify.stderr"
: >"$sealed_cycle/platform-before.txt"
: >"$sealed_cycle/platform-after.txt"
: >"$sealed_cycle/artifacts.txt"
write_value "$sealed_cycle/run-boot-token" test-before-boot
write_value "$sealed_cycle/run-started-utc" 2026-10-05T00:00:00Z
write_value "$sealed_cycle/after-boot-token" test-after-boot
printf '%s\n' \
    'physical_cut_method=test-fixture' \
    'physical_cut_time_utc=2026-10-05T00:00:01Z' \
    'filesystem_recovery=clean' \
    >"$sealed_cycle/cut-attestation.txt"
write_value "$sealed_cycle/verify-exit" 0
write_value "$sealed_cycle/hardware.sha256" \
    "$(sha256_file "$sealed_evidence/hardware.txt")"
write_value "$sealed_cycle/cut-attestation.sha256" \
    "$(sha256_file "$sealed_cycle/cut-attestation.txt")"
write_value "$sealed_cycle/technical-status" passed
write_value "$sealed_cycle/manual-review-required" yes
write_value "$sealed_cycle/release-gate-complete" no

"$runner" seal "$sealed_evidence" >"$scratch/seal.stdout"
[[ -s "$sealed_evidence/evidence-sha256.txt" ]] || {
    echo "seal did not create an evidence hash manifest" >&2
    exit 1
}
[[ "$(sed -n '1p' "$sealed_evidence/sealed-cycle-count")" == 1 ]] || {
    echo "seal recorded the wrong cycle count" >&2
    exit 1
}
[[ "$(sed -n '1p' "$sealed_evidence/release-gate-complete")" == no ]] || {
    echo "seal made an unsupported release-gate claim" >&2
    exit 1
}
"$repository/scripts/verify-evidence-hashes.sh" "$sealed_evidence" >/dev/null
expect_failure repeated-seal "$runner" seal "$sealed_evidence"
expect_failure run-after-seal "$runner" run "$sealed_evidence"
[[ ! -e "$sealed_evidence/cycles/000002" ]] || {
    echo "sealed evidence accepted another cycle" >&2
    exit 1
}

echo "power-loss runner fail-closed tests passed"

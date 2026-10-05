#!/usr/bin/env bash

set -euo pipefail

fail() {
    printf 'checkpoint window report failed: %s\n' "$*" >&2
    exit 1
}

if [[ $# -ne 4 ]]; then
    fail "usage: $0 /path/to/stdout /path/to/stderr w5-or-w6 run"
fi

readonly STDOUT_LOG="$1"
readonly STDERR_LOG="$2"
readonly WORKLOAD="$3"
readonly OUTER_RUN="$4"

[[ -f "$STDOUT_LOG" ]] || fail "stdout log is missing: $STDOUT_LOG"
[[ -f "$STDERR_LOG" ]] || fail "stderr log is missing: $STDERR_LOG"
[[ "$WORKLOAD" == w5 || "$WORKLOAD" == w6 ]] || fail "workload must be w5 or w6"
[[ "$OUTER_RUN" =~ ^[0-9]+$ ]] || fail "run must be a non-negative integer"

awk -F, -v workload="$WORKLOAD" -v outer_run="$OUTER_RUN" '
    function fail(message) {
        print "checkpoint window report failed: " message > "/dev/stderr"
        failed = 1
        exit 1
    }

    function unsigned(value) {
        return value ~ /^[0-9]+$/
    }

    function decimal(value) {
        return value ~ /^[0-9]+([.][0-9]+)?$/
    }

    function remember_metric(kind, window, operations, p99, rate) {
        if (!unsigned(window) || window == 0 || !unsigned(operations)) {
            fail("malformed " kind " metric")
        }
        key = kind SUBSEP window
        if (metric_seen[key]++) {
            fail("duplicate " kind " metric for window " window)
        }
        metric_operations[key] = operations
        if (kind == "all") {
            if (!decimal(rate)) {
                fail("malformed throughput for window " window)
            }
            metric_rate[key] = rate
        } else {
            if (!unsigned(p99)) {
                fail("malformed p99 for " kind " window " window)
            }
            metric_p99[key] = p99
        }
        if (window > maximum_window) {
            maximum_window = window
        }
    }

    FNR == NR {
        if ($1 == workload "_all") {
            remember_metric("all", $2, $3, "", $5)
        } else if ($1 == workload "_overwrite") {
            remember_metric("overwrite", $2, $3, $8, "")
        } else if ($1 == workload "_delete") {
            remember_metric("delete", $2, $3, $8, "")
        }
        next
    }

    {
        count = split($0, fields, /[[:space:]]+/)
        for (name in values) {
            delete values[name]
        }
        for (field_index = 1; field_index <= count; field_index++) {
            separator = index(fields[field_index], "=")
            if (separator == 0) {
                continue
            }
            name = substr(fields[field_index], 1, separator - 1)
            value = substr(fields[field_index], separator + 1)
            values[name] = value
        }
        window = values["sustained_window"]
        if (window == "") {
            next
        }
        if (!unsigned(window) || window == 0 || activity_seen[window]++) {
            fail("malformed or duplicate checkpoint record for window " window)
        }
        required[1] = "checkpoint_activity"
        required[2] = "checkpoint_starts"
        required[3] = "checkpoint_operations"
        required[4] = "checkpoint_failures"
        required[5] = "checkpoint_bytes"
        required[6] = "checkpoints_in_progress"
        for (required_index = 1; required_index <= 6; required_index++) {
            name = required[required_index]
            if (!unsigned(values[name])) {
                fail("missing or malformed " name " for window " window)
            }
            activity[name SUBSEP window] = values[name]
        }
        if (values["checkpoint_activity"] != 0 && values["checkpoint_activity"] != 1) {
            fail("checkpoint activity must be zero or one for window " window)
        }
        if (values["checkpoint_activity"] == 0 &&
            (values["checkpoint_starts"] != 0 ||
             values["checkpoint_operations"] != 0 ||
             values["checkpoint_failures"] != 0 ||
             values["checkpoint_bytes"] != 0 ||
             values["checkpoints_in_progress"] != 0)) {
            fail("inactive checkpoint window contains lifecycle activity for window " window)
        }
        if (window > maximum_window) {
            maximum_window = window
        }
    }

    END {
        if (failed) {
            exit 1
        }
        if (maximum_window == 0) {
            fail("no sustained windows found")
        }
        for (window = 1; window <= maximum_window; window++) {
            if (!activity_seen[window]) {
                fail("missing checkpoint record for window " window)
            }
            all_key = "all" SUBSEP window
            if (!metric_seen[all_key]) {
                fail("missing total metric for window " window)
            }
            overwrite_key = "overwrite" SUBSEP window
            delete_key = "delete" SUBSEP window
            overwrite_operations = metric_seen[overwrite_key] ? metric_operations[overwrite_key] : 0
            overwrite_p99 = metric_seen[overwrite_key] ? metric_p99[overwrite_key] : ""
            delete_operations = metric_seen[delete_key] ? metric_operations[delete_key] : 0
            delete_p99 = metric_seen[delete_key] ? metric_p99[delete_key] : ""
            printf "%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s\n", \
                workload, outer_run, window, \
                activity["checkpoint_activity" SUBSEP window], \
                activity["checkpoint_starts" SUBSEP window], \
                activity["checkpoint_operations" SUBSEP window], \
                activity["checkpoint_failures" SUBSEP window], \
                activity["checkpoint_bytes" SUBSEP window], \
                activity["checkpoints_in_progress" SUBSEP window], \
                overwrite_operations, overwrite_p99, delete_operations, delete_p99, \
                metric_operations[all_key], metric_rate[all_key]
        }
    }
' "$STDOUT_LOG" "$STDERR_LOG"

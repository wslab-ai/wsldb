# Unknown commit retry protection

Date: 2026-10-04

## Risk

After a metadata or WAL sync failure, `CommitOutcomeUnknown` tells the caller
that the transaction may already be durable. Retrying through the unresolved
handle could duplicate application effects if the engine accepted its stale
base snapshot.

## Invariant

Direct, individually buffered, and grouped buffered commits must reject a new
write through that handle with `WriteConflict` after an injected final-sync
failure. Only `refresh` or reopen may install the recovered transaction ID and
permit the application to decide whether a retry is needed.

## Verification

The existing focused sync-failure matrix now attempts the forbidden stale
write before refresh in all three ambiguous modes. Exact command and result are
recorded below:

```text
cargo fmt --all -- --check
PASS

cargo test --features fault-injection --test io_failures \
  commit_sync_failures_report_whether_the_outcome_is_ambiguous -- --exact
PASS: 1 passed; 0 failed; finished in 0.23s
```

The same matrix reopens each database, performs full verification, and confirms
the complete direct transaction, buffered transaction, or two-transaction
group after refresh.

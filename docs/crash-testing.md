# Crash testing

WSLDB has opt-in process-crash hooks at every final page write, after the data
sync, after the metadata write, and after the metadata sync. The integration
test launches a subprocess, terminates it with `abort` at each reachable hook,
reopens the file, performs a full uncached verification, and requires the
logical contents to equal either the complete old snapshot or the complete new
snapshot.

The write-buffer matrix additionally terminates after the complete WAL write
but before its sync, after the WAL sync, during checkpoint page writes, after
checkpoint data sync, after metadata write and sync, and after WAL clearing.
WAL clearing is checked both after its length change but before its sync and
after the sync completes. At the pre-sync append boundary, recovery may return
the old state or the complete new commit but never a partial transaction or
group. Once the WAL append sync hook is reached, recovery must return the
complete new logical commit at every later failure point.
Buffered active compaction is also terminated after staged WAL writes and sync,
after the inode-changing WAL rename and directory sync, immediately after the
main-file rename, after its directory sync, and during covered-WAL retirement;
recovery must preserve the same complete logical snapshot throughout.
Explicit buffer consolidation is terminated after each staged replacement
record write but before sync, after the replacement record sync, after its
atomic rename, and after the parent-directory sync. Recovery must accept either
equivalent WAL generation and return identical logical contents.
Backup and compact-output workers are terminated after every reachable copy or
page write, file sync, metadata write, truncate, hard-link, directory sync, and
temporary-name removal. Before the hard link the final target must be absent;
after it the target must reopen as the complete verified snapshot.

Initial creation is terminated after writing temporary metadata but before its
sync, after temporary metadata sync, after linking the temporary inode at the
final path, after the publication directory sync, after temporary-name removal,
and after the cleanup directory sync. A subsequent open must either create the
still-missing database or validate the fully published empty database; a
partial final metadata file is never accepted.

`tests/injection_contract.rs` scans every production Rust source file and
requires every crash and I/O-failure stage to use a static canonical name with
an explicit integration-test owner. It also verifies that the named test
configures the corresponding injection environment. Adding, renaming, or
removing a storage boundary therefore fails the all-target suite until the
coverage matrix is reviewed together with the implementation.

Run the suite locally with:

```sh
cargo test --all-targets --features crash-injection
```

The feature is disabled by default. A normal build does not inspect failpoint
environment variables and cannot activate these hooks.

The same feature provides a bounded test-only pausepoint at an instrumented
boundary. The compaction race test uses it to publish a competing commit after
the replacement is ready but before the compactor reacquires the writer lock.
The pause aborts after 30 seconds if its coordinating process disappears.

This tests sudden process termination and the ordering of the commit protocol.
It does not emulate lost device caches, torn sectors, filesystem bugs, or actual
power removal. Those remain separate production-readiness gates and must be
tested on every supported filesystem and hardware durability configuration.

## I/O failure injection

The separate `fault-injection` feature can impose one short positioned write or
one storage-full error at initial creation, commit, checkpoint, backup,
compaction, snapshot publication, atomic replacement, or directory-sync
boundaries. Tests run each failure in an isolated subprocess and then fully
verify reopened state:

```sh
cargo test --features fault-injection --test io_failures
cargo test --features fault-injection --test maintenance_failures
cargo test --features fault-injection --test initialization
```

A short initial metadata, data-page, WAL-record, backup, or compaction write
must return `Error::Io` and preserve the previous committed snapshot or leave
an unpublished destination absent. WAL append also removes its incomplete
suffix so the same live handle can commit again. Once metadata publication
begins, or once a complete WAL record reaches its final sync, the operation
returns `Error::CommitOutcomeUnknown`. The application must refresh or reopen
and inspect the durable transaction ID before retrying. Direct, buffered, and
group-commit sync-failure tests also require a write through the unresolved
handle to fail with `WriteConflict` before that refresh.

The first buffered open also injects failure before synchronizing the empty
WAL sidecar's parent directory. The failure may leave an empty file, but a
second open must repeat the same barrier instead of treating the path as
durably published. Only a successful directory sync permits the first WAL
append, so no acknowledged transaction depends on an unpersisted sidecar name.

Automatic checkpoint failure keeps the WAL commit durable, increments the
failure counter, and reconstructs a consistent live store. Backup, compaction,
and consolidation errors after a link or rename return
`Error::PublicationOutcomeUnknown`; callers inspect the target or refresh the
active handle before retrying. Checkpoint WAL clearing returns
`PublicationOutcomeUnknown` if clearing fails after main-file publication.
Buffered compaction instead copies and syncs the complete WAL, changes its
inode, and only then replaces the main file. Fault injection covers every WAL
copy, sync, rename and directory-sync boundary as well as main replacement.
The affected live handle rejects subsequent writes until `refresh` installs
the published generation; a durable write after refresh is included in the
same focused tests.

This feature is disabled by default. It does not replace real filesystem-full,
device-error, or power-loss testing.

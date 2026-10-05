# WSLDB engineering rules

This file is the repository-local source of truth for coding agents and human
contributors.

## Working agreement

1. Inspect this file, the Git state, and the relevant implementation before
   changing code.
2. Keep unrelated user changes intact. Do not use destructive Git commands.
3. Work on a feature branch. Never commit or push directly to a protected base
   branch. The repository bootstrap commit is the sole initialization exception.
4. Do not create remote projects, branches, pull requests, comments, or tracker
   updates unless the user explicitly asks for that external write.
5. Never put credentials, internal addresses, personal data, or raw production
   payloads in source, fixtures, logs, prompts, documentation, or commits.
6. Do not add AI-tool attribution to commits or pull requests.

## Architecture

- Write clear, idiomatic, production-oriented Rust and follow SOLID
  pragmatically.
- Dependencies point inward: `infrastructure -> application -> domain`.
- Domain code must not import storage libraries, operating-system adapters, or
  vendor SDKs.
- Application use cases depend on small explicit ports. Infrastructure adapters
  implement those ports and translate filesystem data at the boundary.
- Keep one authoritative implementation for each file-format, transaction,
  checksum, ordering, and recovery invariant.
- Prefer composition, immutable snapshots, explicit types, and dependency
  injection over global state or service locators.
- Add an abstraction only when it represents an actual I/O boundary, invariant,
  or test seam.

## Size and safety

- No hand-written source file may exceed 700 physical lines. Aim for fewer than
  400 and split by responsibility before reaching the hard limit.
- Keep functions focused and use domain names outside infrastructure adapters.
- Required configuration must fail validation at startup.
- Avoid runtime panics, unchecked indexing, and magic values.
- `unsafe` is forbidden until a measured mmap or platform need is documented,
  isolated, reviewed, and covered by correctness tests.

## Persistence invariants

- Calculate and validate a complete commit before publishing its metadata.
- Sync data before switching the active metadata slot.
- Readers observe one immutable committed snapshot.
- Only one writer may publish within a process.
- Corrupt or incomplete newest snapshots must fall back to the newest valid
  older metadata slot; otherwise opening fails closed.
- Preserve deterministic byte ordering and reproducible file encoding.

## Verification

- Domain rules have fast tests with no filesystem.
- Adapters have recovery and corruption tests using isolated temporary files.
- Add focused tests for new reusable behavior and causal invariants.
- Run `cargo fmt --all -- --check`, `cargo test --all-targets`,
  `cargo clippy --all-targets -- -D warnings`, `cargo check --all-targets`, and
  `git diff --check` before considering a change complete.
- Optimize only after measurement. Use preregistered workloads and independent
  baselines before making performance claims.

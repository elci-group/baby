# Contributing to baby

Thank you for helping improve baby. This document describes the workflow and
quality expectations for contributions.

## Development setup

You need a current stable Rust toolchain with `rustfmt` and `clippy`:

```bash
rustup component add rustfmt clippy
```

## Quality gate

Every change must pass the following before it can be merged:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
cargo audit
brandi lint --fail-under 80
```

You can run everything in one go with:

```bash
./scripts/ci.sh
```

## Project structure

- `src/lib.rs` — shared library code: install logic, XDG helpers, PID management.
- `src/error.rs` — structured `BabyError` type used throughout the crate.
- `src/logger.rs` — tiny `log` implementation that replaces `env_logger`.
- `src/config.rs` — `.birth.toml` parsing and project mapping.
- `src/bin/baby.rs` — `baby` CLI.
- `src/bin/birthctl.rs` — `birthctl` CLI.
- `src/bin/birthd.rs` — `birthd` daemon.
- `tests/cli.rs` — integration tests for the three binaries.
- `man/` — generated man pages.

## Tests

Unit tests live in the same file as the code they exercise, under `#[cfg(test)]`.
Integration tests live in `tests/`.

When adding a feature, add tests that cover both the happy path and the
failure modes. Use `tempfile` for filesystem isolation; never write to real
system paths in tests.

## Commit workflow

This repository is monitored by kaptaind, which clusters file changes and
creates deterministic commits. Because GitHub branch protection blocks the
daemon's pushes, commits accumulate locally and are pushed by maintainers.

If you need to commit manually, prefer conventional-commit style messages and
keep the change focused.

## Man pages

Regenerate man pages after changing CLI arguments:

```bash
cargo run --bin baby -- --generate-man man/baby.1
cargo run --bin birthctl -- --generate-man man/birthctl.1
cargo run --bin birthd -- --generate-man man/birthd.1
```

## Brand coherence

Outward-facing strings, README prose, and docs are linted by Brandi. Run:

```bash
brandi lint --fail-under 80
```

before opening a pull request.

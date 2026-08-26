# baby-dev

Skill for working on the `baby` project: Build And Bin Yield.

## Project overview

`baby` is a Rust toolchain with three binaries:

- `baby` — build a Rust project in release mode and install the binary. Async entrypoint (`#[tokio::main]`); also hosts the `boom` sub-command (see below).
- `birthd` — filesystem watcher daemon that rebuilds on change.
- `birthctl` — control utility for `birthd`.

Configuration lives in `.birth.toml` files discovered from the current directory, `~/.config/birth.d`, and `/etc/birth.d`.

### `boom` sub-command

`baby boom` discovers, tracks, and updates multiple tool installations in parallel, driven by `.boom.toml`. Implementation lives in `src/boom/` (`config`, `detection`, `discovery`, `execution`, `interactive`, `types`); tool discovery/detection runs concurrently via `tokio::task::spawn_blocking`, and `execution` renders results with `tabled`. Full reference: `docs/BOOM_GUIDE.md`, `docs/BOOM_ARCHITECTURE.md`, `docs/BOOM_BENCHMARKS.md`.

## Quality gate

Every change must pass (see `scripts/ci.sh`, mirrored in `.github/workflows/ci.yml`):

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo doc --no-deps        # RUSTDOCFLAGS="-D warnings"
cargo audit
brandi lint --fail-under 80
```

Man pages live in `man/` (`baby.1`, `birthd.1`, `birthctl.1`) — keep them in sync with CLI changes.

Kaptaind monitors the repo and auto-commits qualifying clusters. Push is disabled because GitHub branch protection blocks the daemon; maintainers push manually.

## Conventions

- Use the `BabyError` type for all library errors; keep binary `main()` functions thin.
- Prefer `std::fs` operations over shelling out to `mkdir`, `cp`, or `install`.
- XDG paths: runtime files under `$XDG_RUNTIME_DIR`, state/logs under `$XDG_STATE_HOME`, configs under `$XDG_CONFIG_HOME`.
- PID-file IPC is signal-based (`SIGHUP`, `SIGTERM`).
- Tests should use temp directories; never write to real system paths in tests. Integration tests live in `tests/` (`tests/cli.rs`).

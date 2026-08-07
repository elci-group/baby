# baby-dev

Skill for working on the `baby` project: Build And Bin Yield.

## Project overview

`baby` is a Rust toolchain with three binaries:

- `baby` — build a Rust project in release mode and install the binary.
- `birthd` — filesystem watcher daemon that rebuilds on change.
- `birthctl` — control utility for `birthd`.

Configuration lives in `.birth.toml` files discovered from the current directory, `~/.config/birth.d`, and `/etc/birth.d`.

## Quality gate

Every change must pass:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo doc --no-deps
brandi lint --fail-under 80
```

Kaptaind monitors the repo and auto-commits qualifying clusters. Push is disabled because GitHub branch protection blocks the daemon; maintainers push manually.

## Conventions

- Use the `BabyError` type for all library errors; keep binary `main()` functions thin.
- Prefer `std::fs` operations over shelling out to `mkdir`, `cp`, or `install`.
- XDG paths: runtime files under `$XDG_RUNTIME_DIR`, state/logs under `$XDG_STATE_HOME`, configs under `$XDG_CONFIG_HOME`.
- PID-file IPC is signal-based (`SIGHUP`, `SIGTERM`).
- Tests should use temp directories; never write to real system paths in tests.

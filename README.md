# baby

**baby — Build And Bin Yield** — build a Rust project in release mode and install
the resulting binary with one command.

> Make building and installing Rust binaries effortless, observable, and repeatable.

## Install

From source with a working Rust toolchain:

```bash
cargo install --path .
```

This installs three binaries:

- `baby` — one-shot build and install.
- `birthd` — filesystem watcher daemon that rebuilds on change.
- `birthctl` — control utility for `birthd`.

## Quick start

Build and install the current project to `/usr/local/bin`:

```bash
baby
```

Install to your user bin directory instead:

```bash
baby --user
```

Build, install, and run the binary:

```bash
baby --run -- --help
```

Set up a watched project:

```bash
birthctl watch --project myapp --path src --path Cargo.toml --install ~/.local/bin
birthd &
```

Now every change to `src/` or `Cargo.toml` triggers a rebuild and install.

## Configuration

`birthd` discovers `.birth.toml` files in three places:

1. The current working directory.
2. `$HOME/.config/birth.d/`
3. `/etc/birth.d/`

A `.birth.toml` looks like this:

```toml
project = "myapp"
watch = ["src", "Cargo.toml"]
build = "cargo build --release"
install = "/usr/local/bin"
restart = "myapp.service"  # optional systemd service to restart
debounce_ms = 500
strip = false
backup = false
sudo = false
user = false
```

- `project` — project/binary name.
- `watch` — paths to watch for changes.
- `build` — build command (default: `cargo build --release`).
- `install` — installation directory.
- `restart` — optional systemd service to restart after a successful build.
- `debounce_ms` — quiet period before triggering a rebuild (default: 500).
- `strip` — strip debug symbols before installing.
- `backup` — backup the existing binary before overwriting.
- `sudo` — use `sudo` for privileged operations.
- `user` — install to `~/.local/bin` (ignored if `install` is absolute).

## Commands

### `baby`

```text
baby [OPTIONS] [RUN_ARGS]
```

| Option | Description |
|--------|-------------|
| `--run` | Build, install, then execute the binary. |
| `--strip` | Strip debug symbols before installing. |
| `--backup` | Backup existing binary before overwriting. |
| `--service` | Restart matching systemd service after install. |
| `--sudo` | Use `sudo` for privileged install operations. |
| `--user` | Install to `~/.local/bin`. |
| `--dry-run` | Show what would happen without mutating the filesystem. |
| `--target-dir <DIR>` | Override the Cargo target directory. |
| `--install-dir <DIR>` | Override the installation directory. |

### `birthctl`

```text
birthctl <COMMAND>
```

| Command | Description |
|---------|-------------|
| `status` | Show daemon status and watched projects. |
| `reload` | Tell `birthd` to reload its configs. |
| `stop` | Gracefully stop the daemon. |
| `logs` | Show recent daemon logs. |
| `watch` | Create a local `.birth.toml`. |

### `birthd`

```text
birthd
```

Run the build daemon. It stores its PID in `$XDG_RUNTIME_DIR/birthd.pid`
(or `/tmp/birthd.pid`) and writes logs to `$XDG_STATE_HOME/birthd.log`
(or `~/.local/state/birthd.log`).

Signals:

- `SIGHUP` — reload configuration.
- `SIGTERM` / `SIGINT` — graceful shutdown.

## XDG paths

| Purpose | Path |
|---------|------|
| PID file | `$XDG_RUNTIME_DIR/birthd.pid` |
| Log file | `$XDG_STATE_HOME/birthd.log` |
| User configs | `$XDG_CONFIG_HOME/birth.d/` |
| Fallback user install | `$HOME/.local/bin` |

If the XDG variables are unset, `birthd` falls back to common defaults.

## Development

The quality gate for every change:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
cargo audit
brandi lint --fail-under 80
```

This repository is monitored by [kaptaind](https://github.com/elci-group/kaptaind).
Kaptaind auto-commits qualifying changes, but GitHub branch protection on `main`
prevents auto-push, so maintainers push manually.

## License

MIT OR Apache-2.0
